import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type {
  Clip,
  ClipListQuery,
  ClipPage,
  ClipSummary,
  LibraryFacets,
  SourceDir,
} from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  getLibraryFacets: vi.fn(),
  listClipPage: vi.fn(),
  listClips: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  setClipFavorite: vi.fn(),
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
    setClipFavorite: mocks.setClipFavorite,
    listenToScanProgress: vi.fn(async () => () => undefined),
  };
});

import App from "../../src/App";

const source: SourceDir = {
  id: "7",
  name: "页外来源",
  displayName: "页外来源",
  path: "D:\\Highlights\\wonderfulVideos7001",
  sourceKind: "aclos",
  scanMode: "aclos-structured",
  scanRootPath: "D:\\Highlights\\wonderfulVideos7001",
  enabled: true,
  status: "available",
  accessibility: true,
  lastError: null,
  clipCount: 1,
  lastScanAt: null,
};
const loadedClip = createClip("101");

describe("whole-library facet consumption", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getLibraryFacets.mockReset();
    mocks.listClipPage.mockReset();
    mocks.listSources.mockReset();
    mocks.listTags.mockReset();
    mocks.setClipFavorite.mockReset();
    mocks.getLibraryFacets.mockResolvedValue(fullFacets());
    mocks.listClipPage.mockResolvedValue(page([loadedClip], 37));
    mocks.listSources.mockResolvedValue([{ ...source }]);
    mocks.listTags.mockResolvedValue([]);
    mocks.setClipFavorite.mockImplementation(async (_clipId: string, isFavorite: boolean) => ({
      ...cloneClip(loadedClip),
      isFavorite,
    }));
  });

  it("offers account, agent, map, mode, and tag values that are absent from the loaded page", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("1 / 37 条片段")).toBeVisible();
    expect(screen.getByText("900 个素材")).toBeVisible();

    for (const [filter, option] of [
      ["账号", "页外账号"],
      ["英雄", "页外英雄"],
      ["地图", "页外地图"],
      ["模式", "页外模式"],
      ["自定义标签", "页外标签"],
    ] as const) {
      await user.click(screen.getByRole("combobox", { name: filter }));
      expect(await screen.findByRole("option", { name: option })).toBeVisible();
      await user.keyboard("{Escape}");
    }
    expect(mocks.listClips).not.toHaveBeenCalled();
  });

  it("does not expose source as a library toolbar column", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    expect(screen.queryByRole("combobox", { name: "来源" })).not.toBeInTheDocument();
    expect(lastListQuery()).not.toHaveProperty("sourceDirId");
  });

  it("uses facet source and tag usage counts instead of the loaded summary", async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: "扫描目录" }));
    const sourceRow = (await screen.findByText("页外来源")).closest("article");
    expect(sourceRow).not.toBeNull();
    expect(within(sourceRow as HTMLElement).getByText("700 个片段")).toBeVisible();

    await user.click(screen.getByRole("button", { name: /识别结果/ }));
    expect(screen.getByText("800 个片段")).toBeVisible();

    await user.click(screen.getByRole("button", { name: /^自定义标签/ }));
    const tagRow = (await screen.findByText("页外标签")).closest("article");
    expect(tagRow).not.toBeNull();
    expect(within(tagRow as HTMLElement).getByText("700")).toBeVisible();
    expect(screen.getByText("640")).toBeVisible();
    expect(screen.getByText("260")).toBeVisible();
  });

  it("does not reload facets when loading another result page", async () => {
    mocks.listClipPage.mockImplementation(async (query: ClipListQuery) =>
      query.offset === 1
        ? page([createClip("102")], 2)
        : page([loadedClip], 2, 1),
    );
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    await user.click(await screen.findByRole("button", { name: /加载更多（1 \/ 2）/ }));
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledTimes(2));
    expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1);
  });

  it("refreshes after explicit refresh and favorite mutation while isolating an older response", async () => {
    const oldRefresh = deferred<LibraryFacets>();
    const mutationRefresh = deferred<LibraryFacets>();
    mocks.getLibraryFacets
      .mockResolvedValueOnce(fullFacets())
      .mockReturnValueOnce(oldRefresh.promise)
      .mockReturnValueOnce(mutationRefresh.promise);
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: "刷新索引" }));
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(card("101")).toBeInTheDocument());
    await user.click(within(card("101")).getByRole("button", { name: "收藏" }));
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(3));

    await act(async () => {
      mutationRefresh.resolve(fullFacets({ activeCount: 222 }));
      await mutationRefresh.promise;
    });
    expect(await screen.findByText("222 个素材")).toBeVisible();

    await act(async () => {
      oldRefresh.resolve(fullFacets({ activeCount: 111 }));
      await oldRefresh.promise;
    });
    expect(screen.getByText("222 个素材")).toBeVisible();
    expect(screen.queryByText("111 个素材")).not.toBeInTheDocument();
  });

  it("keeps a selected zero-count facet visible and clearable after a new response omits it", async () => {
    mocks.getLibraryFacets
      .mockResolvedValueOnce(fullFacets())
      .mockResolvedValueOnce(libraryFacets({ activeCount: 900, totalCount: 1_000 }));
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("combobox", { name: "英雄" }));
    await user.click(await screen.findByRole("option", { name: "页外英雄" }));
    await waitFor(() => expect(lastListQuery()).toEqual(expect.objectContaining({ agentName: "页外英雄" })));
    await user.click(screen.getByRole("button", { name: "刷新索引" }));
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(2));

    await user.click(screen.getByRole("combobox", { name: "英雄" }));
    expect(await screen.findByRole("option", { name: "页外英雄" })).toBeVisible();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "清空搜索与所有筛选" }));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "英雄" })).toHaveTextContent("全部英雄"));
  });

  it("keeps pagination usable on first facet failure and retains last-known-good data later", async () => {
    mocks.getLibraryFacets
      .mockRejectedValueOnce(new Error("facet offline"))
      .mockResolvedValueOnce(fullFacets({ activeCount: 80 }))
      .mockRejectedValueOnce(new Error("facet refresh failed"));
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByText(/统计加载失败：facet offline/)).toBeVisible();
    expect(card("101")).toBeInTheDocument();
    expect(screen.getByText("1 / 37 条片段")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "刷新索引" }));
    expect(await screen.findByText("80 个素材")).toBeVisible();
    expect(screen.queryByText(/facet offline/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "刷新索引" }));
    expect(await screen.findByText(/统计加载失败：facet refresh failed/)).toBeVisible();
    expect(screen.getByText("80 个素材")).toBeVisible();
    expect(card("101")).toBeInTheDocument();
  });
});

function fullFacets(overrides: Partial<LibraryFacets> = {}): LibraryFacets {
  return libraryFacets({
    totalCount: 1_000,
    activeCount: 900,
    favoriteCount: 130,
    activeFavoriteCount: 120,
    trashedCount: 100,
    taggedCount: 730,
    activeTaggedCount: 640,
    totalSizeBytes: 9_000,
    activeSizeBytes: 8_000,
    recentCount: 20,
    modifiedAtMax: 1_800_000_000,
    accounts: [{
      accountIdentityKey: "match-account-outside",
      accountDisplayName: "页外账号",
      count: 850,
      activeCount: 800,
    }],
    sourceDirs: [{ sourceDirId: 7, count: 720, activeCount: 700 }],
    agents: [{ value: "页外英雄", count: 710, activeCount: 690 }],
    maps: [{ value: "页外地图", count: 680, activeCount: 660 }],
    gameModes: [{ value: "页外模式", count: 650, activeCount: 630 }],
    killTypes: [{ value: "triple", count: 50, activeCount: 48 }],
    tags: [{ id: 11, name: "页外标签", color: "red", count: 720, activeCount: 700 }],
    ...overrides,
  });
}

function createClip(id: string): Clip {
  const base = mockClips[0];
  return {
    ...base,
    id,
    fileName: `clip-${id}.mp4`,
    filePath: `D:\\Highlights\\clip-${id}.mp4`,
    sourceDirId: "7",
    sourceDirName: "页外来源",
    sourceDirPath: source.path,
    clipGroupId: `group-${id}`,
    clipGroupName: `group-${id}`,
    matchId: `match-${id}`,
    accountId: "match-account-loaded",
    accountDisplayName: "首屏账号",
    agentName: "首屏英雄",
    mapName: "首屏地图",
    gameMode: "首屏模式",
    isFavorite: false,
    tags: [],
    clipEvents: base.clipEvents?.map((event) => ({ ...event })),
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
  for (const field of ["note", "extractedText", "clipEvents", "eventCount", "roundLabel", "weaponName"] as const) {
    delete partial[field];
  }
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

function lastListQuery(): ClipListQuery {
  return mocks.listClipPage.mock.calls.at(-1)?.[0] as ClipListQuery;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}
