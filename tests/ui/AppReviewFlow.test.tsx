import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import {
  APP_PREFERENCES_STORAGE_KEY,
  DEFAULT_APP_PREFERENCES,
} from "../../src/lib/appPreferences";
import type { ClipListQuery, ClipPage, ClipSummary, SourceDir, Tag } from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  getClipMedia: vi.fn(),
  getLibraryFacets: vi.fn(),
  listClipPage: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  setClipReviewDecision: vi.fn(),
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    getClipMedia: mocks.getClipMedia,
    getLibraryFacets: mocks.getLibraryFacets,
    listClipPage: mocks.listClipPage,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    setClipReviewDecision: mocks.setClipReviewDecision,
    listenToScanProgress: vi.fn(async () => () => undefined),
  };
});

import App from "../../src/App";

const source: SourceDir = {
  id: "1",
  name: "NVIDIA 录屏",
  displayName: "NVIDIA 录屏",
  path: "D:\\Recordings",
  sourceKind: "nvidia",
  scanMode: "recursive-mp4",
  scanRootPath: "D:\\Recordings",
  enabled: true,
  status: "ready",
  accessibility: true,
  lastError: null,
  clipCount: 1,
  lastScanAt: null,
};

const reviewClip: ClipSummary = {
  ...mockClips[0],
  id: "10",
  accountId: "winter",
  accountDisplayName: "Winter#0001",
  sourceDirId: "1",
  sourceDirName: source.displayName,
  sourceDirPath: source.path,
  scanRootPath: source.scanRootPath,
  sourceKind: source.sourceKind,
  scanMode: source.scanMode,
  agentName: "尚勃勒",
  mapName: "隐世修所",
  gameMode: "竞技模式",
  reviewDecision: "unreviewed",
  reviewedAt: null,
  isFavorite: false,
  tags: [],
};

const tag: Tag = { id: "12", label: "小红书待剪", color: "red" };

describe("App quick-pick integration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      activeCount: 1,
      totalCount: 1,
      accounts: [{
        accountIdentityKey: "winter",
        accountDisplayName: "Winter#0001",
        count: 1,
        activeCount: 1,
      }],
      agents: [{ value: reviewClip.agentName, count: 1, activeCount: 1 }],
      maps: [{ value: reviewClip.mapName, count: 1, activeCount: 1 }],
      gameModes: [{ value: reviewClip.gameMode, count: 1, activeCount: 1 }],
    }));
    mocks.listSources.mockResolvedValue([source]);
    mocks.listTags.mockResolvedValue([tag]);
    mocks.listClipPage.mockResolvedValue(page([reviewClip]));
    mocks.getClipMedia.mockResolvedValue({
      clipId: reviewClip.id,
      playable: false,
      mediaUrl: null,
      message: "测试媒体不可播放",
    });
  });

  it("opens 快速挑片 immediately when it is the saved startup destination", async () => {
    window.localStorage.setItem(APP_PREFERENCES_STORAGE_KEY, JSON.stringify({
      ...DEFAULT_APP_PREFERENCES,
      startupDestination: "review",
      reviewAutoplay: false,
    }));

    render(<App />);

    await waitFor(() => expect(screen.getByRole("heading", { name: "快速挑片" })).toBeVisible(), {
      timeout: 5_000,
    });
    expect(screen.getByRole("button", { name: "快速挑片" })).toHaveAttribute("aria-current", "page");
  });

  it("resets quick-pick conditions on entry and keeps them separate from the material library", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: /素材库/ });

    await user.click(screen.getByRole("combobox", { name: "账号" }));
    await user.click(await screen.findByRole("option", { name: "Winter#0001" }));
    await user.type(screen.getByLabelText("全局搜索素材"), "library-only");
    await waitFor(() => expect(latestLibraryQuery()).toEqual(expect.objectContaining({
      accountId: "winter",
      query: "library-only",
    })));

    await user.click(screen.getByRole("button", { name: "快速挑片" }));
    expect(await screen.findByRole("heading", { name: "快速挑片" })).toBeVisible();
    await waitFor(() => expect(latestReviewQuery()).toEqual({
      sortBy: "modified-desc",
      offset: 0,
      limit: 200,
    }));

    await user.click(screen.getByRole("button", { name: "修改范围" }));
    expect(screen.getByLabelText("搜索素材")).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "账号" })).toHaveTextContent("全部账号");
    expect(screen.getByRole("combobox", { name: "英雄" })).toHaveTextContent("全部英雄");
    expect(screen.getByRole("combobox", { name: "地图" })).toHaveTextContent("全部地图");
    expect(screen.getByRole("combobox", { name: "模式" })).toHaveTextContent("全部模式");
    expect(screen.getByRole("combobox", { name: "日期" })).toHaveTextContent("全部日期");
    expect(screen.getByRole("combobox", { name: "视频类型" })).toHaveTextContent("全部类型");
    expect(screen.getByRole("combobox", { name: "自定义标签" })).toHaveTextContent("全部自定义标签");
    expect(screen.getByRole("radio", { name: /最新优先/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /全部素材/ })).toBeChecked();

    await chooseReviewScopeOption(user, "账号", "Winter#0001");
    await user.type(screen.getByLabelText("搜索素材"), "quick-pick-only");
    await waitFor(() => expect(latestReviewQuery()).toEqual(expect.objectContaining({
      accountId: "winter",
      query: "quick-pick-only",
      sortBy: "modified-desc",
      offset: 0,
      limit: 200,
    })));

    await user.click(screen.getByRole("button", { name: /^全部素材/ }));
    await screen.findByRole("heading", { name: /素材库/ });
    expect(screen.getByLabelText("全局搜索素材")).toHaveValue("library-only");
    expect(screen.getByRole("combobox", { name: "账号" })).toHaveTextContent("Winter#0001");
  }, 15_000);

  it("keeps quick-pick decisions session-only and reuses selected-result batch UI", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: /素材库/ });

    await user.click(screen.getByRole("button", { name: "快速挑片" }));
    await screen.findByRole("heading", { name: "快速挑片" });
    await user.click(screen.getByRole("button", { name: /开始.*挑片/ }));
    await screen.findByRole("article", { name: "当前挑片素材" });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "d" }));
    await screen.findByRole("heading", { name: "本轮挑片完成" });
    expect(mocks.setClipReviewDecision).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "批量添加标签" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent("批量编辑自定义标签");
    expect(screen.getByRole("dialog")).toHaveTextContent("已选择 1 条素材");
  });

  it("changes filtering conditions directly in 快速挑片 and refreshes its candidate query", async () => {
    const user = userEvent.setup();
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      activeCount: 2,
      totalCount: 2,
      accounts: [
        {
          accountIdentityKey: "winter",
          accountDisplayName: "Winter#0001",
          count: 1,
          activeCount: 1,
        },
        {
          accountIdentityKey: "summer",
          accountDisplayName: "Summer#0002",
          count: 1,
          activeCount: 1,
        },
      ],
      agents: [
        { value: "尚勃勒", count: 1, activeCount: 1 },
        { value: "贤者", count: 1, activeCount: 1 },
      ],
      maps: [
        { value: "隐世修所", count: 1, activeCount: 1 },
        { value: "森寒冬港", count: 1, activeCount: 1 },
      ],
      gameModes: [
        { value: "竞技模式", count: 1, activeCount: 1 },
        { value: "极速模式", count: 1, activeCount: 1 },
      ],
    }));
    render(<App />);
    await screen.findByRole("heading", { name: /素材库/ });

    await user.click(screen.getByRole("button", { name: "快速挑片" }));
    await screen.findByRole("heading", { name: "快速挑片" });
    await user.click(screen.getByRole("button", { name: "修改范围" }));

    await chooseReviewScopeOption(user, "账号", "Summer#0002");
    await chooseReviewScopeOption(user, "英雄", "贤者");
    await chooseReviewScopeOption(user, "地图", "森寒冬港");
    await chooseReviewScopeOption(user, "模式", "极速模式");
    await chooseReviewScopeOption(user, "日期", "近 7 天");
    await chooseReviewScopeOption(user, "视频类型", "三杀时刻");
    await chooseReviewScopeOption(user, "自定义标签", "小红书待剪");
    await user.type(screen.getByLabelText("搜索素材"), "Summer");
    await waitFor(() => expect(latestReviewQuery()).toEqual(expect.objectContaining({
      accountId: "summer",
      agentName: "贤者",
      mapName: "森寒冬港",
      gameMode: "极速模式",
      modifiedFrom: expect.any(Number),
      modifiedTo: expect.any(Number),
      highlightFilter: "triple",
      tagId: 12,
      query: "Summer",
    })));

    expect(screen.getByRole("heading", { name: "快速挑片" })).toBeVisible();
    expect(screen.queryByRole("heading", { name: /素材库/ })).not.toBeInTheDocument();
  }, 15_000);
});

async function chooseReviewScopeOption(
  user: ReturnType<typeof userEvent.setup>,
  label: string,
  option: string,
) {
  await user.click(screen.getByRole("combobox", { name: label }));
  await user.click(await screen.findByRole("option", { name: option }));
}

function latestReviewQuery(): ClipListQuery | undefined {
  const requests = mocks.listClipPage.mock.calls
    .map(([request]) => request as ClipListQuery)
    .filter((request) => request.offset === 0 && request.limit === 200);
  return requests.at(-1);
}

function latestLibraryQuery(): ClipListQuery | undefined {
  const requests = mocks.listClipPage.mock.calls
    .map(([request]) => request as ClipListQuery)
    .filter((request) => request.offset === 0 && request.limit === 50);
  return requests.at(-1);
}

function page(items: ClipSummary[]): ClipPage {
  return {
    items,
    offset: 0,
    limit: 50,
    totalCount: items.length,
    hasMore: false,
    nextOffset: null,
  };
}
