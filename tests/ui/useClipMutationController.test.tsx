import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips } from "../../src/data/mockData";
import type {
  BatchMutationResult,
  Clip,
  ClipListQuery,
  ClipSummary,
  SourceDir,
} from "../../src/types";

const mocks = vi.hoisted(() => ({
  addTagToClip: vi.fn(),
  addTagToClips: vi.fn(),
  deleteClipsPermanently: vi.fn(),
  removeClipsFromIndex: vi.fn(),
  removeTagFromClip: vi.fn(),
  removeTagFromClips: vi.fn(),
  setClipFavorite: vi.fn(),
  setClipsFavorite: vi.fn(),
  setClipsTrashed: vi.fn(),
  updateClipNote: vi.fn(),
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    addTagToClip: mocks.addTagToClip,
    addTagToClips: mocks.addTagToClips,
    deleteClipsPermanently: mocks.deleteClipsPermanently,
    removeClipsFromIndex: mocks.removeClipsFromIndex,
    removeTagFromClip: mocks.removeTagFromClip,
    removeTagFromClips: mocks.removeTagFromClips,
    setClipFavorite: mocks.setClipFavorite,
    setClipsFavorite: mocks.setClipsFavorite,
    setClipsTrashed: mocks.setClipsTrashed,
    updateClipNote: mocks.updateClipNote,
  };
});

import { useClipMutationController } from "../../src/hooks/useClipMutationController";

const clipA = detail("a");
const clipB = detail("b");

describe("useClipMutationController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.setClipFavorite.mockResolvedValue({ ...clipA, isFavorite: true });
    mocks.setClipsFavorite.mockResolvedValue(batch([
      { ...clipA, isFavorite: true },
      { ...clipB, isFavorite: true },
    ]));
    mocks.addTagToClips.mockResolvedValue(batch([
      { ...clipA, tags: ["tag-a"] },
      { ...clipB, tags: ["tag-a"] },
    ]));
    mocks.removeTagFromClips.mockResolvedValue(batch([clipA, clipB]));
    mocks.setClipsTrashed.mockResolvedValue(batch([
      { ...clipA, fileStatus: "trashed" },
      { ...clipB, fileStatus: "trashed" },
    ]));
    mocks.deleteClipsPermanently.mockResolvedValue({
      requested: 2,
      deletedIds: ["a", "b"],
      missingIds: [],
      pendingIds: [],
      blocked: [],
      failures: [],
    });
    mocks.removeClipsFromIndex.mockResolvedValue({
      requested: 2,
      removedIds: ["a", "b"],
      missingIds: [],
      blocked: [],
      failures: [],
    });
    mocks.updateClipNote.mockResolvedValue({ ...clipA, note: "new note" });
    mocks.addTagToClip.mockResolvedValue({ ...clipA, tags: ["tag-a"] });
    mocks.removeTagFromClip.mockResolvedValue(clipA);
  });

  it("toggles one favorite and reads the latest query after the command resolves", async () => {
    const request = deferred<Clip>();
    mocks.setClipFavorite.mockReturnValueOnce(request.promise);
    const harness = renderController();
    let promise!: Promise<void>;

    act(() => {
      promise = harness.result.current.toggleFavorite("a");
    });
    harness.setQuery({ favoriteFilter: "favorite" });
    await act(async () => {
      request.resolve({ ...clipA, isFavorite: true });
      await promise;
    });

    expect(mocks.setClipFavorite).toHaveBeenCalledTimes(1);
    expect(mocks.setClipFavorite).toHaveBeenCalledWith("a", true);
    expect(harness.summary("a")?.isFavorite).toBe(true);
    expect(harness.syncDetail).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
    expect(harness.refreshFacets).toHaveBeenCalledTimes(1);
    expect(harness.refreshClips).toHaveBeenCalledTimes(1);
    expect(harness.activity).toHaveBeenCalledWith("已收藏素材");
  });

  it("uses one batch favorite command and reports missing ids", async () => {
    mocks.setClipsFavorite.mockResolvedValueOnce(batch(
      [{ ...clipA, isFavorite: true }],
      ["missing"],
      2,
    ));
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.setFavoriteForClips(["a", "missing"], true))
        .toBe(false);
    });

    expect(mocks.setClipsFavorite).toHaveBeenCalledTimes(1);
    expect(mocks.setClipsFavorite).toHaveBeenCalledWith(["a", "missing"], true);
    expect(harness.summary("a")?.isFavorite).toBe(true);
    expect(harness.activity).toHaveBeenCalledWith(
      "收藏部分完成：匹配 1/2 条；未找到 ID：missing",
    );
  });

  it("does not patch summaries or details when a batch command fails", async () => {
    mocks.setClipsFavorite.mockRejectedValueOnce(new Error("transaction rolled back"));
    const harness = renderController();
    const before = harness.allSummaries();

    await act(async () => {
      expect(await harness.result.current.setFavoriteForClips(["a", "b"], true))
        .toBe(false);
    });

    expect(harness.allSummaries()).toEqual(before);
    expect(harness.updateSummaries).not.toHaveBeenCalled();
    expect(harness.syncDetail).not.toHaveBeenCalled();
    expect(harness.refreshFacets).not.toHaveBeenCalled();
    expect(harness.activity).toHaveBeenCalledWith(
      "批量收藏失败，当前批次未更新：transaction rolled back",
    );
  });

  it("uses one batch tag command and refreshes a newly active tag query", async () => {
    const request = deferred<BatchMutationResult>();
    mocks.addTagToClips.mockReturnValueOnce(request.promise);
    const harness = renderController();
    let promise!: Promise<boolean>;

    act(() => {
      promise = harness.result.current.setTagForClips(["a", "b"], "tag-a", true);
    });
    harness.setQuery({ tagId: 7 });
    await act(async () => {
      request.resolve(batch([
        { ...clipA, tags: ["tag-a"] },
        { ...clipB, tags: ["tag-a"] },
      ]));
      expect(await promise).toBe(true);
    });

    expect(mocks.addTagToClips).toHaveBeenCalledTimes(1);
    expect(mocks.removeTagFromClips).not.toHaveBeenCalled();
    expect(harness.refreshClips).not.toHaveBeenCalled();
    expect(harness.activity).toHaveBeenCalledWith("已添加“战术标签”：2 条素材");
  });

  it("refreshes the page when the latest numeric tag query matches", async () => {
    const request = deferred<BatchMutationResult>();
    mocks.addTagToClips.mockReturnValueOnce(request.promise);
    const harness = renderController();
    let promise!: Promise<boolean>;

    act(() => {
      promise = harness.result.current.setTagForClips(["a"], "7", true);
    });
    harness.setQuery({ tagId: 7 });
    await act(async () => {
      request.resolve(batch([{ ...clipA, tags: ["7"] }]));
      expect(await promise).toBe(true);
    });
    expect(harness.refreshClips).toHaveBeenCalledTimes(1);
  });

  it("trashes in one command, clears the selected clip, and refreshes both views", async () => {
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.setTrashedForClips(["a", "b"], true))
        .toBe(true);
    });

    expect(mocks.setClipsTrashed).toHaveBeenCalledTimes(1);
    expect(harness.clearSelectedClip).toHaveBeenCalledWith(new Set(["a", "b"]));
    expect(harness.refreshClips).toHaveBeenCalledTimes(1);
    expect(harness.refreshFacets).toHaveBeenCalledTimes(1);
  });

  it("keeps partial remove-from-index semantics and removes only successful ids locally", async () => {
    mocks.removeClipsFromIndex.mockResolvedValueOnce({
      requested: 2,
      removedIds: ["a"],
      missingIds: [],
      blocked: [{ clipId: "b", code: "delete-pending", message: "已进入永久删除队列" }],
      failures: [],
    });
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.removeClipsFromIndex(["a", "b"]))
        .toEqual(expect.objectContaining({ removedIds: ["a"] }));
    });

    expect(mocks.removeClipsFromIndex).toHaveBeenCalledTimes(1);
    expect(mocks.removeClipsFromIndex).toHaveBeenCalledWith(["a", "b"]);
    expect(harness.allSummaries().map((clip) => clip.id)).toEqual(["b"]);
    expect(harness.removeDetail).toHaveBeenCalledWith("a");
    expect(harness.removeDetail).not.toHaveBeenCalledWith("b");
    expect(harness.clearSelectedClip).toHaveBeenCalledWith(new Set(["a"]));
    expect(harness.activity).toHaveBeenCalledWith(
      "仅移除索引部分完成：成功 1 条，失败 1 条：已进入永久删除队列",
    );
  });

  it("permanently deletes successful recycle-bin clips and reports per-file failures", async () => {
    mocks.deleteClipsPermanently.mockResolvedValueOnce({
      requested: 2,
      deletedIds: ["a"],
      missingIds: [],
      pendingIds: [],
      blocked: [],
      failures: [{
        clipId: "b",
        code: "permission-denied",
        retryable: true,
        message: "文件正被占用",
      }],
    });
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.deleteClipsPermanently(["a", "b"]))
        .toBe(false);
    });

    expect(mocks.deleteClipsPermanently).toHaveBeenCalledWith(["a", "b"]);
    expect(harness.allSummaries().map((clip) => clip.id)).toEqual(["b"]);
    expect(harness.removeDetail).toHaveBeenCalledWith("a");
    expect(harness.clearSelectedClip).toHaveBeenCalledWith(new Set(["a"]));
    expect(harness.refreshClips).toHaveBeenCalledTimes(1);
    expect(harness.refreshFacets).toHaveBeenCalledTimes(1);
    expect(harness.activity).toHaveBeenCalledWith(
      "已永久删除 1 条；1 条未进入删除队列：文件正被占用",
    );
  });

  it("keeps pending and blocked permanent deletions visible with distinct status messages", async () => {
    mocks.deleteClipsPermanently.mockResolvedValueOnce({
      requested: 2,
      deletedIds: [],
      missingIds: [],
      pendingIds: ["a"],
      blocked: [{
        clipId: "b",
        code: "target-replaced",
        retryable: false,
        message: "待删除视频的文件身份不一致",
      }],
      failures: [],
    });
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.deleteClipsPermanently(["a", "b"]))
        .toBe(false);
    });

    expect(harness.allSummaries().map((clip) => clip.id)).toEqual(["a", "b"]);
    expect(harness.removeDetail).not.toHaveBeenCalled();
    expect(harness.refreshClips).not.toHaveBeenCalled();
    expect(harness.activity).toHaveBeenCalledWith(
      "已永久删除 0 条；1 条已记录删除意图，等待自动重试；1 条因目标变化或安全校验被阻止：待删除视频的文件身份不一致",
    );
  });

  it("accepts pending-only permanent deletion while keeping the clip visible", async () => {
    mocks.deleteClipsPermanently.mockResolvedValueOnce({
      requested: 1,
      deletedIds: [],
      missingIds: [],
      pendingIds: ["a"],
      blocked: [],
      failures: [],
    });
    const harness = renderController();

    await act(async () => {
      expect(await harness.result.current.deleteClipsPermanently(["a"]))
        .toBe(true);
    });

    expect(harness.allSummaries().map((clip) => clip.id)).toEqual(["a", "b"]);
    expect(harness.removeDetail).not.toHaveBeenCalled();
    expect(harness.clearSelectedClip).toHaveBeenCalledWith(new Set(["a"]));
    expect(harness.activity).toHaveBeenCalledWith(
      "已永久删除 0 条素材的本地视频和索引；1 条已记录删除意图，等待自动重试",
    );
  });

  it("updates a note and refreshes when search becomes active during the request", async () => {
    const request = deferred<Clip>();
    mocks.updateClipNote.mockReturnValueOnce(request.promise);
    const harness = renderController();
    let promise!: Promise<void>;

    act(() => {
      promise = harness.result.current.updateNote("a", "new note");
    });
    harness.setQuery({ query: "new note" });
    await act(async () => {
      request.resolve({ ...clipA, note: "new note" });
      await promise;
    });

    expect(harness.refreshClips).toHaveBeenCalledTimes(1);
    expect(harness.syncDetail).toHaveBeenCalledWith(
      expect.objectContaining({ id: "a", note: "new note" }),
    );
    expect(harness.activity).toHaveBeenCalledWith("备注已保存");
  });

  it("syncs one tag mutation and throws after reporting backend failure", async () => {
    const harness = renderController();
    await act(async () => {
      await harness.result.current.toggleTag("a", "tag-a", true);
    });
    expect(mocks.addTagToClip).toHaveBeenCalledTimes(1);
    expect(harness.summary("a")?.tags).toEqual(["tag-a"]);

    mocks.removeTagFromClip.mockRejectedValueOnce(new Error("tag locked"));
    await act(async () => {
      await expect(harness.result.current.toggleTag("a", "tag-a", false))
        .rejects.toThrow("tag locked");
    });
    expect(harness.activity).toHaveBeenCalledWith("标签更新失败：tag locked");
  });

  it("does not write through injected callbacks after unmount", async () => {
    const request = deferred<BatchMutationResult>();
    mocks.setClipsFavorite.mockReturnValueOnce(request.promise);
    const harness = renderController();
    const promise = harness.result.current.setFavoriteForClips(["a"], true);

    harness.unmount();
    request.resolve(batch([{ ...clipA, isFavorite: true }]));
    await expect(promise).resolves.toBe(false);
    expect(harness.updateSummaries).not.toHaveBeenCalled();
    expect(harness.syncDetail).not.toHaveBeenCalled();
    expect(harness.activity).not.toHaveBeenCalled();
  });
});

function renderController() {
  const harness = {
    query: {} as ClipListQuery,
    summaries: [summary(clipA), summary(clipB)],
    activity: vi.fn(),
    refreshClips: vi.fn(async () => true),
    refreshFacets: vi.fn(async () => true),
    syncDetail: vi.fn(),
    removeDetail: vi.fn(),
    clearSelectedClip: vi.fn(),
    updateSummaries: vi.fn(),
    removeSummaries: vi.fn(),
  };
  harness.updateSummaries.mockImplementation(
    (updater: (current: readonly ClipSummary[]) => ClipSummary[]) => {
      harness.summaries = updater(harness.summaries);
    },
  );
  harness.removeSummaries.mockImplementation((ids: Iterable<string>) => {
    const removed = new Set(ids);
    harness.summaries = harness.summaries.filter((clip) => !removed.has(clip.id));
  });

  const controller = renderHook(() => useClipMutationController({
    sourceDirs: [sourceDir()],
    tags: [{ id: "tag-a", label: "战术标签", color: "teal" }],
    getSummary: (clipId) => harness.summaries.find((clip) => clip.id === clipId),
    getDetail: () => undefined,
    getQuery: () => harness.query,
    updateSummaries: harness.updateSummaries,
    removeSummaries: harness.removeSummaries,
    syncDetail: harness.syncDetail,
    removeDetail: harness.removeDetail,
    refreshClips: harness.refreshClips,
    refreshFacets: harness.refreshFacets,
    clearSelectedClip: harness.clearSelectedClip,
    onActivityMessage: harness.activity,
  }));

  return {
    ...controller,
    ...harness,
    setQuery: (query: ClipListQuery) => {
      harness.query = query;
    },
    allSummaries: () => harness.summaries,
    summary: (clipId: string) => harness.summaries.find((clip) => clip.id === clipId),
  };
}

function detail(id: string): Clip {
  const base = mockClips[0];
  return {
    ...base,
    id,
    sourceDirId: "source-a",
    sourceDirName: "来源 source-a",
    sourceDirPath: "",
    fileName: `clip-${id}.mp4`,
    filePath: `D:\\Highlights\\clip-${id}.mp4`,
    clipGroupId: `group-${id}`,
    clipGroupName: `group-${id}`,
    matchId: `match-${id}`,
    isFavorite: false,
    tags: [],
    note: `note-${id}`,
    clipEvents: base.clipEvents?.map((event) => ({ ...event })),
  };
}

function summary(clip: Clip): ClipSummary {
  const value = { ...clip };
  const partial = value as Partial<Clip>;
  delete partial.note;
  delete partial.extractedText;
  delete partial.clipEvents;
  delete partial.eventCount;
  delete partial.roundLabel;
  delete partial.weaponName;
  return value;
}

function batch(
  clips: Clip[],
  missingIds: string[] = [],
  requested = clips.length + missingIds.length,
): BatchMutationResult {
  return {
    requested,
    matched: clips.length,
    updated: clips.length,
    missingIds,
    clips,
  };
}

function sourceDir(): SourceDir {
  return {
    id: "source-a",
    name: "来源 A",
    displayName: "来源 A",
    path: "D:\\Sources\\A",
    enabled: true,
    status: "ready",
    accessibility: true,
    lastError: null,
    clipCount: 2,
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
