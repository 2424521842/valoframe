import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type { ClipSummary, ThumbnailProgress } from "../../src/types";

type ProgressListener = (progress: ThumbnailProgress) => void;

const mocks = vi.hoisted(() => ({
  ensure: vi.fn(),
  retry: vi.fn(),
  listen: vi.fn(),
  listeners: new Set<ProgressListener>(),
  unlisteners: [] as ReturnType<typeof vi.fn>[],
}));

vi.mock("../../src/api/backend", () => ({
  ensureClipThumbnails: mocks.ensure,
  listenToThumbnailProgress: mocks.listen,
  retryClipThumbnails: mocks.retry,
  THUMBNAIL_ENQUEUE_LIMIT: 200,
}));

import { useThumbnailController } from "../../src/hooks/useThumbnailController";

describe("useThumbnailController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.unlisteners.length = 0;
    mocks.ensure.mockImplementation(async (ids: readonly string[]) => enqueueResult(ids.length));
    mocks.retry.mockImplementation(async (ids: readonly string[]) => enqueueResult(ids.length));
    mocks.listen.mockImplementation(async (listener: ProgressListener) => {
      mocks.listeners.add(listener);
      const unlisten = vi.fn(() => mocks.listeners.delete(listener));
      mocks.unlisteners.push(unlisten);
      return unlisten;
    });
  });

  it("ensures only newly loaded ids within a generation and rechecks a new generation", async () => {
    const onProgress = vi.fn();
    const first = [summary("1"), summary("2")];
    const controller = renderHook(
      ({ generation, clips }) => useThumbnailController({ generation, clips, onProgress }),
      { initialProps: { generation: 1, clips: first } },
    );

    await waitFor(() => expect(mocks.ensure).toHaveBeenCalledTimes(1));
    expect(mocks.ensure).toHaveBeenLastCalledWith(["1", "2"]);

    controller.rerender({ generation: 1, clips: [...first, summary("3")] });
    await waitFor(() => expect(mocks.ensure).toHaveBeenCalledTimes(2));
    expect(mocks.ensure).toHaveBeenLastCalledWith(["3"]);

    controller.rerender({ generation: 1, clips: [...first, summary("3")] });
    await Promise.resolve();
    expect(mocks.ensure).toHaveBeenCalledTimes(2);

    controller.rerender({ generation: 2, clips: [...first, summary("3")] });
    await waitFor(() => expect(mocks.ensure).toHaveBeenCalledTimes(3));
    expect(mocks.ensure).toHaveBeenLastCalledWith(["1", "2", "3"]);
  });

  it("chunks ensure and retry requests at 200 ids without per-card calls", async () => {
    const clips = Array.from({ length: 205 }, (_, index) => summary(String(index + 1)));
    const { result } = renderHook(() => useThumbnailController({
      generation: 1,
      clips,
      onProgress: vi.fn(),
    }));

    await waitFor(() => expect(mocks.ensure).toHaveBeenCalledTimes(2));
    expect(mocks.ensure.mock.calls[0][0]).toHaveLength(200);
    expect(mocks.ensure.mock.calls[1][0]).toHaveLength(5);

    const retried = await result.current.retry([
      ...clips.map((clip) => clip.id),
      clips[0].id,
    ]);
    expect(mocks.retry).toHaveBeenCalledTimes(2);
    expect(retried).toEqual(enqueueResult(205));
  });

  it("keeps one effective listener in StrictMode and cleans every subscription", async () => {
    const controller = renderHook(() => useThumbnailController({
      generation: 1,
      clips: [summary("1")],
      onProgress: vi.fn(),
    }), { reactStrictMode: true });

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    expect(mocks.ensure).toHaveBeenCalledTimes(1);
    expect(mocks.listen.mock.calls.length).toBeGreaterThan(1);
    expect(mocks.unlisteners).toHaveLength(mocks.listen.mock.calls.length);
    expect(mocks.unlisteners.filter((unlisten) => unlisten.mock.calls.length === 0)).toHaveLength(1);

    controller.unmount();
    await waitFor(() => expect(mocks.listeners.size).toBe(0));
    for (const unlisten of mocks.unlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  });

  it("forwards non-ready states and accepts the same ready revision after invalidation", async () => {
    const onProgress = vi.fn();
    renderHook(() => useThumbnailController({
      generation: 1,
      clips: [summary("42")],
      onProgress,
    }));
    await waitFor(() => expect(mocks.listeners.size).toBe(1));

    act(() => {
      emit({ clipId: "42", status: "ready", revision: "rev-1", errorCode: null });
      emit({ clipId: "42", status: "ready", revision: "rev-1", errorCode: null });
      emit({ clipId: "42", status: "evicted", revision: null, errorCode: null });
      emit({ clipId: "42", status: "ready", revision: "rev-1", errorCode: null });
      emit({ clipId: "42", status: "pending", revision: null, errorCode: null });
      emit({ clipId: "42", status: "failed", revision: null, errorCode: "decode" });
      emit({ clipId: "42", status: "unavailable", revision: null, errorCode: "generator" });
      emit({ clipId: "42", status: "suppressed", revision: null, errorCode: null });
      emit({ clipId: "42", status: "ready", revision: "rev-1", errorCode: null });
    });

    expect(onProgress).toHaveBeenCalledTimes(8);
    expect(onProgress).toHaveBeenNthCalledWith(1, {
      clipId: "42",
      status: "ready",
      revision: "rev-1",
      errorCode: null,
    });
    expect(onProgress).toHaveBeenNthCalledWith(2, {
      clipId: "42",
      status: "evicted",
      revision: null,
      errorCode: null,
    });
    expect(onProgress).toHaveBeenNthCalledWith(3, {
      clipId: "42",
      status: "ready",
      revision: "rev-1",
      errorCode: null,
    });
    expect(onProgress.mock.calls.slice(3).map(([progress]) => progress.status))
      .toEqual(["pending", "failed", "unavailable", "suppressed", "ready"]);
  });
});

function summary(id: string): ClipSummary {
  return { ...mockClips[0], id };
}

function enqueueResult(requested: number) {
  return {
    requested,
    queued: requested,
    alreadyQueued: 0,
    skipped: 0,
  };
}

function emit(progress: ThumbnailProgress) {
  for (const listener of mocks.listeners) listener(progress);
}
