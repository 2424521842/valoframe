import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => path,
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

import {
  addTagToClips,
  deleteClipsPermanently,
  ensureClipThumbnails,
  exportClips,
  getClipDetail,
  getLibraryFacets,
  getThumbnailStatus,
  listClipPage,
  listenToThumbnailProgress,
  removeTagFromClips,
  retryClipThumbnails,
  setClipsFavorite,
  setClipsTrashed,
} from "../../src/api/backend";
import type { BackendBatchMutationResult } from "../../src/types";

const emptyResult: BackendBatchMutationResult = {
  requested: 2,
  matched: 0,
  updated: 0,
  missingIds: [1, 2],
  clips: [],
};

describe("backend batch mutation APIs", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    mocks.invoke.mockResolvedValue(emptyResult);
    mocks.listen.mockResolvedValue(() => undefined);
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  });

  it("favorites multiple clips with one Tauri invoke", async () => {
    await setClipsFavorite(["1", "2", "1"], true);

    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("set_clips_favorite", {
      clipIds: [1, 2, 1],
      isFavorite: true,
    });
  });

  it("exports multiple clips with one Tauri invoke and maps returned ids", async () => {
    mocks.invoke.mockResolvedValueOnce({
      requested: 2,
      exported: 1,
      failed: 1,
      destinationDir: "D:\\Exports",
      exportedIds: [1],
      missingIds: [],
      missingFileIds: [2],
      exports: [{
        clipId: 1,
        fileName: "ace.mp4",
        destinationPath: "D:\\Exports\\ace.mp4",
        bytesCopied: 2048,
      }],
      failures: [{
        clipId: 2,
        code: "source-file-missing",
        message: "源视频文件不存在",
      }],
    });

    await expect(exportClips(["1", "2", "1"], "D:\\Exports")).resolves.toEqual({
      requested: 2,
      exported: 1,
      failed: 1,
      destinationDir: "D:\\Exports",
      exportedIds: ["1"],
      missingIds: [],
      missingFileIds: ["2"],
      exports: [{
        clipId: "1",
        fileName: "ace.mp4",
        destinationPath: "D:\\Exports\\ace.mp4",
        bytesCopied: 2048,
      }],
      failures: [{
        clipId: "2",
        code: "source-file-missing",
        message: "源视频文件不存在",
      }],
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("export_clips", {
      clipIds: [1, 2, 1],
      destinationDir: "D:\\Exports",
    });
  });

  it("exposes the paginated summary and on-demand detail commands without switching production", async () => {
    mocks.invoke.mockResolvedValueOnce({
      items: [],
      offset: 25,
      limit: 25,
      totalCount: 100,
      hasMore: true,
      nextOffset: 50,
    });
    await listClipPage({
      offset: 25,
      limit: 25,
      query: "FixtureAlpha%_\\",
      accountId: "match-account-1001",
      tagId: 7,
      metadataStatus: "enriched",
      sortBy: "modified-desc",
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith("list_clip_page", {
      query: {
        offset: 25,
        limit: 25,
        query: "FixtureAlpha%_\\",
        accountId: "match-account-1001",
        tagId: 7,
        metadataStatus: "enriched",
        sortBy: "modified-desc",
      },
    });

    mocks.invoke.mockResolvedValueOnce({ id: 42, tagIds: [], tags: [], clipEvents: [] });
    await getClipDetail("42");
    expect(mocks.invoke).toHaveBeenLastCalledWith("get_clip_detail", {
      clipId: 42,
    });
  });

  it("exposes whole-library facets through a parameter-free command", async () => {
    mocks.invoke.mockResolvedValueOnce({
      totalCount: 0,
      activeCount: 0,
      favoriteCount: 0,
      activeFavoriteCount: 0,
      trashedCount: 0,
      taggedCount: 0,
      activeTaggedCount: 0,
      totalSizeBytes: 0,
      activeSizeBytes: 0,
      sizeBytesMin: null,
      sizeBytesMax: null,
      recentCount: 0,
      recordedAtMin: null,
      recordedAtMax: null,
      modifiedAtMin: null,
      modifiedAtMax: null,
      fileStatuses: [],
      metadataStatuses: [],
      accounts: [],
      sourceDirs: [],
      agents: [],
      maps: [],
      gameModes: [],
      killTypes: [],
      tags: [],
    });

    await getLibraryFacets();

    expect(mocks.invoke).toHaveBeenLastCalledWith("get_library_facets");
  });

  it("exposes bounded thumbnail enqueue, retry, and status commands", async () => {
    const enqueueResult = {
      requested: 2,
      queued: 1,
      alreadyQueued: 1,
      skipped: 0,
    };
    mocks.invoke.mockResolvedValueOnce(enqueueResult);
    await expect(ensureClipThumbnails(["42", "43"])).resolves.toEqual(enqueueResult);
    expect(mocks.invoke).toHaveBeenLastCalledWith("ensure_clip_thumbnails", {
      clipIds: [42, 43],
    });

    mocks.invoke.mockResolvedValueOnce(enqueueResult);
    await retryClipThumbnails(["42", "43"]);
    expect(mocks.invoke).toHaveBeenLastCalledWith("retry_clip_thumbnails", {
      clipIds: [42, 43],
    });

    const queueStatus = {
      generatorStatus: "available" as const,
      pendingCount: 1,
      runningCount: 1,
      readyCount: 4,
      failedCount: 2,
      unavailableCount: 0,
      evictedCount: 3,
      cacheBytes: 1024,
      processingClipId: 42,
      lastErrorCode: null,
    };
    mocks.invoke.mockResolvedValueOnce(queueStatus);
    await expect(getThumbnailStatus()).resolves.toEqual(queueStatus);
    expect(mocks.invoke).toHaveBeenLastCalledWith("get_thumbnail_status");
  });

  it("keeps every thumbnail API as a browser-preview no-op", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    mocks.invoke.mockClear();

    await expect(ensureClipThumbnails(["42"])).resolves.toEqual({
      requested: 0,
      queued: 0,
      alreadyQueued: 0,
      skipped: 0,
    });
    await expect(retryClipThumbnails(["42"])).resolves.toEqual({
      requested: 0,
      queued: 0,
      alreadyQueued: 0,
      skipped: 0,
    });
    await expect(getThumbnailStatus()).resolves.toMatchObject({
      generatorStatus: "unknown",
      pendingCount: 0,
      runningCount: 0,
    });
    const unlisten = await listenToThumbnailProgress(vi.fn());
    expect(unlisten()).toBeUndefined();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.listen).not.toHaveBeenCalled();
  });

  it("maps numeric thumbnail progress ids to the frontend string identity", async () => {
    const onProgress = vi.fn();
    const unlisten = vi.fn();
    mocks.listen.mockImplementationOnce(async (_eventName, listener) => {
      listener({
        payload: {
          clipId: 42,
          status: "ready",
          revision: "rev-42",
          errorCode: null,
        },
      });
      return unlisten;
    });

    const cleanup = await listenToThumbnailProgress(onProgress);

    expect(mocks.listen).toHaveBeenCalledWith("thumbnail-progress", expect.any(Function));
    expect(onProgress).toHaveBeenCalledWith({
      clipId: "42",
      status: "ready",
      revision: "rev-42",
      errorCode: null,
    });
    cleanup();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("adds or removes one tag from multiple clips with one invoke per action", async () => {
    await addTagToClips(["1", "2"], "7");
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenLastCalledWith("add_tag_to_clips", {
      clipIds: [1, 2],
      tagId: 7,
    });

    mocks.invoke.mockClear();
    await removeTagFromClips(["1", "2"], "7");
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenLastCalledWith("remove_tag_from_clips", {
      clipIds: [1, 2],
      tagId: 7,
    });
  });

  it("recycles or restores multiple clips with one invoke per action", async () => {
    await setClipsTrashed(["1", "2"], true);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_clips_trashed", {
      clipIds: [1, 2],
      isTrashed: true,
    });

    mocks.invoke.mockClear();
    await setClipsTrashed(["1", "2"], false);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_clips_trashed", {
      clipIds: [1, 2],
      isTrashed: false,
    });
  });

  it("permanently deletes multiple recycle-bin clips with one invoke", async () => {
    mocks.invoke.mockResolvedValueOnce({
      requested: 2,
      deletedIds: [1],
      missingIds: [2],
      pendingIds: [],
      blocked: [],
      failures: [],
    });

    await expect(deleteClipsPermanently(["1", "2"])).resolves.toEqual({
      requested: 2,
      deletedIds: ["1"],
      missingIds: ["2"],
      pendingIds: [],
      blocked: [],
      failures: [],
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenLastCalledWith("delete_clips_permanently", {
      clipIds: [1, 2],
    });
  });
});
