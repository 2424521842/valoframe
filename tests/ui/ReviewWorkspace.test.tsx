import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import { REVIEW_SESSION_STORAGE_KEY } from "../../src/lib/reviewSessions";
import type { ReviewScopeOptions } from "../../src/components/review/ReviewSetup";
import { ReviewWorkspace } from "../../src/screens/ReviewWorkspace";
import type { ClipListQuery, ClipPage, ClipSummary } from "../../src/types";

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

  it("starts from default local conditions and updates them without leaving 快速挑片", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    renderWorkspace({ onBack });

    expect(await screen.findByRole("heading", { name: "快速挑片" })).toBeVisible();
    expect(screen.getByText("全部可用素材")).toBeVisible();
    expect(screen.queryByRole("combobox", { name: "账号" })).not.toBeInTheDocument();
    await waitFor(() => expect(latestReviewQuery()).toEqual({
      sortBy: "modified-desc",
      offset: 0,
      limit: 200,
    }));

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
    expect(screen.getByLabelText("搜索素材")).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "账号" })).toHaveTextContent("全部账号");
    expect(screen.getByRole("combobox", { name: "英雄" })).toHaveTextContent("全部英雄");

    await user.click(screen.getByRole("combobox", { name: "账号" }));
    await user.click(await screen.findByRole("option", { name: "Summer#0002" }));
    fireEvent.change(screen.getByLabelText("搜索素材"), { target: { value: "ace" } });
    expect(onBack).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByText("5 条素材")).toBeVisible());
    await waitFor(() => expect(latestReviewQuery()).toEqual(expect.objectContaining({
      accountId: "summer",
      query: "ace",
      sortBy: "modified-desc",
    })));
  });

  it("shows and resets the local condition in every inline filter trigger", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await user.click(await screen.findByRole("button", { name: "修改范围" }));
    await chooseReviewScopeOption(user, "账号", "Summer#0002");
    await chooseReviewScopeOption(user, "英雄", "尚勃勒");
    await chooseReviewScopeOption(user, "地图", "隐世修所");
    await chooseReviewScopeOption(user, "模式", "竞技模式");
    await chooseReviewScopeOption(user, "日期", "近 30 天");
    await chooseReviewScopeOption(user, "视频类型", "三杀时刻");
    await chooseReviewScopeOption(user, "自定义标签", "小红书待剪");
    await user.type(screen.getByLabelText("搜索素材"), "ace");
    expect(screen.getByLabelText("搜索素材")).toHaveValue("ace");
    expect(screen.getByRole("combobox", { name: "账号" })).toHaveTextContent("Summer#0002");
    expect(screen.getByRole("combobox", { name: "英雄" })).toHaveTextContent("尚勃勒");
    expect(screen.getByRole("combobox", { name: "地图" })).toHaveTextContent("隐世修所");
    expect(screen.getByRole("combobox", { name: "模式" })).toHaveTextContent("竞技模式");
    expect(screen.getByRole("combobox", { name: "日期" })).toHaveTextContent("近 30 天");
    expect(screen.getByRole("combobox", { name: "视频类型" })).toHaveTextContent("三杀时刻");
    expect(screen.getByRole("combobox", { name: "自定义标签" })).toHaveTextContent("小红书待剪");

    await user.click(screen.getByRole("button", { name: "重置条件" }));
    expect(screen.getByLabelText("搜索素材")).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "账号" })).toHaveTextContent("全部账号");
    expect(screen.getByRole("combobox", { name: "英雄" })).toHaveTextContent("全部英雄");
    expect(screen.getByRole("combobox", { name: "地图" })).toHaveTextContent("全部地图");
    expect(screen.getByRole("combobox", { name: "模式" })).toHaveTextContent("全部模式");
    expect(screen.getByRole("combobox", { name: "日期" })).toHaveTextContent("全部日期");
    expect(screen.getByRole("combobox", { name: "视频类型" })).toHaveTextContent("全部类型");
    expect(screen.getByRole("combobox", { name: "自定义标签" })).toHaveTextContent("全部自定义标签");
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
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledWith("5"));
    expect(mocks.getClipMedia).toHaveBeenCalledTimes(1);

    const input = document.createElement("input");
    document.body.append(input);
    input.focus();
    fireEvent.keyDown(window, { key: "d" });
    expect(screen.getByText("0 / 5")).toBeVisible();
    input.remove();

    const selectAction = screen.getByRole("button", { name: /^入选D \/ →$/ });
    await userEvent.click(selectAction);
    await waitFor(() => expect(mocks.getClipMedia).toHaveBeenCalledWith("4"));
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
      expect(video?.getAttribute("src")).toBe("https://media.test/5.mp4");
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
        .toBe("https://media.test/4.mp4");
    });
    expect(screen.getByRole("article", { name: "当前挑片素材" })).not.toHaveClass("review-card--exit");
  });

  it("keeps quick-pick playback controls usable in fullscreen without triggering a swipe decision", async () => {
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
    mocks.getClipMedia.mockImplementation(async (clipId: string) => ({
      clipId,
      playable: true,
      mediaUrl: `https://media.test/${clipId}.mp4`,
      message: null,
    }));
    const onBack = vi.fn();
    renderWorkspace({ autoplay: false, onBack });
    await startReview();
    await waitFor(() => expect(document.querySelector(".review-card-media video")).toBeInTheDocument());

    const mediaShell = document.querySelector<HTMLElement>(".review-card-media")!;
    const reviewCard = screen.getByRole("article", { name: "当前挑片素材" });
    let fullscreenElement: Element | null = null;
    let now = 2_000;
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => now);
    const requestFullscreen = vi.fn().mockResolvedValue(undefined);
    const exitFullscreen = vi.fn(async () => {
      fullscreenElement = null;
    });
    Object.defineProperty(mediaShell, "requestFullscreen", {
      configurable: true,
      value: requestFullscreen,
    });
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      get: () => fullscreenElement,
    });
    Object.defineProperty(document, "exitFullscreen", {
      configurable: true,
      value: exitFullscreen,
    });

    try {
      const enterButton = screen.getByRole("button", { name: "进入全屏" });
      expect(enterButton).toHaveAttribute("aria-keyshortcuts", "F");
      fireEvent.click(enterButton);
      await waitFor(() => expect(requestFullscreen).toHaveBeenCalledTimes(1));
      fullscreenElement = mediaShell;
      fireEvent(document, new Event("fullscreenchange"));
      expect(await screen.findByRole("button", { name: "退出全屏" })).toBeVisible();

      fireEvent.pointerDown(reviewCard, { pointerId: 1, clientX: 0, clientY: 0 });
      fireEvent.pointerMove(reviewCard, { pointerId: 1, clientX: 160, clientY: 0 });
      fireEvent.pointerUp(reviewCard, { pointerId: 1, clientX: 160, clientY: 0 });
      expect(screen.getByText("0 / 5")).toBeVisible();

      fireEvent.click(screen.getByRole("button", { name: "退出全屏" }));
      await waitFor(() => expect(exitFullscreen).toHaveBeenCalledTimes(1));
      fireEvent(document, new Event("fullscreenchange"));

      const focusedEnterButton = screen.getByRole("button", { name: "进入全屏" });
      focusedEnterButton.focus();
      fireEvent.keyDown(focusedEnterButton, { key: "f" });
      await waitFor(() => expect(requestFullscreen).toHaveBeenCalledTimes(2));
      fullscreenElement = mediaShell;
      fireEvent(document, new Event("fullscreenchange"));

      fullscreenElement = null;
      fireEvent(document, new Event("fullscreenchange"));
      fireEvent.keyDown(window, { key: "Escape" });
      expect(onBack).not.toHaveBeenCalled();

      now += 400;
      fireEvent.keyDown(window, { key: "Escape" });
      expect(onBack).toHaveBeenCalledTimes(1);
    } finally {
      nowSpy.mockRestore();
      delete (mediaShell as Partial<HTMLElement>).requestFullscreen;
      delete (document as Partial<Document>).exitFullscreen;
      delete (document as Partial<Document>).fullscreenElement;
    }
  });

  it("preserves progress through repeated resume and exit cycles", async () => {
    const onBack = vi.fn();
    const initial = renderWorkspace({ onBack });
    await startReview();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(screen.getByText("1 / 5")).toBeVisible());

    await userEvent.click(screen.getByRole("button", { name: "退出挑片" }));
    expect(await screen.findByText("退出或结束本轮挑片？")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "保存进度并退出" }));
    expect(onBack).toHaveBeenCalledTimes(1);
    initial.unmount();

    const second = renderWorkspace();
    expect(await screen.findByText("有一轮未完成的挑片")).toBeVisible();
    expect(screen.getByRole("button", { name: "开始新的挑片" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "继续上次挑片" }));
    expect(await screen.findByRole("article", { name: "当前挑片素材" })).toBeVisible();
    expect(screen.getByText("1 / 5")).toBeVisible();

    await userEvent.click(screen.getByRole("button", { name: "退出挑片" }));
    expect(await screen.findByText("退出或结束本轮挑片？")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "保存进度并退出" }));
    second.unmount();

    renderWorkspace();
    expect(await screen.findByText("有一轮未完成的挑片")).toBeVisible();
    expect(screen.getByText("已浏览 1 / 5 · 已入选 1 · 待定 0")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "继续上次挑片" }));
    expect(await screen.findByRole("article", { name: "当前挑片素材" })).toBeVisible();
    expect(screen.getByText("1 / 5")).toBeVisible();
  });

  it("ends a partially reviewed round early without turning remaining clips into skipped choices", async () => {
    const onBack = vi.fn();
    const onViewSelected = vi.fn();
    const workspace = renderWorkspace({ onBack, onViewSelected });
    await startReview();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(screen.getByText("1 / 5")).toBeVisible());

    await userEvent.click(screen.getByRole("button", { name: "退出挑片" }));
    expect(await screen.findByText("退出或结束本轮挑片？")).toBeVisible();
    expect(screen.getByText(/剩余 4 条素材保持未处理/)).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "提前结束并查看结果" }));

    expect(await screen.findByRole("heading", { name: "本轮挑片已提前结束" })).toBeVisible();
    expect(screen.getByText("已浏览 1 / 5 条素材，剩余 4 条未处理。已做出的挑片决定会保留。")).toBeVisible();
    expect(onBack).not.toHaveBeenCalled();
    expect(mocks.setClipReviewDecision).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "查看入选素材 (1)" }));
    expect(onViewSelected).toHaveBeenCalledWith(
      expect.objectContaining({
        status: "completed",
        items: expect.arrayContaining([{ videoId: "5", decision: "selected" }]),
      }),
      expect.arrayContaining([expect.objectContaining({ id: "5" })]),
      false,
    );

    workspace.unmount();
    renderWorkspace();
    await screen.findByRole("heading", { name: "快速挑片" });
    expect(screen.queryByText("有一轮未完成的挑片")).not.toBeInTheDocument();
  });

  it("clears every condition from the current-scope panel", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await user.click(await screen.findByRole("button", { name: "修改范围" }));
    await chooseReviewScopeOption(user, "账号", "Summer#0002");
    await waitFor(() => expect(screen.getByText("账号：Summer#0002")).toBeVisible());

    await user.click(screen.getByRole("button", { name: /清空全部条件/ }));

    expect(screen.getByText("全部可用素材")).toBeVisible();
    expect(screen.queryByText("账号：Summer#0002")).not.toBeInTheDocument();
    await waitFor(() => expect(latestReviewQuery()).toEqual({
      sortBy: "modified-desc",
      offset: 0,
      limit: 200,
    }));
  });

  it("offers finishing early directly in the session once anything has been decided", async () => {
    const onViewSelected = vi.fn();
    renderWorkspace({ onViewSelected });
    await startReview();

    const session = screen.getByLabelText("快速挑片会话");
    // Nothing decided yet: finishing early is not an outcome worth offering.
    expect(within(session).queryByRole("button", { name: "提前结束并查看结果" })).not.toBeInTheDocument();

    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(screen.getByText("1 / 5")).toBeVisible());

    // Reachable without opening the exit dialog first.
    const finish = within(screen.getByLabelText("快速挑片会话"))
      .getByRole("button", { name: "提前结束并查看结果" });
    expect(finish).toBeVisible();
    expect(screen.getByText(/剩余 4 条保持未处理/)).toBeVisible();

    await userEvent.click(finish);

    expect(await screen.findByRole("heading", { name: "本轮挑片已提前结束" })).toBeVisible();
    expect(mocks.setClipReviewDecision).not.toHaveBeenCalled();
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
        items: expect.arrayContaining([{ videoId: "5", decision: "selected" }]),
      }),
      expect.arrayContaining([expect.objectContaining({ id: "5" })]),
      true,
    );
    await userEvent.click(screen.getByRole("button", { name: "收藏入选素材" }));
    expect(onFavoriteSelected).toHaveBeenCalledWith(["5"]);
  });
});

function renderWorkspace(overrides: Partial<ComponentProps<typeof ReviewWorkspace>> = {}) {
  return render(
    <ReviewWorkspace
      scopeOptions={createScopeOptions()}
      onBack={vi.fn()}
      onFavoriteSelected={vi.fn(async () => true)}
      onOpenOriginal={vi.fn()}
      onRemoveFromIndex={vi.fn(async () => true)}
      onViewSelected={vi.fn()}
      {...overrides}
    />,
  );
}

function createScopeOptions(overrides: Partial<ReviewScopeOptions> = {}): ReviewScopeOptions {
  return {
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
    agentNames: ["尚勃勒"],
    mapNames: ["隐世修所"],
    gameModes: ["竞技模式"],
    tags: [{ id: "12", label: "小红书待剪", color: "red" }],
    videoTypes: ["triple"],
    ...overrides,
  };
}

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

async function startReview() {
  await waitFor(() => expect(screen.getByRole("button", { name: /开始.*挑片/ })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: /开始.*挑片/ }));
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
