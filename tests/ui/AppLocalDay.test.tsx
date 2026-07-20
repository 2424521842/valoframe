import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipListQuery, ClipPage } from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  getLibraryFacets: vi.fn(),
  listClipPage: vi.fn(),
  listClips: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  localDay: new Date(2026, 6, 20),
}));

vi.mock("../../src/hooks/useLocalDay", () => ({
  useLocalDay: () => mocks.localDay,
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    getLibraryFacets: mocks.getLibraryFacets,
    listClipPage: mocks.listClipPage,
    listClips: mocks.listClips,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    listenToScanProgress: vi.fn(async () => () => undefined),
  };
});

import App from "../../src/App";

describe("App local-day query boundaries", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.localDay = new Date(2026, 6, 20);
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets());
    mocks.listClipPage.mockResolvedValue(emptyPage());
    mocks.listSources.mockResolvedValue([]);
    mocks.listTags.mockResolvedValue([]);
  });

  it("updates active date boundaries at midnight without reloading an unfiltered query", async () => {
    const user = userEvent.setup();
    const view = render(<App />);
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));

    await act(async () => {
      mocks.localDay = new Date(2026, 6, 21);
      view.rerender(<App />);
      await Promise.resolve();
    });
    expect(mocks.listClipPage).toHaveBeenCalledTimes(1);

    await user.click(await screen.findByRole("combobox", { name: "日期" }));
    await user.click(await screen.findByRole("option", { name: "今天" }));
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    expect(lastListQuery()).toEqual(expect.objectContaining(
      localDayBoundaries(new Date(2026, 6, 21)),
    ));

    await act(async () => {
      mocks.localDay = new Date(2026, 6, 22);
      view.rerender(<App />);
      await Promise.resolve();
    });
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(3));
    expect(lastListQuery()).toEqual(expect.objectContaining(
      localDayBoundaries(new Date(2026, 6, 22)),
    ));
    expect(mocks.listClips).not.toHaveBeenCalled();
  });
});

function emptyPage(): ClipPage {
  return {
    items: [],
    offset: 0,
    limit: 50,
    totalCount: 0,
    hasMore: false,
    nextOffset: null,
  };
}

function lastListQuery(): ClipListQuery {
  return mocks.listClipPage.mock.calls.at(-1)?.[0] as ClipListQuery;
}

function localDayBoundaries(day: Date): Pick<ClipListQuery, "modifiedFrom" | "modifiedTo"> {
  return {
    modifiedFrom: Math.floor(new Date(
      day.getFullYear(),
      day.getMonth(),
      day.getDate(),
    ).getTime() / 1_000),
    modifiedTo: Math.floor(new Date(
      day.getFullYear(),
      day.getMonth(),
      day.getDate(),
      23,
      59,
      59,
    ).getTime() / 1_000),
  };
}
