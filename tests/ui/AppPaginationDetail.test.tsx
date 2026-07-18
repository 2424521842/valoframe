import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type { Clip, ClipListQuery, ClipPage, ClipSummary, ThumbnailProgress } from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  ensureClipThumbnails: vi.fn(),
  getClipDetail: vi.fn(),
  getClipMedia: vi.fn(),
  getLibraryFacets: vi.fn(),
  listClipPage: vi.fn(),
  listClips: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  setClipFavorite: vi.fn(),
  updateClipNote: vi.fn(),
  listenToThumbnailProgress: vi.fn(),
  thumbnailListener: null as ((progress: ThumbnailProgress) => void) | null,
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    coverUrlForClipId: (clipId: string, revision?: string | null) =>
      `clip-media://cover/${clipId}${revision ? `?v=${encodeURIComponent(revision)}` : ""}`,
    ensureClipThumbnails: mocks.ensureClipThumbnails,
    getClipDetail: mocks.getClipDetail,
    getClipMedia: mocks.getClipMedia,
    getLibraryFacets: mocks.getLibraryFacets,
    listClipPage: mocks.listClipPage,
    listClips: mocks.listClips,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    setClipFavorite: mocks.setClipFavorite,
    updateClipNote: mocks.updateClipNote,
    listenToThumbnailProgress: mocks.listenToThumbnailProgress,
    listenToScanProgress: vi.fn(async () => () => undefined),
  };
});

import App from "../../src/App";

const clipA = createClip("101", { officialVideoName: "分页高光 A" });
const clipB = createClip("102", { officialVideoName: "分页高光 B" });
const clipC = createClip("103", { officialVideoName: "分页高光 C" });

describe("production paginated list and on-demand detail flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.thumbnailListener = null;
    mocks.ensureClipThumbnails.mockImplementation(async (clipIds: readonly string[]) => ({
      requested: clipIds.length,
      queued: clipIds.length,
      alreadyQueued: 0,
      skipped: 0,
    }));
    mocks.listenToThumbnailProgress.mockImplementation(async (
      listener: (progress: ThumbnailProgress) => void,
    ) => {
      mocks.thumbnailListener = listener;
      return () => {
        if (mocks.thumbnailListener === listener) mocks.thumbnailListener = null;
      };
    });
    mocks.listSources.mockResolvedValue([]);
    mocks.listTags.mockResolvedValue([]);
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({ activeCount: 2, totalCount: 2 }));
    mocks.listClipPage.mockResolvedValue(page([clipA, clipB], 2));
    mocks.getClipDetail.mockImplementation(async (clipId: string) =>
      cloneClip([clipA, clipB, clipC].find((clip) => clip.id === clipId) ?? clipA),
    );
    mocks.getClipMedia.mockImplementation(async (clipId: string) => ({
      clipId,
      playable: false,
      mediaUrl: null,
      message: "测试媒体不可播放",
    }));
    mocks.setClipFavorite.mockImplementation(async (clipId: string, isFavorite: boolean) => ({
      ...cloneClip([clipA, clipB, clipC].find((clip) => clip.id === clipId) ?? clipA),
      isFavorite,
    }));
    mocks.updateClipNote.mockImplementation(async (clipId: string, note: string) => ({
      ...cloneClip([clipA, clipB, clipC].find((clip) => clip.id === clipId) ?? clipA),
      note,
    }));
  });

  it("initializes with one bounded list_clip_page request and never calls the legacy list", async () => {
    const fifty = Array.from({ length: 50 }, (_, index) =>
      createClip(String(1000 + index), { matchId: `match-${index}` }),
    );
    mocks.listClipPage.mockResolvedValue(page(fifty, 10_000, 50));

    render(<App />);

    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));
    expect(mocks.listClipPage).toHaveBeenCalledWith(expect.objectContaining({
      offset: 0,
      limit: 50,
      sortBy: "modified-desc",
    }));
    expect(mocks.listClips).not.toHaveBeenCalled();
    expect(await screen.findByText("50 / 10000 条片段")).toBeVisible();
    expect(document.querySelectorAll("[data-clip-id]").length).toBeLessThan(20);
  });

  it("retries the same thumbnail revision after eviction without list or media reloads", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.ensureClipThumbnails).toHaveBeenCalledWith(["101", "102"]));
    await waitFor(() => expect(mocks.thumbnailListener).not.toBeNull());

    await user.click(await screen.findByRole("button", { name: /预览分页高光 A/ }));
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledTimes(1));
    const listRequests = mocks.listClipPage.mock.calls.length;
    const facetRequests = mocks.getLibraryFacets.mock.calls.length;
    const mediaRequests = mocks.getClipMedia.mock.calls.length;
    const ensureRequests = mocks.ensureClipThumbnails.mock.calls.length;

    act(() => {
      mocks.thumbnailListener?.({
        clipId: "101",
        status: "ready",
        revision: "revision-1",
        errorCode: null,
      });
    });

    const heroImage = await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(".cinematic-artwork--hero img");
      expect(image).toBeInTheDocument();
      return image!;
    });
    expect(heroImage.src).toContain("?v=revision-1");
    expect(mocks.listClipPage).toHaveBeenCalledTimes(listRequests);
    expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(facetRequests);
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(mediaRequests);
    expect(mocks.ensureClipThumbnails).toHaveBeenCalledTimes(ensureRequests);

    fireEvent.error(heroImage);
    await waitFor(() => {
      expect(document.querySelector(".cinematic-artwork--hero img")).not.toBeInTheDocument();
    });

    act(() => {
      mocks.thumbnailListener?.({
        clipId: "101",
        status: "evicted",
        revision: null,
        errorCode: null,
      });
    });
    expect(document.querySelector(".cinematic-artwork--hero img")).not.toBeInTheDocument();

    act(() => {
      mocks.thumbnailListener?.({
        clipId: "101",
        status: "ready",
        revision: "revision-1",
        errorCode: null,
      });
    });
    const retriedHeroImage = await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(".cinematic-artwork--hero img");
      expect(image).toBeInTheDocument();
      return image!;
    });
    expect(retriedHeroImage).not.toBe(heroImage);
    expect(retriedHeroImage.src).toContain("?v=revision-1");
    expect(mocks.listClipPage).toHaveBeenCalledTimes(listRequests);
    expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(facetRequests);
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(mediaRequests);
    expect(mocks.ensureClipThumbnails).toHaveBeenCalledTimes(ensureRequests);

    await user.click(screen.getByRole("button", { name: "返回素材库" }));
    await waitFor(() => {
      expect(document.querySelector("[data-clip-id='101'] .match-clip-thumb img"))
        .toBeInTheDocument();
    });
    expect(mocks.listClipPage).toHaveBeenCalledTimes(listRequests);
    expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(facetRequests);
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(mediaRequests);
  });

  it("preserves a source cover when thumbnail generation is suppressed", async () => {
    const sourceCoverUrl = "https://example.test/source-cover-101.jpg";
    mocks.listClipPage.mockResolvedValue(page([
      createClip("101", {
        officialVideoName: "分页高光 A",
        thumbnailUrl: sourceCoverUrl,
        thumbnailStatus: undefined,
        thumbnailRevision: null,
      }),
    ], 1));

    render(<App />);
    await waitFor(() => expect(mocks.thumbnailListener).not.toBeNull());
    const sourceCover = await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(
        "[data-clip-id='101'] .match-clip-thumb img",
      );
      expect(image).toBeInTheDocument();
      return image!;
    });
    expect(sourceCover.src).toBe(sourceCoverUrl);

    act(() => {
      mocks.thumbnailListener?.({
        clipId: "101",
        status: "suppressed",
        revision: null,
        errorCode: null,
      });
    });

    expect(document.querySelector("[data-clip-id='101'] .match-clip-thumb img"))
      .toBe(sourceCover);
    expect(sourceCover.src).toBe(sourceCoverUrl);
    expect(mocks.listClipPage).toHaveBeenCalledTimes(1);
  });

  it("loads only nextOffset, deduplicates by clip id, preserves order, and supports keyboard loading", async () => {
    mocks.listClipPage.mockImplementation(async (query: ClipListQuery) =>
      query.offset === 2
        ? page([clipB, clipC], 3)
        : page([clipA, clipB], 3, 2),
    );
    const user = userEvent.setup();
    render(<App />);

    const loadMore = await screen.findByRole("button", { name: /加载更多（2 \/ 3）/ });
    loadMore.focus();
    await user.keyboard("{Enter}");

    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    expect(mocks.listClipPage.mock.calls.map(([query]) => query.offset)).toEqual([0, 2]);
    await waitFor(() => {
      expect(document.querySelectorAll('[data-clip-id="102"]')).toHaveLength(1);
      expect(clipCardIds()).toEqual(["101", "102", "103"]);
    });
    expect(screen.getByText("已加载全部 3 条素材")).toBeVisible();
  });

  it("loads the next page once when the library scroll approaches the end", async () => {
    const nextPage = deferred<ClipPage>();
    mocks.listClipPage
      .mockResolvedValueOnce(page([clipA], 2, 1))
      .mockReturnValueOnce(nextPage.promise);
    render(<App />);
    await screen.findByRole("button", { name: /加载更多（1 \/ 2）/ });
    const scrollRegion = document.querySelector<HTMLElement>(".library-workspace-scroll");
    if (!scrollRegion) throw new Error("library scroll region not found");
    Object.defineProperties(scrollRegion, {
      scrollTop: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 1200 },
      clientHeight: { configurable: true, value: 500 },
    });

    fireEvent.scroll(scrollRegion);
    fireEvent.scroll(scrollRegion);
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    expect(lastListQuery().offset).toBe(1);

    await act(async () => {
      nextPage.resolve(page([clipB], 2));
      await nextPage.promise;
    });
    expect(mocks.listClipPage).toHaveBeenCalledTimes(2);
  });

  it("starts filter, debounced search, and sort generations from offset zero", async () => {
    mocks.listClipPage.mockResolvedValue(page([], 0));
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: /^收藏/ }));
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    expect(lastListQuery()).toEqual(expect.objectContaining({ offset: 0, favoriteFilter: "favorite" }));

    fireEvent.change(screen.getByPlaceholderText("搜索账号、英雄、地图、标签、文件名…"), {
      target: { value: "  ACE  " },
    });
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(3), { timeout: 1000 });
    expect(lastListQuery()).toEqual(expect.objectContaining({ offset: 0, query: "ACE" }));

    await user.click(screen.getByRole("combobox", { name: "排序" }));
    await user.click(await screen.findByRole("option", { name: "文件名" }));
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(4));
    expect(lastListQuery()).toEqual(expect.objectContaining({ offset: 0, sortBy: "name-asc" }));
  });

  it("ignores an old query page after a newer generation has rendered", async () => {
    const oldPage = deferred<ClipPage>();
    mocks.listClipPage
      .mockReturnValueOnce(oldPage.promise)
      .mockResolvedValueOnce(page([clipB], 1));
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: /^收藏/ }));
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(card("102")).toBeInTheDocument());

    await act(async () => {
      oldPage.resolve(page([clipA], 1));
      await oldPage.promise;
    });
    expect(document.querySelector('[data-clip-id="101"]')).not.toBeInTheDocument();
    expect(card("102")).toBeInTheDocument();
  });

  it("offers independent initial and load-more retries without losing successful pages", async () => {
    mocks.listClipPage
      .mockRejectedValueOnce(new Error("initial offline"))
      .mockResolvedValueOnce(page([clipA], 2, 1))
      .mockRejectedValueOnce(new Error("next offline"))
      .mockResolvedValueOnce(page([clipB], 2));
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "重试加载" }));
    await waitFor(() => expect(card("101")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: /加载更多（1 \/ 2）/ }));
    expect(await screen.findByText(/更多素材加载失败：next offline/)).toBeVisible();
    expect(card("101")).toBeInTheDocument();

    const retryMore = screen.getByRole("button", { name: "重试加载更多" });
    retryMore.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(card("102")).toBeInTheDocument());
    expect(mocks.listClipPage.mock.calls.map(([query]) => query.offset)).toEqual([0, 0, 1, 1]);
  });

  it("does not fetch detail before selection and reuses a valid selected-clip cache entry", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());
    expect(mocks.getClipDetail).not.toHaveBeenCalled();

    await openPreview(user, "101");
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledWith("101"));
    await user.click(screen.getByRole("button", { name: "返回素材库" }));
    await openPreview(user, "101");
    expect((await screen.findAllByText("分页高光 A")).length).toBeGreaterThan(0);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(1);
  });

  it("restores the library scroll position after returning from preview", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());

    const scrollRegion = document.querySelector<HTMLElement>(".library-workspace-scroll");
    const libraryWorkspace = scrollRegion?.closest<HTMLElement>(".library-workspace");
    if (!scrollRegion || !libraryWorkspace) {
      throw new Error("library scroll region not found");
    }
    scrollRegion.scrollTop = 684;
    fireEvent.scroll(scrollRegion);

    await openPreview(user, "101");
    await screen.findByRole("button", { name: "返回素材库" });
    expect(scrollRegion).toBeInTheDocument();
    expect(libraryWorkspace).toHaveAttribute("hidden");

    await user.click(screen.getByRole("button", { name: "返回素材库" }));
    expect(document.querySelector(".library-workspace-scroll")).toBe(scrollRegion);
    expect(scrollRegion.scrollTop).toBe(684);
    expect(libraryWorkspace).not.toHaveAttribute("hidden");
  });

  it("keeps the current detail ready when its active rail item is clicked again", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());

    await openPreview(user, "101");
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledWith("101"));
    const activeRailItem = document.querySelector<HTMLButtonElement>(
      ".preview-rail-clip--active",
    );
    expect(activeRailItem).not.toBeNull();
    await user.click(activeRailItem!);

    expect(screen.queryByRole("heading", { name: "正在加载素材详情" }))
      .not.toBeInTheDocument();
    expect(screen.getAllByText("分页高光 A").length).toBeGreaterThan(0);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(1);
  });

  it("switches the active rail clip without flashing the full detail loader", async () => {
    const relatedClipB = createClip("102", {
      officialVideoName: "分页高光 B",
      accountId: clipA.accountId,
      matchId: clipA.matchId,
      clipGroupId: clipA.clipGroupId,
      clipGroupName: clipA.clipGroupName,
      modifiedAt: "2026-07-16T10:01:00+08:00",
    });
    const detailB = deferred<Clip>();
    mocks.listClipPage.mockResolvedValue(page([clipA, relatedClipB], 2));
    mocks.getClipDetail.mockImplementation((clipId: string) =>
      clipId === clipA.id
        ? Promise.resolve(cloneClip(clipA))
        : detailB.promise,
    );
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());

    await openPreview(user, "101");
    await screen.findByPlaceholderText("记录复盘重点或剪辑思路");
    await user.click(screen.getByRole("button", { name: /分页高光 B/ }));

    expect(screen.queryByRole("heading", { name: "正在加载素材详情" }))
      .not.toBeInTheDocument();
    expect(document.querySelector(".preview-rail-clip--active"))
      .toHaveTextContent("分页高光 B");
    expect(screen.getByPlaceholderText("正在加载备注…")).toBeDisabled();
    expect(screen.getByText("SYNCING")).toBeVisible();

    await act(async () => {
      detailB.resolve(cloneClip(relatedClipB));
      await detailB.promise;
    });
    await waitFor(() => {
      expect(screen.getByPlaceholderText("记录复盘重点或剪辑思路"))
        .toHaveValue("note-102");
    });
    expect(screen.queryByText("SYNCING")).not.toBeInTheDocument();
  });

  it("isolates a closed old detail request from a quickly selected clip and stale not-found", async () => {
    const detailA = deferred<Clip>();
    const detailB = deferred<Clip>();
    mocks.getClipDetail.mockImplementation((clipId: string) =>
      clipId === "101" ? detailA.promise : detailB.promise,
    );
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());

    await openPreview(user, "101");
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledWith("101"));
    await user.click(screen.getByRole("button", { name: "返回素材库" }));
    await openPreview(user, "102");
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledWith("102"));

    await act(async () => {
      detailB.resolve(cloneClip(clipB));
      await detailB.promise;
    });
    expect((await screen.findAllByText("分页高光 B")).length).toBeGreaterThan(0);
    await act(async () => {
      detailA.reject({ code: "clip-not-found", message: "old clip gone", clipId: 101 });
      await detailA.promise.catch(() => undefined);
    });
    expect(screen.queryByText("素材已不存在")).not.toBeInTheDocument();
    expect(screen.getAllByText("分页高光 B").length).toBeGreaterThan(0);
  });

  it("renders structured not-found separately and provides a detail retry", async () => {
    mocks.getClipDetail.mockRejectedValueOnce({
      code: "clip-not-found",
      message: "clip 101 no longer exists",
      clipId: 101,
    }).mockResolvedValueOnce(cloneClip(clipA));
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());

    await openPreview(user, "101");
    expect(await screen.findByRole("heading", { name: "素材已不存在" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "重试详情" }));
    expect((await screen.findAllByText("分页高光 A")).length).toBeGreaterThan(0);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);
  });

  it("syncs note and favorite mutations into detail and summary without reloading media", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(card("101")).toBeInTheDocument());
    await openPreview(user, "101");
    const note = await screen.findByPlaceholderText("记录复盘重点或剪辑思路");
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledTimes(1));

    fireEvent.change(note, { target: { value: "新的复盘备注" } });
    await user.click(screen.getByRole("button", { name: "保存备注" }));
    await waitFor(() => expect(mocks.updateClipNote).toHaveBeenCalledWith("101", "新的复盘备注"));
    expect(screen.getByPlaceholderText("记录复盘重点或剪辑思路")).toHaveValue("新的复盘备注");
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "收藏" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "取消收藏" })).toBeVisible());
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "返回素材库" }));
    expect(within(card("101")).getByRole("button", { name: "取消收藏" })).toHaveAttribute("aria-pressed", "true");
  });
});

function createClip(id: string, overrides: Partial<Clip> = {}): Clip {
  const base = mockClips[0];
  return {
    ...base,
    id,
    fileName: `clip-${id}.mp4`,
    filePath: `D:\\Highlights\\clip-${id}.mp4`,
    clipGroupId: `group-${id}`,
    clipGroupName: `group-${id}`,
    matchId: `match-${id}`,
    isFavorite: false,
    note: `note-${id}`,
    tags: [...base.tags],
    clipEvents: base.clipEvents?.map((event) => ({ ...event })),
    ...overrides,
  };
}

function cloneClip(clip: Clip): Clip {
  return {
    ...clip,
    tags: [...clip.tags],
    clipEvents: clip.clipEvents?.map((event) => ({ ...event })),
  };
}

function summary(clip: Clip): ClipSummary {
  const value = cloneClip(clip);
  const partial = value as Partial<Clip>;
  delete partial.note;
  delete partial.extractedText;
  delete partial.clipEvents;
  delete partial.eventCount;
  delete partial.roundLabel;
  delete partial.weaponName;
  return value;
}

function page(clips: Clip[], totalCount: number, nextOffset: number | null = null): ClipPage {
  return {
    items: clips.map(summary),
    offset: nextOffset === null ? 0 : Math.max(0, nextOffset - clips.length),
    limit: 50,
    totalCount,
    hasMore: nextOffset !== null,
    nextOffset,
  };
}

function card(clipId: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(`[data-clip-id="${clipId}"]`);
  if (!element) throw new Error(`clip card not found: ${clipId}`);
  return element;
}

function clipCardIds(): string[] {
  return [...document.querySelectorAll<HTMLElement>("[data-clip-id]")]
    .map((element) => element.dataset.clipId ?? "");
}

async function openPreview(user: ReturnType<typeof userEvent.setup>, clipId: string) {
  await user.click(within(card(clipId)).getByRole("button", { name: /^预览/ }));
}

function lastListQuery(): ClipListQuery {
  return mocks.listClipPage.mock.calls.at(-1)?.[0] as ClipListQuery;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, reject, resolve };
}
