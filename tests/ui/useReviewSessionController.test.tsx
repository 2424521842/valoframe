import { StrictMode, type PropsWithChildren } from "react";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import { useReviewSessionController } from "../../src/hooks/useReviewSessionController";
import type { ClipListQuery, ClipPage, ClipSummary, ReviewSessionFilters } from "../../src/types";

const mocks = vi.hoisted(() => ({
  listClipPage: vi.fn(),
  setClipReviewDecision: vi.fn(),
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    listClipPage: mocks.listClipPage,
    setClipReviewDecision: mocks.setClipReviewDecision,
  };
});

const clips = Array.from({ length: 5 }, (_, index) => ({
  ...mockClips[index % mockClips.length],
  id: String(index + 1),
  createdAt: `2026-08-0${index + 1}T12:00:00.000Z`,
  modifiedAt: `2026-08-0${index + 1}T12:00:00.000Z`,
  isFavorite: false,
  tags: ["existing-tag"],
  fileStatus: "available",
  reviewDecision: "unreviewed" as const,
  reviewedAt: null,
})) as ClipSummary[];

const filters: ReviewSessionFilters = {
  query: {
    accountId: "winter",
    agentName: "尚勃勒",
    mapName: "隐世修所",
    gameMode: "竞技模式",
    modifiedFrom: 1_784_649_600,
    modifiedTo: 1_787_241_599,
    sortBy: "modified-desc",
  },
  labels: ["账号：Winter", "英雄：尚勃勒", "地图：隐世修所", "模式：竞技模式"],
  sort: "library",
  candidateScope: "all",
};

function strictWrapper({ children }: PropsWithChildren) {
  return <StrictMode>{children}</StrictMode>;
}

describe("useReviewSessionController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listClipPage.mockResolvedValue(page(clips));
  });

  it("inherits the complete library query and records D, D, A, S, D independently", async () => {
    const { result } = renderHook(() => useReviewSessionController(), { wrapper: strictWrapper });

    await act(async () => {
      await result.current.prepare(filters);
    });
    expect(mocks.listClipPage).toHaveBeenCalledWith({
      ...filters.query,
      offset: 0,
      limit: 200,
    });
    expect(mocks.listClipPage.mock.calls[0][0]).not.toHaveProperty("reviewDecision");

    await act(async () => {
      expect(await result.current.startNew(filters)).toBe(true);
    });
    for (const decision of ["selected", "selected", "skipped", "pending", "selected"] as const) {
      await act(async () => {
        expect(await result.current.decide(decision)).toBe(true);
      });
    }

    expect(result.current.phase).toBe("completed");
    expect(result.current.counts).toEqual({
      total: 5,
      reviewed: 5,
      selected: 3,
      pending: 1,
      skipped: 1,
      remaining: 0,
    });
    expect(result.current.candidateClips.every((clip) => !clip.isFavorite)).toBe(true);
    expect(result.current.candidateClips.every((clip) => clip.tags[0] === "existing-tag")).toBe(true);
    expect(mocks.setClipReviewDecision).not.toHaveBeenCalled();

    act(() => {
      expect(result.current.undo()).toBe(true);
    });
    expect(result.current.phase).toBe("reviewing");
    expect(result.current.currentClip?.id).toBe("5");
    expect(result.current.counts).toMatchObject({
      reviewed: 4,
      selected: 2,
      pending: 1,
      skipped: 1,
      remaining: 1,
    });
  });

  it("loads paginated candidate metadata while leaving media loading to the player", async () => {
    mocks.listClipPage.mockImplementation(async (query: ClipListQuery) => (
      query.offset === 2
        ? page([clips[2]])
        : page(clips.slice(0, 2), { hasMore: true, nextOffset: 2, totalCount: 3 })
    ));
    const { result } = renderHook(() => useReviewSessionController());

    await act(async () => {
      await result.current.prepare(filters);
    });

    expect(mocks.listClipPage).toHaveBeenNthCalledWith(1, {
      ...filters.query,
      offset: 0,
      limit: 200,
    });
    expect(mocks.listClipPage).toHaveBeenNthCalledWith(2, {
      ...filters.query,
      offset: 2,
      limit: 200,
    });
    expect(result.current.draftCandidates.map((clip) => clip.id)).toEqual(["1", "2", "3"]);
  });

  it("persists an unfinished session and resumes from its next unreviewed clip", async () => {
    const first = renderHook(() => useReviewSessionController());
    await act(async () => {
      await first.result.current.prepare(filters);
      await first.result.current.startNew(filters);
      await first.result.current.decide("selected");
    });
    first.unmount();

    const second = renderHook(() => useReviewSessionController());
    await act(async () => {
      await second.result.current.prepare(filters);
    });
    expect(second.result.current.resumableSession).toMatchObject({
      status: "active",
      currentIndex: 1,
    });
    await act(async () => {
      expect(await second.result.current.resume(second.result.current.resumableSession!)).toBe(true);
    });
    expect(second.result.current.currentClip?.id).toBe("2");
    expect(second.result.current.counts).toMatchObject({ selected: 1, reviewed: 1, remaining: 4 });
  });

  it("prefers saved progress over a newer empty session with the same conditions", async () => {
    const first = renderHook(() => useReviewSessionController());
    await act(async () => {
      await first.result.current.prepare(filters);
      await first.result.current.startNew(filters);
      await first.result.current.decide("selected");
    });
    const progressedSessionId = first.result.current.session?.id;
    first.unmount();

    const accidental = renderHook(() => useReviewSessionController());
    await act(async () => {
      await accidental.result.current.prepare(filters);
      await accidental.result.current.startNew(filters);
    });
    expect(accidental.result.current.counts).toMatchObject({ reviewed: 0, remaining: 5 });
    accidental.unmount();

    const third = renderHook(() => useReviewSessionController());
    await act(async () => {
      await third.result.current.prepare(filters);
    });
    expect(third.result.current.resumableSession).toMatchObject({
      id: progressedSessionId,
      currentIndex: 1,
    });
    await act(async () => {
      expect(await third.result.current.resume(third.result.current.resumableSession!)).toBe(true);
    });
    expect(third.result.current.currentClip?.id).toBe("2");
    expect(third.result.current.counts).toMatchObject({ selected: 1, reviewed: 1, remaining: 4 });
  });

  it("locks repeated input while a visual exit is still pending", async () => {
    const exit = deferred<void>();
    const { result } = renderHook(() => useReviewSessionController());
    await act(async () => {
      await result.current.prepare(filters);
      await result.current.startNew(filters);
    });

    let firstDecision!: Promise<boolean>;
    let duplicateDecision!: Promise<boolean>;
    act(() => {
      firstDecision = result.current.decide("selected", () => exit.promise);
      duplicateDecision = result.current.decide("selected", () => exit.promise);
    });
    await expect(duplicateDecision).resolves.toBe(false);
    expect(result.current.currentClip?.id).toBe("1");

    exit.resolve();
    await act(async () => {
      expect(await firstDecision).toBe(true);
    });
    expect(result.current.currentClip?.id).toBe("2");
    expect(result.current.counts).toMatchObject({ selected: 1, reviewed: 1 });
  });
});

function page(
  items: ClipSummary[],
  overrides: Partial<ClipPage> = {},
): ClipPage {
  return {
    items,
    offset: 0,
    limit: 200,
    totalCount: items.length,
    hasMore: false,
    nextOffset: null,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
