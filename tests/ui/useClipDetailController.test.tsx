import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type { ClipDetail, SourceDir } from "../../src/types";

const mocks = vi.hoisted(() => ({
  getClipDetail: vi.fn(),
}));

vi.mock("../../src/api/backend", () => ({
  commandErrorMessage: (error: unknown) => {
    if (error instanceof Error) return error.message;
    if (typeof error === "object" && error !== null && "message" in error) {
      return String(error.message);
    }
    return String(error);
  },
  getClipDetail: mocks.getClipDetail,
  mergeClipsWithSources: (
    clips: readonly ClipDetail[],
    sources: readonly SourceDir[],
  ) => clips.map((clip) => {
    const source = sources.find((candidate) => candidate.id === clip.sourceDirId);
    return source
      ? {
        ...clip,
        sourceDirName: source.displayName,
        sourceDirPath: source.path,
        accountSourceName: source.displayName,
      }
      : clip;
  }),
}));

import { useClipDetailController } from "../../src/hooks/useClipDetailController";

const sourceA = source("source-a", "来源 A");

describe("useClipDetailController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getClipDetail.mockImplementation(async (clipId: string) => detail(clipId));
  });

  it("loads only while preview is active and a clip is selected", async () => {
    const controller = renderController({ active: false, clipId: "a" });

    expect(controller.result.current.state.status).toBe("idle");
    expect(mocks.getClipDetail).not.toHaveBeenCalled();

    controller.rerender({
      active: true,
      clipId: "a",
      sourceDirs: [sourceA],
      cacheLimit: 6,
    });
    await waitForReady(controller.result, "a");
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(1);
    expect(controller.result.current.state.status).toBe("ready");
  });

  it("deduplicates its initial request under StrictMode effect replay", async () => {
    const controller = renderController({ reactStrictMode: true });

    await waitForReady(controller.result, "a");
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(1);
  });

  it("ignores a delayed result after switching clips or closing preview", async () => {
    const requestA = deferred<ClipDetail>();
    const requestB = deferred<ClipDetail>();
    const requestC = deferred<ClipDetail>();
    mocks.getClipDetail
      .mockReturnValueOnce(requestA.promise)
      .mockReturnValueOnce(requestB.promise)
      .mockReturnValueOnce(requestC.promise);
    const controller = renderController();
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledTimes(1));

    controller.rerender({
      active: true,
      clipId: "b",
      sourceDirs: [sourceA],
      cacheLimit: 6,
    });
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledTimes(2));
    await act(async () => {
      requestB.resolve(detail("b"));
      await requestB.promise;
    });
    await waitForReady(controller.result, "b");

    await act(async () => {
      requestA.resolve(detail("a"));
      await requestA.promise;
    });
    expect(readyClipId(controller.result.current.state)).toBe("b");

    controller.rerender({
      active: true,
      clipId: "c",
      sourceDirs: [sourceA],
      cacheLimit: 6,
    });
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledTimes(3));
    act(() => controller.result.current.cancelPending());
    controller.rerender({
      active: false,
      clipId: "c",
      sourceDirs: [sourceA],
      cacheLimit: 6,
    });
    await act(async () => {
      requestC.resolve(detail("c"));
      await requestC.promise;
    });
    expect(controller.result.current.state.status).toBe("idle");
  });

  it("maps clip-not-found to a stable state", async () => {
    mocks.getClipDetail.mockRejectedValueOnce({
      code: "clip-not-found",
      message: "素材不存在",
    });
    const { result } = renderController();

    await waitFor(() => expect(result.current.state.status).toBe("not-found"));
    expect(result.current.state.error).toBe("素材不存在");
  });

  it("reports ordinary errors and retries the current clip", async () => {
    mocks.getClipDetail
      .mockRejectedValueOnce(new Error("database unavailable"))
      .mockResolvedValueOnce(detail("a"));
    const { result } = renderController();

    await waitFor(() => expect(result.current.state.status).toBe("error"));
    expect(result.current.state.error).toBe("database unavailable");

    await act(async () => {
      expect(await result.current.retry()).toBe(true);
    });
    expect(readyClipId(result.current.state)).toBe("a");
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);
  });

  it("uses an LRU cache, promotes hits, and evicts only the oldest detail", async () => {
    const controller = renderController({ cacheLimit: 2 });
    await waitForReady(controller.result, "a");
    await select(controller, "b", 2);
    await select(controller, "a", 2);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);

    await select(controller, "c", 2);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(3);
    await select(controller, "a", 2);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(3);

    await select(controller, "b", 2);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(4);
  });

  it("rehydrates cached and visible details when source DTOs change", async () => {
    const controller = renderController();
    await waitForReady(controller.result, "a");
    expect(readyClip(controller.result.current.state)?.sourceDirName).toBe("来源 A");

    controller.rerender({
      active: true,
      clipId: "a",
      sourceDirs: [source("source-a", "重命名来源")],
      cacheLimit: 6,
    });
    await waitFor(() =>
      expect(readyClip(controller.result.current.state)?.sourceDirName)
        .toBe("重命名来源"),
    );
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(1);
  });

  it("synchronizes mutations into the active detail and valid cache entries", async () => {
    const controller = renderController();
    await waitForReady(controller.result, "a");

    act(() => controller.result.current.syncClip({
      ...detail("a"),
      isFavorite: true,
      note: "updated note",
      tags: ["tag-a", "tag-b"],
    }));
    expect(readyClip(controller.result.current.state)?.isFavorite).toBe(true);
    expect(readyClip(controller.result.current.state)?.note).toBe("updated note");
    expect(controller.result.current.getClip("a")?.isFavorite).toBe(true);

    await select(controller, "b", 6);
    await select(controller, "a", 6);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);
    expect(readyClip(controller.result.current.state)?.isFavorite).toBe(true);

    act(() => controller.result.current.removeTag("tag-a"));
    expect(readyClip(controller.result.current.state)?.tags).toEqual(["tag-b"]);
  });

  it("clears and restores the same thumbnail revision in the active detail and LRU cache", async () => {
    const controller = renderController();
    await waitForReady(controller.result, "a");

    act(() => controller.result.current.patchThumbnail("a", {
      thumbnailStatus: "ready",
      thumbnailRevision: "rev-2",
      thumbnailUrl: "clip-media://cover/a?v=rev-2",
    }));
    expect(readyClip(controller.result.current.state)?.thumbnailUrl)
      .toBe("clip-media://cover/a?v=rev-2");
    expect(controller.result.current.getClip("a")?.thumbnailRevision).toBe("rev-2");

    act(() => controller.result.current.patchThumbnail("a", {
      thumbnailStatus: "evicted",
      thumbnailRevision: null,
      thumbnailUrl: null,
    }));
    expect(readyClip(controller.result.current.state)?.thumbnailStatus).toBe("evicted");
    expect(readyClip(controller.result.current.state)?.thumbnailUrl).toBeNull();
    expect(controller.result.current.getClip("a")?.thumbnailRevision).toBeNull();

    act(() => controller.result.current.patchThumbnail("a", {
      thumbnailStatus: "ready",
      thumbnailRevision: "rev-2",
      thumbnailUrl: "clip-media://cover/a?v=rev-2",
    }));
    expect(readyClip(controller.result.current.state)?.thumbnailUrl)
      .toBe("clip-media://cover/a?v=rev-2");

    await select(controller, "b", 6);
    await select(controller, "a", 6);
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);
    expect(readyClip(controller.result.current.state)?.thumbnailRevision).toBe("rev-2");
  });

  it("immediately reloads the active detail after invalidation and ignores stale work", async () => {
    const staleRequest = deferred<ClipDetail>();
    const refreshedRequest = deferred<ClipDetail>();
    mocks.getClipDetail
      .mockReturnValueOnce(staleRequest.promise)
      .mockReturnValueOnce(refreshedRequest.promise);
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.state.status).toBe("loading"));

    act(() => controller.result.current.invalidate());
    expect(controller.result.current.state.status).toBe("loading");
    await waitFor(() => expect(mocks.getClipDetail).toHaveBeenCalledTimes(2));

    await act(async () => {
      staleRequest.resolve({ ...detail("a"), note: "stale note" });
      await staleRequest.promise;
    });
    expect(controller.result.current.state.status).toBe("loading");

    await act(async () => {
      refreshedRequest.resolve({ ...detail("a"), note: "refreshed note" });
      await refreshedRequest.promise;
    });
    expect(readyClipId(controller.result.current.state)).toBe("a");
    expect(readyClip(controller.result.current.state)?.note).toBe("refreshed note");
  });

  it("removes deleted clips from the cache", async () => {
    const controller = renderController();
    await waitForReady(controller.result, "a");

    act(() => controller.result.current.removeClip("a"));
    expect(controller.result.current.state.status).toBe("idle");
    mocks.getClipDetail.mockResolvedValueOnce(detail("a"));
    await act(async () => {
      expect(await controller.result.current.retry()).toBe(true);
    });
    expect(mocks.getClipDetail).toHaveBeenCalledTimes(2);
  });
});

type ControllerProps = {
  active: boolean;
  clipId: string;
  sourceDirs: SourceDir[];
  cacheLimit: number;
};

function renderController(options: {
  active?: boolean;
  clipId?: string;
  cacheLimit?: number;
  reactStrictMode?: boolean;
} = {}) {
  return renderHook(
    (props: ControllerProps) => useClipDetailController(props),
    {
      initialProps: {
        active: options.active ?? true,
        clipId: options.clipId ?? "a",
        sourceDirs: [sourceA],
        cacheLimit: options.cacheLimit ?? 6,
      },
      reactStrictMode: options.reactStrictMode,
    },
  );
}

async function select(
  controller: ReturnType<typeof renderController>,
  clipId: string,
  cacheLimit: number,
) {
  controller.rerender({
    active: true,
    clipId,
    sourceDirs: [sourceA],
    cacheLimit,
  });
  await waitForReady(controller.result, clipId);
}

async function waitForReady(
  result: ReturnType<typeof renderController>["result"],
  clipId: string,
) {
  await waitFor(() => expect(readyClipId(result.current.state)).toBe(clipId));
}

function readyClipId(state: ReturnType<typeof useClipDetailController>["state"]): string | null {
  return state.status === "ready" ? state.clip.id : null;
}

function readyClip(state: ReturnType<typeof useClipDetailController>["state"]): ClipDetail | null {
  return state.status === "ready" ? state.clip : null;
}

function detail(id: string): ClipDetail {
  const base = mockClips[0];
  return {
    ...base,
    id,
    sourceDirId: "source-a",
    sourceDirName: "来源 source-a",
    sourceDirPath: "",
    accountSourceName: "来源 source-a",
    fileName: `clip-${id}.mp4`,
    filePath: `D:\\Highlights\\clip-${id}.mp4`,
    clipGroupId: `group-${id}`,
    clipGroupName: `group-${id}`,
    matchId: `match-${id}`,
    tags: [],
    note: `note-${id}`,
    clipEvents: base.clipEvents?.map((event) => ({ ...event })),
  };
}

function source(id: string, displayName: string): SourceDir {
  return {
    id,
    name: displayName,
    displayName,
    path: `D:\\Sources\\${id}`,
    enabled: true,
    status: "ready",
    accessibility: true,
    lastError: null,
    clipCount: 1,
    lastScanAt: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}
