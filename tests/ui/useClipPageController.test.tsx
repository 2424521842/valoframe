import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type {
  Clip,
  ClipListQuery,
  ClipPage,
  ClipSummary,
} from "../../src/types";

const mocks = vi.hoisted(() => ({
  listClipPage: vi.fn(),
}));

vi.mock("../../src/api/backend", () => ({
  commandErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  listClipPage: mocks.listClipPage,
}));

import {
  mergeClipSummaryPages,
  useClipPageController,
} from "../../src/hooks/useClipPageController";

const DEFAULT_QUERY: ClipListQuery = {
  offset: 0,
  limit: 50,
  sortBy: "modified-desc",
};
const clipA = summary("a");
const clipB = summary("b");
const clipC = summary("c");

describe("useClipPageController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listClipPage.mockResolvedValue(page([clipA, clipB], 2));
  });

  it("loads the bounded first page and exposes backend paging state", async () => {
    const onActivityMessage = vi.fn();
    const { result } = renderController({ onActivityMessage });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(mocks.listClipPage).toHaveBeenCalledTimes(1);
    expect(mocks.listClipPage).toHaveBeenCalledWith(DEFAULT_QUERY);
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b"]);
    expect(result.current.totalCount).toBe(2);
    expect(result.current.hasMore).toBe(false);
    expect(result.current.nextOffset).toBeNull();
    expect(result.current.error).toBeNull();
    expect(onActivityMessage).toHaveBeenCalledWith("已加载全部 2 个素材");
  });

  it("loads nextOffset once, replaces duplicates, and preserves backend order", async () => {
    const more = deferred<ClipPage>();
    mocks.listClipPage
      .mockResolvedValueOnce(page([clipA, clipB], 3, 2))
      .mockReturnValueOnce(more.promise);
    const { result } = renderController();

    await waitFor(() => expect(result.current.hasMore).toBe(true));
    let first!: Promise<boolean>;
    let duplicate!: Promise<boolean>;
    act(() => {
      first = result.current.loadMore();
      duplicate = result.current.loadMore();
    });

    expect(result.current.isLoadingMore).toBe(true);
    await expect(duplicate).resolves.toBe(false);
    expect(mocks.listClipPage).toHaveBeenCalledTimes(2);
    expect(mocks.listClipPage).toHaveBeenLastCalledWith({
      ...DEFAULT_QUERY,
      offset: 2,
    });

    const refreshedB = { ...clipB, isFavorite: true };
    await act(async () => {
      more.resolve(page([refreshedB, clipC], 3));
      await first;
    });

    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b", "c"]);
    expect(result.current.getItem("b")?.isFavorite).toBe(true);
    expect(result.current.hasMore).toBe(false);
    expect(result.current.isLoadingMore).toBe(false);
  });

  it("loads every remaining page for an explicit select-all request", async () => {
    mocks.listClipPage
      .mockResolvedValueOnce(page([clipA], 3, 1))
      .mockResolvedValueOnce(page([clipB], 3, 2))
      .mockResolvedValueOnce(page([clipC], 3));
    const { result } = renderController();

    await waitFor(() => expect(result.current.hasMore).toBe(true));
    let allClips: ClipSummary[] | null = null;
    await act(async () => {
      allClips = await result.current.loadAll();
    });

    expect(mocks.listClipPage.mock.calls.map(([query]) => query.offset)).toEqual([0, 1, 2]);
    expect(mocks.listClipPage.mock.calls.map(([query]) => query.limit)).toEqual([50, 200, 200]);
    expect(allClips?.map((clip) => clip.id)).toEqual(["a", "b", "c"]);
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b", "c"]);
    expect(result.current.hasMore).toBe(false);
  });

  it("ignores a delayed response from the previous query generation", async () => {
    const oldPage = deferred<ClipPage>();
    const newPage = deferred<ClipPage>();
    mocks.listClipPage
      .mockReturnValueOnce(oldPage.promise)
      .mockReturnValueOnce(newPage.promise);
    const controller = renderController();

    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));
    controller.rerender({
      query: { ...DEFAULT_QUERY, query: "new" },
      queryKey: "query:new",
      onActivityMessage: controller.activity,
    });
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));

    await act(async () => {
      newPage.resolve(page([clipC], 1));
      await newPage.promise;
    });
    expect(controller.result.current.items.map((clip) => clip.id)).toEqual(["c"]);
    expect(controller.result.current.generation).toBe(2);

    await act(async () => {
      oldPage.resolve(page([clipA, clipB], 2));
      await oldPage.promise;
    });
    expect(controller.result.current.items.map((clip) => clip.id)).toEqual(["c"]);
  });

  it("reports an initial failure and retries from offset zero", async () => {
    mocks.listClipPage
      .mockRejectedValueOnce(new Error("database unavailable"))
      .mockResolvedValueOnce(page([clipA], 1));
    const { result } = renderController();

    await waitFor(() => expect(result.current.error).toBe("database unavailable"));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.items).toEqual([]);

    await act(async () => {
      await result.current.retry();
    });

    expect(mocks.listClipPage).toHaveBeenCalledTimes(2);
    expect(mocks.listClipPage).toHaveBeenLastCalledWith(DEFAULT_QUERY);
    expect(result.current.error).toBeNull();
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a"]);
  });

  it("keeps the loaded page after a load-more failure and retries that offset", async () => {
    mocks.listClipPage
      .mockResolvedValueOnce(page([clipA], 2, 1))
      .mockRejectedValueOnce(new Error("next page failed"))
      .mockResolvedValueOnce(page([clipB], 2));
    const { result } = renderController();

    await waitFor(() => expect(result.current.hasMore).toBe(true));
    await act(async () => {
      await result.current.loadMore();
    });
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a"]);
    expect(result.current.loadMoreError).toBe("next page failed");

    await act(async () => {
      await result.current.retryLoadMore();
    });
    expect(mocks.listClipPage).toHaveBeenLastCalledWith({
      ...DEFAULT_QUERY,
      offset: 1,
    });
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b"]);
    expect(result.current.loadMoreError).toBeNull();
  });

  it("updates, merges, finds, and removes only the in-memory summaries", async () => {
    const { result } = renderController();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    act(() => {
      result.current.updateItems((current) => current.map((clip) =>
        clip.id === "a" ? { ...clip, isFavorite: true } : clip,
      ));
      result.current.mergeSummaries([{ ...clipB, tags: ["updated"] }, clipC]);
    });
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b", "c"]);
    expect(result.current.getItem("a")?.isFavorite).toBe(true);
    expect(result.current.getItem("b")?.tags).toEqual(["updated"]);

    act(() => result.current.removeSummaries(["a", "missing"]));
    expect(result.current.items.map((clip) => clip.id)).toEqual(["b", "c"]);
    expect(result.current.getItem("a")).toBeUndefined();
  });

  it("supports a silent explicit reload and increments its generation", async () => {
    const onActivityMessage = vi.fn();
    const { result } = renderController({ onActivityMessage });
    await waitFor(() => expect(result.current.items).toHaveLength(2));
    onActivityMessage.mockClear();
    mocks.listClipPage.mockResolvedValueOnce(page([clipC], 1));

    await act(async () => {
      await result.current.reload({ preserveActivity: true });
    });

    expect(result.current.generation).toBe(2);
    expect(result.current.items.map((clip) => clip.id)).toEqual(["c"]);
    expect(onActivityMessage).not.toHaveBeenCalled();
  });

  it("completes its initial request under StrictMode effect replay", async () => {
    const { result } = renderController({ reactStrictMode: true });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.items.map((clip) => clip.id)).toEqual(["a", "b"]);
    expect(mocks.listClipPage).toHaveBeenCalledTimes(1);
  });
});

describe("mergeClipSummaryPages", () => {
  it("updates existing ids and appends unseen ids without reordering", () => {
    const updatedA = { ...clipA, isFavorite: true };
    expect(mergeClipSummaryPages([clipA, clipB], [updatedA, clipC]))
      .toEqual([updatedA, clipB, clipC]);
  });
});

type ControllerProps = {
  query: ClipListQuery;
  queryKey: string;
  onActivityMessage: ReturnType<typeof vi.fn>;
};

function renderController(options: {
  onActivityMessage?: ReturnType<typeof vi.fn>;
  reactStrictMode?: boolean;
} = {}) {
  const activity = options.onActivityMessage ?? vi.fn();
  const controller = renderHook(
    ({ query, queryKey, onActivityMessage }: ControllerProps) =>
      useClipPageController({ query, queryKey, onActivityMessage }),
    {
      initialProps: {
        query: DEFAULT_QUERY,
        queryKey: "query:default",
        onActivityMessage: activity,
      },
      reactStrictMode: options.reactStrictMode,
    },
  );
  return { ...controller, activity };
}

function summary(id: string): ClipSummary {
  const value: Clip = {
    ...mockClips[0],
    id,
    fileName: `clip-${id}.mp4`,
    filePath: `D:\\Highlights\\clip-${id}.mp4`,
    clipGroupId: `group-${id}`,
    clipGroupName: `group-${id}`,
    matchId: `match-${id}`,
    isFavorite: false,
    tags: [],
    clipEvents: mockClips[0].clipEvents?.map((event) => ({ ...event })),
  };
  const partial = value as Partial<Clip>;
  delete partial.note;
  delete partial.extractedText;
  delete partial.clipEvents;
  delete partial.eventCount;
  delete partial.roundLabel;
  delete partial.weaponName;
  return value;
}

function page(
  items: ClipSummary[],
  totalCount: number,
  nextOffset: number | null = null,
): ClipPage {
  return {
    items,
    offset: nextOffset === null ? 0 : Math.max(0, nextOffset - items.length),
    limit: 50,
    totalCount,
    hasMore: nextOffset !== null,
    nextOffset,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
