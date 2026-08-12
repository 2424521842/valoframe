import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getClipMedia } from "../../src/api/backend";
import { mockClips } from "../../src/data/mockData";
import { PreviewWorkspace } from "../../src/screens/PreviewWorkspace";
import type { Clip, ClipEvent, ClipMedia } from "../../src/types";

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    getClipMedia: vi.fn(),
  };
});

const getClipMediaMock = vi.mocked(getClipMedia);
const clipA = { ...mockClips[0], id: "clip-a", note: "旧备注" };
const clipB = { ...mockClips[1], id: "clip-b", note: "B 备注" };

type PreviewProps = ComponentProps<typeof PreviewWorkspace>;

function createPreviewProps(overrides: Partial<PreviewProps> = {}): PreviewProps {
  return {
    clip: clipA,
    clips: [clipA, clipB],
    tags: [],
    activityMessage: "测试中",
    onBack: vi.fn(),
    onCopyPath: vi.fn(),
    onCreateTag: vi.fn(async () => null),
    onManageTags: vi.fn(),
    onOpenExternal: vi.fn(),
    onOpenOriginal: vi.fn(),
    onSelectClip: vi.fn(),
    onToggleFavorite: vi.fn(),
    onToggleTag: vi.fn(async () => undefined),
    onUpdateNote: vi.fn(async () => undefined),
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function timelineEvent(overrides: Partial<ClipEvent>): ClipEvent {
  return {
    id: "timeline-event",
    eventType: "kill",
    videoTimeMs: 1_000,
    eventTime: null,
    roundId: null,
    playerName: "FixtureAlpha#0001",
    weaponName: "狂徒",
    killerName: "FixtureAlpha#0001",
    killedName: "Opponent#0001",
    killerIsMe: true,
    killedIsMe: false,
    ...overrides,
  };
}

describe("PreviewWorkspace media and note behavior", () => {
  beforeEach(() => {
    getClipMediaMock.mockReset();
  });

  it("keeps the selected clip workspace visible while detail fields are loading", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });

    render(
      <PreviewWorkspace
        {...createPreviewProps()}
        detailStatus="loading"
      />,
    );

    expect(screen.queryByRole("heading", { name: "正在加载素材详情" }))
      .not.toBeInTheDocument();
    expect(screen.getByLabelText("素材预览")).toHaveAttribute("aria-busy", "true");
    expect(screen.getByPlaceholderText("正在加载备注…")).toBeDisabled();
    expect(screen.getByText("SYNCING")).toBeVisible();
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(clipA.id));
  });

  it("keeps decorative video HUD hidden and promotes favorite and tag controls", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    const unstarredClip = { ...clipA, isFavorite: false };

    const { container } = render(
      <PreviewWorkspace
        {...createPreviewProps()}
        clip={unstarredClip}
        clips={[unstarredClip]}
      />,
    );

    expect(screen.queryByText(/ROUND/)).not.toBeInTheDocument();
    expect(screen.queryByText("REC")).not.toBeInTheDocument();
    expect(screen.getByText("素材整理")).toBeVisible();
    expect(screen.getByRole("button", { name: "收藏" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(container.querySelector(".preview-tag-section")).toBeInTheDocument();
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(unstarredClip.id));
  });

  it("supports muting, restoring, and changing the active video volume", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: true,
      mediaUrl: "clip-media://clip-a",
      message: null,
    });

    const { container } = render(<PreviewWorkspace {...createPreviewProps()} />);
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });

    await waitFor(() => {
      expect(video.volume).toBe(1);
      expect(video.muted).toBe(false);
    });

    fireEvent.click(screen.getByRole("button", { name: "静音" }));
    await waitFor(() => expect(video.muted).toBe(true));
    expect(screen.getByRole("button", { name: "恢复声音" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    fireEvent.change(screen.getByRole("slider", { name: "音量" }), {
      target: { value: "0.35" },
    });
    await waitFor(() => {
      expect(video.muted).toBe(false);
      expect(video.volume).toBeCloseTo(0.35);
    });
    expect(screen.getByRole("slider", { name: "音量" })).toHaveAttribute(
      "aria-valuetext",
      "35%",
    );
  });

  it("initializes audio preferences, reports actual changes, and preserves them across clips", async () => {
    getClipMediaMock.mockImplementation(async (clipId) => ({
      clipId,
      playable: true,
      mediaUrl: `clip-media://${clipId}`,
      message: null,
    }));
    const onAudioPreferenceChange = vi.fn();
    const props = createPreviewProps({
      initialVolumePercent: 34.6,
      initialMuted: true,
      onAudioPreferenceChange,
    });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);
    const firstVideo = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });

    await waitFor(() => {
      expect(firstVideo.volume).toBeCloseTo(0.35);
      expect(firstVideo.muted).toBe(true);
    });
    expect(onAudioPreferenceChange).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "恢复声音" }));
    expect(onAudioPreferenceChange).toHaveBeenLastCalledWith({
      volumePercent: 35,
      muted: false,
    });

    fireEvent.change(screen.getByRole("slider", { name: "音量" }), {
      target: { value: "0.42" },
    });
    expect(onAudioPreferenceChange).toHaveBeenLastCalledWith({
      volumePercent: 42,
      muted: false,
    });

    rerender(<PreviewWorkspace {...props} clip={clipB} />);
    const secondVideo = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      expect(element).not.toBe(firstVideo);
      return element!;
    });
    await waitFor(() => {
      expect(secondVideo.volume).toBeCloseTo(0.42);
      expect(secondVideo.muted).toBe(false);
    });
  });

  it("connects playback shortcuts and exposes them on visible controls", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: true,
      mediaUrl: "clip-media://clip-a",
      message: null,
    });

    const { container } = render(<PreviewWorkspace {...createPreviewProps()} />);
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });
    let currentTime = 7;
    Object.defineProperties(video, {
      currentTime: {
        configurable: true,
        get: () => currentTime,
        set: (value: number) => {
          currentTime = value;
        },
      },
      duration: {
        configurable: true,
        get: () => 30,
      },
    });

    fireEvent.keyDown(window, { key: "ArrowRight" });
    expect(currentTime).toBe(12);

    const note = screen.getByPlaceholderText("记录复盘重点或剪辑思路");
    note.focus();
    fireEvent.keyDown(note, { key: "ArrowRight" });
    expect(currentTime).toBe(12);

    expect(screen.getByRole("button", { name: "播放" }))
      .toHaveAttribute("aria-keyshortcuts", "Space K");
    expect(screen.getByRole("button", { name: "播放" }))
      .toHaveAttribute("title", "播放 / 暂停（空格或 K）");
    expect(screen.getByRole("button", { name: "静音" }))
      .toHaveAttribute("aria-keyshortcuts", "M");
    expect(screen.getByRole("button", { name: "调整播放进度" }))
      .toHaveAttribute(
        "aria-keyshortcuts",
        "ArrowLeft ArrowRight Shift+ArrowLeft Shift+ArrowRight J L",
      );
  });

  it("shows only exact self-kill events for kill compilations and seeks precisely", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: "kill-compilation",
      playable: true,
      mediaUrl: "clip-media://kill-compilation",
      message: null,
    });
    const killClip: Clip = {
      ...clipA,
      id: "kill-compilation",
      officialVideoName: "击杀集锦",
      officialVideoType: "击杀集锦",
      highlightType: 2,
      durationMs: 10_000,
      clipEvents: [
        timelineEvent({ id: "zero", eventType: "kill", videoTimeMs: 0 }),
        timelineEvent({ id: "middle", eventType: " KILL ", videoTimeMs: 4_000 }),
        timelineEvent({ id: "end", eventType: "kill", videoTimeMs: 10_000 }),
        timelineEvent({ id: "other", killerIsMe: false, videoTimeMs: 2_000 }),
        timelineEvent({ id: "assist", eventType: "assist", videoTimeMs: 3_000 }),
        timelineEvent({ id: "market", eventType: "market", videoTimeMs: 3_500 }),
        timelineEvent({ id: "blank", eventType: "", videoTimeMs: 4_500 }),
        timelineEvent({ id: "substring", eventType: "killfeed", videoTimeMs: 5_000 }),
        timelineEvent({
          id: "death",
          eventType: "death",
          killedIsMe: true,
          videoTimeMs: 6_000,
        }),
        timelineEvent({ id: "negative", videoTimeMs: -1 }),
        timelineEvent({ id: "too-late", videoTimeMs: 10_001 }),
      ],
    };

    const { container } = render(
      <PreviewWorkspace
        {...createPreviewProps({ clip: killClip, clips: [killClip] })}
      />,
    );
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });
    let currentTime = 0;
    Object.defineProperty(video, "currentTime", {
      configurable: true,
      get: () => currentTime,
      set: (value: number) => {
        currentTime = value;
      },
    });

    const markers = screen.getAllByRole("button", { name: /本人击杀/ });
    expect(markers).toHaveLength(3);
    const middle = screen.getByRole("button", { name: "本人击杀 · 00:04" });
    expect(middle).toHaveAttribute("title", "本人击杀 · 00:04");
    expect(middle).toHaveClass("preview-timeline-flag--kill");
    expect(middle).toHaveStyle({ left: "40%" });
    expect(middle.querySelector(".preview-timeline-icon--kill")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /本人死亡 ·/ })).not.toBeInTheDocument();

    fireEvent.click(middle);
    expect(currentTime).toBe(4);
    expect(screen.getByLabelText("时间轴标记图例")).toHaveTextContent("本人击杀本人死亡");

    Object.defineProperty(video, "duration", {
      configurable: true,
      get: () => 5,
    });
    fireEvent.durationChange(video);
    await waitFor(() => {
      expect(screen.getAllByRole("button", { name: /本人击杀/ })).toHaveLength(2);
      expect(screen.queryByRole("button", { name: "本人击杀 · 00:10" }))
        .not.toBeInTheDocument();
    });
  });

  it("shows only exact self-death events with skull markers for death compilations", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: "death-compilation",
      playable: true,
      mediaUrl: "clip-media://death-compilation",
      message: null,
    });
    const deathClip: Clip = {
      ...clipA,
      id: "death-compilation",
      officialVideoName: "死亡集锦",
      officialVideoType: "death compilation",
      highlightType: 3,
      durationMs: 10_000,
      clipEvents: [
        timelineEvent({
          id: "death-middle",
          eventType: "DEATH",
          killedIsMe: true,
          killerIsMe: false,
          videoTimeMs: 2_000,
        }),
        timelineEvent({
          id: "death-end",
          eventType: " death ",
          killedIsMe: true,
          killerIsMe: false,
          videoTimeMs: 10_000,
        }),
        timelineEvent({
          id: "other-death",
          eventType: "death",
          killedIsMe: false,
          videoTimeMs: 3_000,
        }),
        timelineEvent({ id: "self-kill", eventType: "kill", videoTimeMs: 4_000 }),
        timelineEvent({
          id: "substring",
          eventType: "death recap",
          killedIsMe: true,
          videoTimeMs: 5_000,
        }),
      ],
    };

    const { container } = render(
      <PreviewWorkspace
        {...createPreviewProps({ clip: deathClip, clips: [deathClip] })}
      />,
    );
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });
    let currentTime = 0;
    Object.defineProperty(video, "currentTime", {
      configurable: true,
      get: () => currentTime,
      set: (value: number) => {
        currentTime = value;
      },
    });

    const markers = screen.getAllByRole("button", { name: /本人死亡/ });
    expect(markers).toHaveLength(2);
    const middle = screen.getByRole("button", { name: "本人死亡 · 00:02" });
    expect(middle).toHaveAttribute("title", "本人死亡 · 00:02");
    expect(middle).toHaveClass("preview-timeline-flag--death");
    expect(middle.querySelector(".preview-timeline-icon--death")).toBeInTheDocument();
    middle.focus();
    expect(middle).toHaveFocus();
    fireEvent.click(middle);
    expect(currentTime).toBe(2);
    expect(container.querySelectorAll(".preview-timeline-flag--kill")).toHaveLength(0);
    expect(screen.queryByRole("button", { name: /本人击杀 ·/ })).not.toBeInTheDocument();
  });

  it("shows only exact self-kill markers for ordinary multi-kills and hides unknown clips", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: "ordinary",
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    const ordinaryClip: Clip = {
      ...clipA,
      id: "ordinary",
      officialVideoName: "三杀时刻",
      officialVideoType: "高光时刻",
      highlightType: 4,
      durationMs: 10_000,
      clipEvents: [
        timelineEvent({ id: "ordinary-kill", videoTimeMs: 2_000 }),
        timelineEvent({ id: "other-kill", killerIsMe: false, videoTimeMs: 3_000 }),
        timelineEvent({ id: "assist", eventType: "assist", videoTimeMs: 4_000 }),
        timelineEvent({
          id: "self-death",
          eventType: "death",
          killerIsMe: false,
          killedIsMe: true,
          videoTimeMs: 5_000,
        }),
      ],
    };
    const props = createPreviewProps({ clip: ordinaryClip, clips: [ordinaryClip] });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(ordinaryClip.id));
    expect(screen.getAllByRole("button", { name: /本人击杀 ·/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "本人击杀 · 00:02" })).toHaveClass(
      "preview-timeline-flag--kill",
    );
    expect(screen.queryByRole("button", { name: /本人死亡 ·/ })).not.toBeInTheDocument();

    const unknownClip: Clip = {
      ...ordinaryClip,
      id: "unknown",
      officialVideoName: "普通素材",
      officialVideoType: "",
      highlightType: null,
      killCount: null,
      extractedText: "",
    };
    getClipMediaMock.mockResolvedValueOnce({
      clipId: unknownClip.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    rerender(
      <PreviewWorkspace
        {...props}
        clip={unknownClip}
        clips={[unknownClip]}
      />,
    );
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(unknownClip.id));
    expect(container.querySelectorAll(".preview-timeline-flag")).toHaveLength(0);

    const zeroDurationClip: Clip = {
      ...unknownClip,
      id: "zero-duration",
      officialVideoName: "击杀集锦",
      officialVideoType: "击杀集锦",
      highlightType: 2,
      durationMs: 0,
      clipEvents: [
        timelineEvent({ id: "at-zero", videoTimeMs: 0 }),
        timelineEvent({ id: "after-zero", videoTimeMs: 1 }),
      ],
    };
    getClipMediaMock.mockResolvedValueOnce({
      clipId: zeroDurationClip.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    rerender(
      <PreviewWorkspace
        {...props}
        clip={zeroDurationClip}
        clips={[zeroDurationClip]}
      />,
    );
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(zeroDurationClip.id));
    expect(screen.getAllByRole("button", { name: /本人击杀/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "本人击杀 · 00:00" })).toBeVisible();
  });

  it("recognizes ordinary multi-kills from numeric type, kill count, and Chinese name", async () => {
    getClipMediaMock.mockImplementation(async (clipId) => ({
      clipId,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    }));
    const variants: Clip[] = [
      ...[4, 6, 10].map((highlightType) => ({
        ...clipA,
        id: `numeric-${highlightType}`,
        officialVideoName: "普通高光",
        officialVideoType: "高光时刻",
        highlightType,
        killCount: null,
        extractedText: "",
      })),
      ...[3, 4, 5, 6].map((killCount) => ({
        ...clipA,
        id: `count-${killCount}`,
        officialVideoName: "普通高光",
        officialVideoType: "高光时刻",
        highlightType: null,
        killCount,
        extractedText: "",
      })),
      ...["三杀时刻", "四杀时刻", "五杀时刻", "六杀时刻"].map(
        (officialVideoName, index) => ({
          ...clipA,
          id: `name-${index}`,
          officialVideoName,
          officialVideoType: "高光时刻",
          highlightType: null,
          killCount: null,
          extractedText: "",
        }),
      ),
    ].map((clip) => ({
      ...clip,
      durationMs: 10_000,
      clipEvents: [timelineEvent({ id: `event-${clip.id}`, videoTimeMs: 2_000 })],
    }));

    const props = createPreviewProps({ clip: variants[0], clips: [variants[0]] });
    const { rerender } = render(<PreviewWorkspace {...props} />);
    for (const variant of variants) {
      rerender(<PreviewWorkspace {...props} clip={variant} clips={[variant]} />);
      await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(variant.id));
      expect(screen.getByRole("button", { name: "本人击杀 · 00:02" })).toBeVisible();
    }

    expect(variants).toHaveLength(11);
  });

  it("returns from preview with Escape without overriding open controls or layers", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    const onBack = vi.fn();
    render(<PreviewWorkspace {...createPreviewProps({ onBack })} />);
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(clipA.id));

    const backButton = screen.getByRole("button", { name: "返回素材库" });
    expect(backButton).toHaveAttribute("aria-keyshortcuts", "Escape");
    expect(backButton).toHaveAttribute("title", "返回素材库（Esc）");

    const note = screen.getByPlaceholderText("记录复盘重点或剪辑思路");
    note.focus();
    fireEvent.keyDown(note, { key: "Escape" });
    expect(onBack).toHaveBeenCalledTimes(1);

    const select = screen.getByRole("combobox", { name: "选择已有标签" });
    select.focus();
    fireEvent.keyDown(select, { key: "Escape" });
    expect(onBack).toHaveBeenCalledTimes(2);
    select.blur();

    const openCombobox = document.createElement("button");
    openCombobox.setAttribute("aria-expanded", "true");
    openCombobox.setAttribute("role", "combobox");
    document.body.append(openCombobox);
    openCombobox.focus();
    fireEvent.keyDown(openCombobox, { key: "Escape" });
    openCombobox.remove();
    expect(onBack).toHaveBeenCalledTimes(2);

    const modal = document.createElement("div");
    modal.className = "ui-dialog-content";
    document.body.append(modal);
    fireEvent.keyDown(window, { key: "Escape" });
    modal.remove();
    expect(onBack).toHaveBeenCalledTimes(2);

    fireEvent.keyDown(window, { key: "Escape", ctrlKey: true });
    expect(onBack).toHaveBeenCalledTimes(2);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onBack).toHaveBeenCalledTimes(3);
  });

  it("offers the system player when the embedded decoder rejects an indexed video", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: true,
      mediaUrl: "clip-media://clip-a",
      message: null,
    });
    const onOpenExternal = vi.fn();
    const { container } = render(
      <PreviewWorkspace {...createPreviewProps({ onOpenExternal })} />,
    );
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });

    fireEvent.error(video);
    expect(screen.getByRole("alert")).toHaveTextContent(/WebView2 解码链无法播放/);
    fireEvent.click(screen.getByRole("button", { name: /使用系统默认播放器打开/ }));
    expect(onOpenExternal).toHaveBeenCalledWith(clipA.id);
  });

  it("syncs a parent note update without requesting clip media again", async () => {
    getClipMediaMock.mockResolvedValue({
      clipId: clipA.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    const props = createPreviewProps();
    const { rerender } = render(<PreviewWorkspace {...props} />);

    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledTimes(1));
    const updatedClip = { ...clipA, note: "父级已保存的新备注" };
    rerender(
      <PreviewWorkspace
        {...props}
        clip={updatedClip}
        clips={[updatedClip, clipB]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByPlaceholderText("记录复盘重点或剪辑思路")).toHaveValue(
        "父级已保存的新备注",
      );
    });
    expect(getClipMediaMock).toHaveBeenCalledTimes(1);
  });

  it("ignores an older media response after switching clips quickly", async () => {
    const mediaA = deferred<ClipMedia>();
    const mediaB = deferred<ClipMedia>();
    getClipMediaMock.mockImplementation((clipId) =>
      clipId === clipA.id ? mediaA.promise : mediaB.promise,
    );
    const props = createPreviewProps();
    const { container, rerender } = render(<PreviewWorkspace {...props} />);

    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(clipA.id));
    rerender(<PreviewWorkspace {...props} clip={clipB} />);
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledWith(clipB.id));

    await act(async () => {
      mediaB.resolve({
        clipId: clipB.id,
        playable: false,
        mediaUrl: null,
        message: "B 媒体状态",
      });
      await mediaB.promise;
    });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "播放视频" })).toHaveAttribute(
        "title",
        "B 媒体状态",
      );
    });

    await act(async () => {
      mediaA.resolve({
        clipId: clipA.id,
        playable: true,
        mediaUrl: "clip-media://old-a",
        message: null,
      });
      await mediaA.promise;
    });

    expect(screen.getByRole("button", { name: "播放视频" })).toHaveAttribute(
      "title",
      "B 媒体状态",
    );
    expect(container.querySelector("video")).not.toBeInTheDocument();
  });

  it("does not apply an old note-save result to the newly selected clip", async () => {
    getClipMediaMock.mockImplementation(async (clipId) => ({
      clipId,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    }));
    const saveA = deferred<void>();
    const onUpdateNote = vi.fn(() => saveA.promise);
    const props = createPreviewProps({ onUpdateNote });
    const { rerender } = render(<PreviewWorkspace {...props} />);
    const note = screen.getByPlaceholderText("记录复盘重点或剪辑思路");

    fireEvent.change(note, { target: { value: "A 的待保存备注" } });
    fireEvent.click(screen.getByRole("button", { name: "保存备注" }));
    await waitFor(() => expect(onUpdateNote).toHaveBeenCalledWith(clipA.id, "A 的待保存备注"));

    rerender(<PreviewWorkspace {...props} clip={clipB} />);
    await waitFor(() => {
      expect(screen.getByPlaceholderText("记录复盘重点或剪辑思路")).toHaveValue("B 备注");
      expect(screen.getByRole("button", { name: "保存备注" })).toBeDisabled();
    });

    await act(async () => {
      saveA.resolve(undefined);
      await saveA.promise;
    });

    expect(screen.queryByText("已保存")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("记录复盘重点或剪辑思路")).toHaveValue("B 备注");
  });

  it("keeps the match rail order fixed while the active frame follows the selected clip", async () => {
    getClipMediaMock.mockImplementation(async (clipId) => ({
      clipId,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    }));
    const common = {
      accountId: "fixed-account",
      matchId: "fixed-match",
      clipGroupId: "fixed-group",
    };
    const first = {
      ...clipA,
      ...common,
      id: "fixed-first",
      officialVideoName: "固定顺序一",
      modifiedAt: "2026-07-16T10:00:00+08:00",
    };
    const second = {
      ...clipB,
      ...common,
      id: "fixed-second",
      officialVideoName: "固定顺序二",
      modifiedAt: "2026-07-16T10:01:00+08:00",
    };
    const third = {
      ...clipA,
      ...common,
      id: "fixed-third",
      officialVideoName: "固定顺序三",
      modifiedAt: "2026-07-16T10:02:00+08:00",
    };
    const clips = [third, first, second];
    const onSelectClip = vi.fn();
    const props = createPreviewProps({ clip: first, clips, onSelectClip });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);

    expect(railTitles(container)).toEqual(["固定顺序一", "固定顺序二", "固定顺序三"]);
    expect(activeRailTitle(container)).toBe("固定顺序一");
    expect(screen.getByText("3 条片段 · 数字键 1–3 切换")).toBeVisible();

    const railButtons = [...container.querySelectorAll<HTMLButtonElement>(".preview-rail-clip")];
    expect(railButtons[0]).toHaveAttribute("aria-keyshortcuts", "1");
    expect(railButtons[1]).toHaveAttribute("aria-keyshortcuts", "2");
    expect(railButtons[2]).toHaveAttribute("title", "选择第 3 条片段（数字键 3）");

    fireEvent.keyDown(window, { key: "2" });
    expect(onSelectClip).toHaveBeenLastCalledWith(second.id);

    const note = screen.getByPlaceholderText("记录复盘重点或剪辑思路");
    note.focus();
    fireEvent.keyDown(note, { key: "3" });
    expect(onSelectClip).toHaveBeenCalledTimes(1);

    note.blur();
    fireEvent.keyDown(window, { key: "3", ctrlKey: true });
    fireEvent.keyDown(window, { key: "4" });
    expect(onSelectClip).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(window, { key: "3" });
    expect(onSelectClip).toHaveBeenLastCalledWith(third.id);

    rerender(<PreviewWorkspace {...props} clip={third} />);

    expect(railTitles(container)).toEqual(["固定顺序一", "固定顺序二", "固定顺序三"]);
    expect(activeRailTitle(container)).toBe("固定顺序三");
  });

  it("falls back after an artwork error and retries a new revision", async () => {
    const first = {
      ...clipA,
      thumbnailUrl: "clip-media://cover/42?v=rev-1",
      thumbnailRevision: "rev-1",
    };
    getClipMediaMock.mockResolvedValue({
      clipId: first.id,
      playable: false,
      mediaUrl: null,
      message: "不可播放",
    });
    const props = createPreviewProps({ clip: first, clips: [first] });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);
    await waitFor(() => expect(getClipMediaMock).toHaveBeenCalledTimes(1));

    const heroImage = container.querySelector<HTMLImageElement>(".cinematic-artwork--hero img");
    expect(heroImage).toHaveAttribute("src", first.thumbnailUrl);
    fireEvent.error(heroImage!);
    expect(container.querySelector(".cinematic-artwork--hero img")).not.toBeInTheDocument();
    expect(container.querySelector(".cinematic-artwork--hero .cinematic-artwork-fallback"))
      .toBeInTheDocument();

    const updated = {
      ...first,
      thumbnailUrl: "clip-media://cover/42?v=rev-2",
      thumbnailRevision: "rev-2",
    };
    rerender(<PreviewWorkspace {...props} clip={updated} clips={[updated]} />);
    expect(container.querySelector(".cinematic-artwork--hero .cinematic-artwork-fallback"))
      .not.toBeInTheDocument();
    expect(container.querySelector(".cinematic-artwork--hero img"))
      .toHaveAttribute("src", updated.thumbnailUrl);
    expect(getClipMediaMock).toHaveBeenCalledTimes(1);
  });

  it("updates the video poster without requesting or remounting media", async () => {
    const first = {
      ...clipA,
      thumbnailUrl: "clip-media://cover/42?v=rev-1",
      thumbnailRevision: "rev-1",
    };
    getClipMediaMock.mockResolvedValue({
      clipId: first.id,
      playable: true,
      mediaUrl: "clip-media://clip/42",
      message: null,
    });
    const props = createPreviewProps({ clip: first, clips: [first] });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);
    const video = await waitFor(() => {
      const element = container.querySelector<HTMLVideoElement>("video");
      expect(element).toBeInTheDocument();
      return element!;
    });
    expect(video).toHaveAttribute("poster", first.thumbnailUrl);
    video.currentTime = 7;

    const updated = {
      ...first,
      thumbnailUrl: "clip-media://cover/42?v=rev-2",
      thumbnailRevision: "rev-2",
    };
    rerender(<PreviewWorkspace {...props} clip={updated} clips={[updated]} />);

    const updatedVideo = container.querySelector<HTMLVideoElement>("video");
    expect(updatedVideo).toBe(video);
    expect(updatedVideo).toHaveAttribute("poster", updated.thumbnailUrl);
    expect(updatedVideo?.currentTime).toBe(7);
    expect(getClipMediaMock).toHaveBeenCalledTimes(1);
  });
});

function railTitles(container: HTMLElement): string[] {
  return [...container.querySelectorAll<HTMLElement>(".preview-rail-clip strong")]
    .map((element) => element.textContent ?? "");
}

function activeRailTitle(container: HTMLElement): string | null {
  return container.querySelector<HTMLElement>(".preview-rail-clip--active strong")
    ?.textContent ?? null;
}
