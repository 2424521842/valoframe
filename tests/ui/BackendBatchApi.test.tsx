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
  listReviewClipPage,
  listenToThumbnailProgress,
  removeClipsFromIndex,
  removeTagFromClips,
  retryClipThumbnails,
  registerScanSource,
  setScanSourceEnabled,
  setClipsFavorite,
  setClipsTrashed,
  setClipReviewDecision,
  restoreClipReviewState,
  resetClipReviewDecision,
  syncEnabledSources,
  syncScanSource,
  openClipExternally,
  previewScanSourceRelocation,
  relocateScanSource,
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

  it("removes eligible index rows with one batch invoke and maps partial item ids", async () => {
    mocks.invoke.mockResolvedValueOnce({
      requested: 3,
      removedIds: [1],
      missingIds: [404],
      blocked: [{ clipId: 2, code: "delete-pending", message: "已进入永久删除队列" }],
      failures: [],
    });

    await expect(removeClipsFromIndex(["1", "2", "1", "404"])).resolves.toEqual({
      requested: 3,
      removedIds: ["1"],
      missingIds: ["404"],
      blocked: [{ clipId: "2", code: "delete-pending", message: "已进入永久删除队列" }],
      failures: [],
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("remove_clips_from_index", {
      clipIds: [1, 2, 1, 404],
    });
  });

  it("registers and controls persistent video sources with numeric backend ids", async () => {
    mocks.invoke.mockResolvedValueOnce({
      sources: [],
      createdCount: 0,
      duplicateCount: 0,
      normalizedRootPath: "D:\\Recordings",
      requiresOverlapConfirmation: false,
      overlaps: [],
    });
    await registerScanSource({
      sourceKind: "nvidia",
      scanRootPath: "D:\\Recordings",
      displayName: "NVIDIA 录屏",
      enabled: true,
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith("register_scan_source", {
      input: {
        sourceKind: "nvidia",
        scanRootPath: "D:\\Recordings",
        displayName: "NVIDIA 录屏",
        enabled: true,
      },
    });

    mocks.invoke.mockResolvedValueOnce({
      id: 17,
      path: "D:\\Recordings",
      displayName: "NVIDIA 录屏",
      sourceKind: "nvidia",
      scanMode: "recursive-mp4",
      scanRootPath: "D:\\Recordings",
      enabled: false,
      status: "pending",
      accessibility: false,
      lastError: null,
      clipCount: 0,
      lastScanAt: null,
    });
    await setScanSourceEnabled("17", false);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_scan_source_enabled", {
      sourceId: 17,
      enabled: false,
    });

    await syncScanSource("17");
    expect(mocks.invoke).toHaveBeenLastCalledWith("sync_scan_source", { sourceId: 17 });
    await syncEnabledSources();
    expect(mocks.invoke).toHaveBeenLastCalledWith("sync_enabled_sources");
  });

  it("previews and commits source relocation with the frozen camelCase contract", async () => {
    const preview = {
      sourceId: 17,
      oldRootPath: "D:\\Archive",
      newRootPath: "E:\\Moved",
      affectedSources: [{
        id: 18,
        displayName: "ACLOS 账号",
        oldSourcePath: "D:\\Archive\\wonderfulVideos18",
        newSourcePath: "E:\\Moved\\wonderfulVideos18",
        clipCount: 12,
      }],
      exactPathMatchCount: 8,
      identityMatchCount: 2,
      legacyFingerprintMatchCount: 1,
      unmatchedCount: 1,
      newCandidateCount: 3,
      expectedClipUpdateCount: 11,
      expectedGroupUpdateCount: 4,
      expectedCoverUpdateCount: 5,
      expectedMetadataReferenceUpdateCount: 6,
      conflicts: [{
        code: "duplicate-identity",
        message: "稳定身份不唯一",
        oldClipIds: ["91"],
        candidatePaths: ["E:\\Moved\\copy.mp4"],
      }],
      blockers: [],
      canRelocate: true,
    };
    mocks.invoke.mockResolvedValueOnce(preview);

    await expect(previewScanSourceRelocation("17", "E:\\Moved")).resolves.toMatchObject({
      sourceId: "17",
      affectedSources: [{ id: "18" }],
      expectedMetadataReferenceUpdateCount: 6,
      conflicts: [{ oldClipIds: ["91"] }],
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith("preview_scan_source_relocation", {
      sourceId: 17,
      newRootPath: "E:\\Moved",
    });

    mocks.invoke.mockResolvedValueOnce({
      preview,
      relocatedClipCount: 11,
      syncJobId: "relocation-failed-job",
      syncStarted: false,
      syncStatus: "failed",
      syncMessage: "目录读取失败",
    });
    await expect(relocateScanSource("17", "E:\\Moved")).resolves.toMatchObject({
      preview: { sourceId: "17", affectedSources: [{ id: "18" }] },
      relocatedClipCount: 11,
      syncJobId: "relocation-failed-job",
      syncStarted: false,
      syncStatus: "failed",
      syncMessage: "目录读取失败",
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith("relocate_scan_source", {
      sourceId: 17,
      newRootPath: "E:\\Moved",
    });
  });

  it("keeps browser relocation previews read-only", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    mocks.invoke.mockClear();

    const first = await previewScanSourceRelocation("source-ranked", "F:\\Moved-A");
    const second = await previewScanSourceRelocation("source-ranked", "G:\\Moved-B");

    expect(first.oldRootPath).toBe("C:\\Users\\Player\\Videos\\Valorant\\Ranked");
    expect(second.oldRootPath).toBe(first.oldRootPath);
    expect(first.newRootPath).toBe("F:\\Moved-A");
    expect(second.newRootPath).toBe("G:\\Moved-B");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("opens an indexed clip through the secured external-player command", async () => {
    await openClipExternally("23");
    expect(mocks.invoke).toHaveBeenCalledWith("open_clip_externally", { clipId: 23 });
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
      reviewDecision: "liked",
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
        reviewDecision: "liked",
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

  it("wraps the cursor review queue and concurrency-safe decision commands", async () => {
    mocks.invoke.mockResolvedValueOnce({
      items: [],
      snapshotMaxClipId: 99,
      candidateCount: 12,
      limit: 3,
      hasMore: true,
      nextCursor: "opaque-cursor",
    });
    await expect(listReviewClipPage({
      accountId: "match-account-1001",
      agentName: "芮娜",
      mapName: "天枢云阙",
      gameMode: "竞技模式",
      sourceDirIds: [2, 5],
      tagIds: [7, 9],
      recordedFrom: 100,
      recordedTo: 200,
      snapshotMaxClipId: 99,
      cursor: "previous",
      limit: 3,
    })).resolves.toMatchObject({ candidateCount: 12, nextCursor: "opaque-cursor" });
    expect(mocks.invoke).toHaveBeenLastCalledWith("list_review_clip_page", {
      query: {
        accountId: "match-account-1001",
        agentName: "芮娜",
        mapName: "天枢云阙",
        gameMode: "竞技模式",
        sourceDirIds: [2, 5],
        tagIds: [7, 9],
        recordedFrom: 100,
        recordedTo: 200,
        snapshotMaxClipId: 99,
        cursor: "previous",
        limit: 3,
      },
    });

    const before = {
      clipId: 42,
      reviewDecision: "unreviewed",
      reviewedAt: null,
      favorite: false,
    } as const;
    const after = {
      clipId: 42,
      reviewDecision: "liked",
      reviewedAt: "2026-08-09T00:00:00Z",
      favorite: true,
    } as const;
    mocks.invoke.mockResolvedValueOnce({ before, after, changed: true });
    const mappedDecision = await setClipReviewDecision("42", "liked");
    expect(mappedDecision.after.reviewedAt).toBe("2026-08-09T00:00:00Z");
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_clip_review_decision", {
      clipId: 42,
      decision: "liked",
    });

    mocks.invoke.mockResolvedValueOnce({ before: after, after: before, changed: true });
    await restoreClipReviewState(
      "42",
      { ...after, clipId: "42" },
      { ...before, clipId: "42" },
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith("restore_clip_review_state", {
      clipId: 42,
      expectedCurrent: after,
      restore: before,
    });

    mocks.invoke.mockResolvedValueOnce({ before: after, after: before, changed: true });
    await resetClipReviewDecision("42");
    expect(mocks.invoke).toHaveBeenLastCalledWith("reset_clip_review_decision", { clipId: 42 });
  });

  it("loads synthetic string ids in the browser preview detail view", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    mocks.invoke.mockClear();

    await expect(getClipDetail("clip-1")).resolves.toMatchObject({
      id: "clip-1",
      accountDisplayName: "FixtureAlpha#0001",
      officialVideoName: "三杀时刻",
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("filters browser-preview pages by the schema v14 review decision", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    mocks.invoke.mockClear();
    mocks.invoke.mockRejectedValueOnce(new Error("invoke unavailable"));
    mocks.invoke.mockRejectedValueOnce(new Error("invoke unavailable"));

    const liked = await listClipPage({ reviewDecision: "liked", limit: 50 });
    const unreviewed = await listClipPage({ reviewDecision: "unreviewed", limit: 50 });

    expect(liked.items.length).toBeGreaterThan(0);
    expect(liked.items.every((clip) => clip.reviewDecision === "liked")).toBe(
      true,
    );
    expect(unreviewed.items.length).toBeGreaterThan(0);
    expect(
      unreviewed.items.every((clip) => clip.reviewDecision === "unreviewed"),
    ).toBe(true);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("keeps browser-preview review decisions idempotent and rejects malformed cursors", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    const first = await setClipReviewDecision("clip-2", "disliked");
    const repeated = await setClipReviewDecision("clip-2", "disliked");
    expect(repeated).toEqual({
      before: first.after,
      after: first.after,
      changed: false,
    });
    await restoreClipReviewState("clip-2", first.after, first.before);

    mocks.invoke.mockRejectedValueOnce(new Error("invoke unavailable"));
    await expect(listReviewClipPage({
      snapshotMaxClipId: 3,
      cursor: "not-a-valid-cursor",
    })).rejects.toThrow(/无效的旧版挑片游标/);
  });

  it("filters browser-preview review queues by game metadata and binds it to the cursor", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    mocks.invoke.mockRejectedValueOnce(new Error("invoke unavailable"));
    const query = {
      accountId: "match-account-1001",
      agentName: "芮娜",
      mapName: "天枢云阙",
      gameMode: "竞技模式",
      limit: 1,
    };
    const first = await listReviewClipPage(query);
    expect(first.items).toHaveLength(1);
    expect(first.items[0]).toMatchObject({
      accountId: query.accountId,
      agentName: query.agentName,
      mapName: query.mapName,
      gameMode: query.gameMode,
    });
    expect(first.nextCursor).not.toBeNull();

    mocks.invoke.mockRejectedValueOnce(new Error("invoke unavailable"));
    await expect(listReviewClipPage({
      ...query,
      mapName: "隐世修所",
      snapshotMaxClipId: first.snapshotMaxClipId,
      cursor: first.nextCursor ?? undefined,
    })).rejects.toThrow(/筛选条件不匹配/);
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
