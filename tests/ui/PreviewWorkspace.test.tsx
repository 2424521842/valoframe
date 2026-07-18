import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getClipMedia } from "../../src/api/backend";
import { mockClips } from "../../src/data/mockData";
import { PreviewWorkspace } from "../../src/screens/PreviewWorkspace";
import type { ClipMedia } from "../../src/types";

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
    const props = createPreviewProps({ clip: first, clips });
    const { container, rerender } = render(<PreviewWorkspace {...props} />);

    expect(railTitles(container)).toEqual(["固定顺序一", "固定顺序二", "固定顺序三"]);
    expect(activeRailTitle(container)).toBe("固定顺序一");

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
