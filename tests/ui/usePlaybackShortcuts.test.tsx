import { act, render, screen } from "@testing-library/react";
import { useRef, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  usePlaybackShortcuts,
  type PlaybackShortcutFocusGuard,
} from "../../src/hooks/usePlaybackShortcuts";

type HarnessProps = {
  active?: boolean;
  children?: ReactNode;
  isFocusProtected?: PlaybackShortcutFocusGuard;
  enableArrowSeek?: boolean;
};

function ShortcutHarness({
  active = true,
  children,
  isFocusProtected,
  enableArrowSeek,
}: HarnessProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  usePlaybackShortcuts({ videoRef, active, isFocusProtected, enableArrowSeek });
  return (
    <div>
      <video aria-label="测试视频" ref={videoRef} />
      {children}
    </div>
  );
}

describe("usePlaybackShortcuts", () => {
  it("maps seek keys to the correct offsets and clamps at both ends", () => {
    render(<ShortcutHarness />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: 30, currentTime: 8 });

    expect(pressKey("ArrowLeft").defaultPrevented).toBe(true);
    expect(media.currentTime).toBe(3);

    media.currentTime = 8;
    expect(pressKey("ArrowRight").defaultPrevented).toBe(true);
    expect(media.currentTime).toBe(13);

    media.currentTime = 8;
    pressKey("ArrowLeft", { shiftKey: true });
    expect(media.currentTime).toBe(0);

    media.currentTime = 24;
    pressKey("ArrowRight", { shiftKey: true });
    expect(media.currentTime).toBe(30);

    media.currentTime = 8;
    expect(pressKey("j").defaultPrevented).toBe(false);
    expect(media.currentTime).toBe(0);

    media.currentTime = 24;
    expect(pressKey("L").defaultPrevented).toBe(false);
    expect(media.currentTime).toBe(30);
  });

  it("does not seek or throw before a finite duration is available", () => {
    render(<ShortcutHarness />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: Number.NaN, currentTime: 4 });

    expect(() => pressKey("ArrowRight")).not.toThrow();
    expect(media.currentTime).toBe(4);
  });

  it("leaves arrow keys untouched when a review card reserves them for decisions", () => {
    render(<ShortcutHarness enableArrowSeek={false} />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: 30, currentTime: 8 });

    expect(pressKey("ArrowLeft").defaultPrevented).toBe(false);
    expect(pressKey("ArrowRight").defaultPrevented).toBe(false);
    expect(media.currentTime).toBe(8);
    pressKey("L");
    expect(media.currentTime).toBe(18);
  });

  it("toggles playback and ignores repeats for toggle actions", () => {
    render(<ShortcutHarness />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video);

    const kEvent = pressKey("K");
    expect(kEvent.defaultPrevented).toBe(false);
    expect(media.play).toHaveBeenCalledTimes(1);

    const spaceEvent = pressKey(" ");
    expect(spaceEvent.defaultPrevented).toBe(true);
    expect(media.pause).toHaveBeenCalledTimes(1);

    media.paused = true;
    pressKey(" ", { repeat: true });
    expect(media.play).toHaveBeenCalledTimes(1);
  });

  it("mutes and restores the last audible volume", () => {
    render(<ShortcutHarness />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    configureMedia(video);
    video.volume = 0.35;
    video.muted = false;

    pressKey("m");
    expect(video.muted).toBe(true);
    expect(video.volume).toBeCloseTo(0.35);

    video.volume = 0;
    pressKey("M");
    expect(video.muted).toBe(false);
    expect(video.volume).toBeCloseTo(0.35);
  });

  it("protects editable controls, command palettes, and dialogs", () => {
    render(
      <ShortcutHarness>
        <input aria-label="输入框" />
        <textarea aria-label="备注" />
        <select aria-label="选择框"><option>选项</option></select>
        <button aria-label="按钮" type="button">按钮</button>
        <div aria-label="可编辑区域" contentEditable suppressContentEditableWarning tabIndex={0}>内容</div>
        <div aria-label="组合框" role="combobox" tabIndex={0} />
        <div className="ui-command"><div aria-label="命令面板项目" tabIndex={0} /></div>
        <div role="dialog"><div aria-label="对话框内容" tabIndex={0} /></div>
      </ShortcutHarness>,
    );
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: 30, currentTime: 10 });
    const protectedNames = [
      "输入框",
      "备注",
      "选择框",
      "按钮",
      "可编辑区域",
      "组合框",
      "命令面板项目",
      "对话框内容",
    ];

    for (const name of protectedNames) {
      const element = screen.getByLabelText(name);
      element.focus();
      const event = pressKey("ArrowRight", {}, element);
      expect(event.defaultPrevented).toBe(false);
      expect(media.currentTime).toBe(10);
    }
  });

  it("blocks shortcuts while a modal is open even if focus has not settled", () => {
    render(
      <ShortcutHarness>
        <div className="ui-dialog-content" />
      </ShortcutHarness>,
    );
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: 30, currentTime: 10 });

    pressKey("ArrowRight");
    expect(media.currentTime).toBe(10);
  });

  it("leaves inactive, modified, preempted, and unrelated keys untouched", () => {
    const { rerender } = render(<ShortcutHarness active={false} />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video, { duration: 30, currentTime: 10 });

    expect(pressKey("ArrowRight").defaultPrevented).toBe(false);
    expect(media.currentTime).toBe(10);

    rerender(<ShortcutHarness active />);
    for (const event of [
      pressKey("q"),
      pressKey("ArrowRight", { ctrlKey: true }),
      pressKey(" ", { shiftKey: true }),
    ]) {
      expect(event.defaultPrevented).toBe(false);
    }

    const preemptedEvent = new KeyboardEvent("keydown", {
      key: "ArrowRight",
      bubbles: true,
      cancelable: true,
    });
    preemptedEvent.preventDefault();
    act(() => window.dispatchEvent(preemptedEvent));
    expect(media.currentTime).toBe(10);
  });

  it("supports a caller-provided focus guard", () => {
    const isFocusProtected = vi.fn(() => true);
    render(<ShortcutHarness isFocusProtected={isFocusProtected} />);
    const video = screen.getByLabelText<HTMLVideoElement>("测试视频");
    const media = configureMedia(video);

    pressKey("k");
    expect(isFocusProtected).toHaveBeenCalledTimes(1);
    expect(media.play).not.toHaveBeenCalled();
  });
});

type ConfigurableMedia = {
  currentTime: number;
  duration: number;
  paused: boolean;
  pause: ReturnType<typeof vi.fn>;
  play: ReturnType<typeof vi.fn>;
};

function configureMedia(
  video: HTMLVideoElement,
  initial: Partial<Pick<ConfigurableMedia, "currentTime" | "duration" | "paused">> = {},
): ConfigurableMedia {
  const media: ConfigurableMedia = {
    currentTime: initial.currentTime ?? 0,
    duration: initial.duration ?? 30,
    paused: initial.paused ?? true,
    pause: vi.fn(() => {
      media.paused = true;
    }),
    play: vi.fn(async () => {
      media.paused = false;
    }),
  };

  Object.defineProperties(video, {
    currentTime: {
      configurable: true,
      get: () => media.currentTime,
      set: (value: number) => {
        media.currentTime = value;
      },
    },
    duration: {
      configurable: true,
      get: () => media.duration,
    },
    paused: {
      configurable: true,
      get: () => media.paused,
    },
    pause: {
      configurable: true,
      value: media.pause,
    },
    play: {
      configurable: true,
      value: media.play,
    },
  });

  return media;
}

function pressKey(
  key: string,
  init: KeyboardEventInit = {},
  target: EventTarget = window,
): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => target.dispatchEvent(event));
  return event;
}
