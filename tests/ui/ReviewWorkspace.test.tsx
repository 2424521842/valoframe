import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import { REVIEW_SESSION_STORAGE_KEY } from "../../src/lib/reviewSessions";
import type { ReviewScopeEditor } from "../../src/components/review/ReviewSetup";
import { ReviewWorkspace } from "../../src/screens/ReviewWorkspace";
import type { ClipPage, ClipSummary, ReviewSessionFilters } from "../../src/types";

const mocks = vi.hoisted(() => ({
  getClipMedia: vi.fn(),
  listClipPage: vi.fn(),
  setClipReviewDecision: vi.fn(),
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    getClipMedia: mocks.getClipMedia,
    listClipPage: mocks.listClipPage,
    setClipReviewDecision: mocks.setClipReviewDecision,
  };
});

const clips = Array.from({ length: 5 }, (_, index) => ({
  ...mockClips[index % mockClips.length],
  id: String(index + 1),
  agentName: "尚勃勒",
  accountDisplayName: "Winter#0001",
  mapName: "隐世修所",
  gameMode: "竞技模式",
  createdAt: `2026-08-0${index + 1}T12:00:00.000Z`,
  modifiedAt: `2026-08-0${index + 1}T12:00:00.000Z`,
  kda: "20 / 10 / 5",
  killCount: 20 - index,
  combatScore: 260 + index,
  isFavorite: false,
  tags: ["existing-tag"],
  reviewDecision: "unreviewed" as const,
  reviewedAt: null,
  thumbnailUrl: null,
})) as ClipSummary[];

const inheritedFilters: ReviewSessionFilters = {
  query: {
    accountId: "winter",
    agentName: "尚勃勒",
    mapName: "隐世修所",
    gameMode: "竞技模式",
    modifiedFrom: 1_784_649_600,
    modifiedTo: 1_787_241_599,
    sortBy: "name-asc",
  },
  labels: ["账号：Winter", "英雄：尚勃勒", "地图：隐世修所", "模式：竞技模式", "日期：最近 30 天"],
  sort: "library",
  candidateScope: "all",
};

describe("ReviewWorkspace", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
      matches: query.includes("prefers-reduced-motion"),
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => false,
    }) as MediaQueryList);
    mocks.listClipPage.mockResolvedValue(page(clips));
    mocks.getClipMedia.mockImplementation(async (clipId: string) => ({
      clipId,
      playable: false,
      mediaUrl: null,
      message: "测试环境不播放本地视频",
    }));
  });

  it("edits inherited conditions inline without leaving 快速挑片", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    const onAccountChange = vi.fn();
    const onQueryChange = vi.fn();
    renderWorkspace({
      onBack,
      scopeEditor: createScopeEditor({ onAccountChange, onQueryChange }),
    });

    expect(await screen.findByRole("heading", { name: "快速挑片" })).toBeVisible();
    expect(screen.getByText("账号：Winter")).toBeVisible();
    expect(screen.getByText("英雄：尚勃勒")).toBeVisible();
    expect(screen.queryByRole("combobox", { name: "账号" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "修改范围" }));
    expect(screen.getByRole("heading", { name: "调整筛选范围" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "账号" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "英雄" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "地图" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "模式" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "日期" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "视频类型" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "自定义标签" })).toBeVisible();
    expect(screen.getByLabelText("搜索素材")).toBeVisible();

    await user.click(screen.getByRole("combobox", { name: "账号" }));
    await user.click(await screen.findByRole("option", { name: "Summer#0002" }));
    expect(onAccountChange).toHaveBeenCalledWith("summer");
    fireEvent.change(screen.getByLabelText("搜索素材"), { target: { value: "ace" } });
    expect(onQueryChange).toHaveBeenCalledWith("ace");
    expect(onBack).not.toHaveBeenCalled();
    expect(screen.getByText("5 条素材")).toBeVisible();
    await waitFor(() => expect(mocks.listClipPage).toHaveBeenCalledWith({
      ...inheritedFilters.query,
      offset: 0,
      limit: 200,
    }));
    expect(mocks.listClipPage.mock.calls[0][0]).not.toHaveProperty("reviewDecision");
  });

  it("records D, D, A, S, D as session-only selected, pending, and skipped decisions", async () => {
    renderWorkspace();
    await startReview();

    for (const [index, key] of ["d", "d", "a", "s", "d"].entries()) {
      fireEvent.keyDown(window, { key });
      await waitFor(() => expect(screen.getByText(`${index + 1} / 5`)).toBeVisible());
    }

    const heading = await screen.findByRole("heading", { name: "本轮挑片完成" });
    expect(heading).toBeVisible();
    const stats = document.querySelector<HTMLElement>(".review-complete-stats")!;
    expect(within(stats).getByText("入选").nextElementSibling).toHaveTextContent("3");
    expect(within(stats).getByText("待定").nextElementSibling).toHaveTextContent("1");
    expect(within(stats).getByText("跳过").nextElementSibling).toHaveTextContent("1");
    expect(mocks.setClipReviewDecision).not.toHaveBeenCalled();
    expect(window.localStorage.getItem(REVIEW_SESSION_STORAGE_KEY)).toContain('"selected"');

    fireEvent.keyDown(window, { key: "z" });
    expect(await screen.findByRole("article", { name: "当前挑片素材" })).toBeVisible();
    expect(screen.getByText("4 / 5")).toBeVisible();
  });

  it("loads media for only the active card, protects inputs, and keeps keyboard choices usable after a button click", async () => {
    renderWorkspace();
    await startReview();
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledWith("1"));
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(1);

    const input = document.createElement("input");
    document.body.append(input);
    input.focus();
    fireEvent.keyDown(window, { key: "d" });
    expect(screen.getByText("0 / 5")).toBeVisible();
    input.remove();

    const selectAction = screen.getByRole("button", { name: /^入选D \/ →$/ });
    await userEvent.click(selectAction);
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledWith("2"));
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(2);

    fireEvent.keyDown(selectAction, { key: "d" });
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledWith("3"));
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(3);
  });

  it("keeps the next card visible and permits unmuted manual playback", async () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
    vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => undefined);
    mocks.getClipMedia.mockImplementation(async (clipId: string) => ({
      clipId,
      playable: true,
      mediaUrl: `https://media.test/${clipId}.mp4`,
      message: null,
    }));

    renderWorkspace({ autoplay: false, initialMuted: false, initialVolumePercent: 70 });
    await startReview();
    const firstVideo = await waitFor(() => {
      const video = document.querySelector<HTMLVideoElement>(".review-card-media video");
      expect(video?.getAttribute("src")).toBe("https://media.test/1.mp4");
      return video!;
    });
    expect(firstVideo.muted).toBe(false);
    expect(firstVideo.volume).toBeCloseTo(0.7);

    await userEvent.click(screen.getByRole("button", { name: "播放或暂停预览" }));
    expect(play).toHaveBeenCalled();
    expect(firstVideo.muted).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: /^入选D \/ →$/ }));
    await waitFor(() => {
      expect(document.querySelector<HTMLVideoElement>(".review-card-media video")?.getAttribute("src"))
        .toBe("https://media.test/2.mp4");
    });
    expect(screen.getByRole("article", { name: "当前挑片素材" })).not.toHaveClass("review-card--exit");
  });

  it("preserves progress through repeated resume and exit cycles", async () => {
    const onBack = vi.fn();
    const initial = renderWorkspace({ onBack });
    await startReview();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(screen.getByText("1 / 5")).toBeVisible());

    await userEvent.click(screen.getByRole("button", { name: "退出挑片" }));
    expect(await screen.findByText("退出快速挑片？")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "保存进度并退出" }));
    expect(onBack).toHaveBeenCalledTimes(1);
    initial.unmount();

    const second = renderWorkspace();
    expect(await screen.findByText("有一轮未完成的挑片")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "继续挑片" }));
    expect(await screen.findByRole("article", { name: "当前挑片素材" })).toBeVisible();
    expect(screen.getByText("1 / 5")).toBeVisible();

    await userEvent.click(screen.getByRole("button", { name: "退出挑片" }));
    expect(await screen.findByText("退出快速挑片？")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "保存进度并退出" }));
    second.unmount();

    renderWorkspace();
    expect(await screen.findByText("有一轮未完成的挑片")).toBeVisible();
    expect(screen.getByText("已浏览 1 / 5 · 已入选 1 · 待定 0")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "继续挑片" }));
    expect(await screen.findByRole("article", { name: "当前挑片素材" })).toBeVisible();
    expect(screen.getByText("1 / 5")).toBeVisible();
  });

  it("uses the result actions to hand only selected clips to existing batch workflows", async () => {
    const onViewSelected = vi.fn();
    const onFavoriteSelected = vi.fn(async () => true);
    renderWorkspace({ onViewSelected, onFavoriteSelected });
    await startReview();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(screen.getByText("1 / 5")).toBeVisible());
    for (const [index, key] of ["a", "a", "a", "a"].entries()) {
      fireEvent.keyDown(window, { key });
      await waitFor(() => expect(screen.getByText(`${index + 2} / 5`)).toBeVisible());
    }
    await screen.findByRole("heading", { name: "本轮挑片完成" });

    await userEvent.click(screen.getByRole("button", { name: "批量添加标签" }));
    expect(onViewSelected).toHaveBeenCalledWith(
      expect.objectContaining({
        items: expect.arrayContaining([{ videoId: "1", decision: "selected" }]),
      }),
      expect.arrayContaining([expect.objectContaining({ id: "1" })]),
      true,
    );
    await userEvent.click(screen.getByRole("button", { name: "收藏入选素材" }));
    expect(onFavoriteSelected).toHaveBeenCalledWith(["1"]);
  });
});

function renderWorkspace(overrides: Partial<ComponentProps<typeof ReviewWorkspace>> = {}) {
  return render(
    <ReviewWorkspace
      inheritedFilters={inheritedFilters}
      scopeEditor={createScopeEditor()}
      onBack={vi.fn()}
      onFavoriteSelected={vi.fn(async () => true)}
      onOpenOriginal={vi.fn()}
      onRemoveFromIndex={vi.fn(async () => true)}
      onViewSelected={vi.fn()}
      {...overrides}
    />,
  );
}

function createScopeEditor(overrides: Partial<ReviewScopeEditor> = {}): ReviewScopeEditor {
  return {
    query: "",
    accounts: [{
      id: "summer",
      displayName: "Summer#0002",
      sourceName: "",
      clipCount: 1,
      missingCount: 0,
      favoriteCount: 0,
      sizeBytes: 0,
      lastModifiedAt: "",
      detectedBy: "source-dir",
    }],
    accountId: "all",
    agentNames: ["尚勃勒"],
    agentName: "all",
    mapNames: ["隐世修所"],
    mapName: "all",
    gameModes: ["竞技模式"],
    gameMode: "all",
    tags: [{ id: "12", label: "小红书待剪", color: "red" }],
    tagId: "all",
    datePreset: "all",
    highlightFilter: "all",
    videoTypes: ["triple"],
    onQueryChange: vi.fn(),
    onAccountChange: vi.fn(),
    onAgentChange: vi.fn(),
    onMapChange: vi.fn(),
    onGameModeChange: vi.fn(),
    onTagChange: vi.fn(),
    onDatePresetChange: vi.fn(),
    onHighlightFilterChange: vi.fn(),
    onClearFilters: vi.fn(),
    ...overrides,
  };
}

async function startReview() {
  await waitFor(() => expect(screen.getByRole("button", { name: /开始挑片/ })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: /开始挑片/ }));
  await screen.findByRole("article", { name: "当前挑片素材" });
}

function page(items: ClipSummary[]): ClipPage {
  return {
    items,
    offset: 0,
    limit: 200,
    totalCount: items.length,
    hasMore: false,
    nextOffset: null,
  };
}
