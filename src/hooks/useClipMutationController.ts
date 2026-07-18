import { useCallback, useEffect, useRef } from "react";
import {
  addTagToClip,
  addTagToClips,
  commandErrorMessage,
  deleteClipsPermanently,
  mergeClipsWithSources,
  removeClipFromIndex,
  removeTagFromClip,
  removeTagFromClips,
  setClipFavorite,
  setClipsFavorite,
  setClipsTrashed,
  toClipSummary,
  updateClipNote,
} from "../api/backend";
import type {
  BatchMutationResult,
  Clip,
  ClipDetail,
  ClipListQuery,
  ClipSummary,
  SourceDir,
  Tag,
} from "../types";

export type ClipQueryRefreshOptions = {
  preserveActivity?: boolean;
};

export type UseClipMutationControllerOptions = {
  sourceDirs: readonly SourceDir[];
  tags: readonly Tag[];
  getSummary: (clipId: string) => ClipSummary | undefined;
  getDetail: (clipId: string) => ClipDetail | undefined;
  getQuery: () => ClipListQuery;
  updateSummaries: (
    updater: (current: readonly ClipSummary[]) => ClipSummary[],
  ) => void;
  removeSummaries: (clipIds: Iterable<string>) => void;
  syncDetail: (clip: ClipDetail) => void;
  removeDetail: (clipId: string) => void;
  refreshClips: (options?: ClipQueryRefreshOptions) => Promise<boolean>;
  refreshFacets: () => Promise<boolean>;
  clearSelectedClip: (clipIds: ReadonlySet<string>) => void;
  onActivityMessage: (message: string) => void;
};

export type ClipMutationController = {
  toggleFavorite: (clipId: string) => Promise<void>;
  setFavoriteForClips: (clipIds: string[], isFavorite: boolean) => Promise<boolean>;
  setTagForClips: (
    clipIds: string[],
    tagId: string,
    shouldAttach: boolean,
  ) => Promise<boolean>;
  setTrashedForClips: (clipIds: string[], isTrashed: boolean) => Promise<boolean>;
  deleteClipsPermanently: (clipIds: string[]) => Promise<boolean>;
  removeClipsFromIndex: (clipIds: string[]) => Promise<boolean>;
  updateNote: (clipId: string, note: string) => Promise<void>;
  toggleTag: (
    clipId: string,
    tagId: string,
    shouldAttach: boolean,
  ) => Promise<void>;
};

export function useClipMutationController(
  options: UseClipMutationControllerOptions,
): ClipMutationController {
  const mountedRef = useRef(false);
  const optionsRef = useRef(options);
  optionsRef.current = options;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const applyUpdatedClips = useCallback((updatedClips: readonly Clip[]) => {
    const current = optionsRef.current;
    const hydratedClips = mergeClipsWithSources(updatedClips, current.sourceDirs);
    const updatedById = new Map(
      hydratedClips.map((clip) => [clip.id, toClipSummary(clip)]),
    );
    current.updateSummaries((summaries) =>
      summaries.map((clip) => updatedById.get(clip.id) ?? clip),
    );
    for (const clip of updatedClips) current.syncDetail(clip);
  }, []);

  const toggleFavorite = useCallback(async (clipId: string): Promise<void> => {
    const before = optionsRef.current;
    const clip = before.getSummary(clipId) ?? before.getDetail(clipId);
    if (!clip) return;
    try {
      const updatedClip = await setClipFavorite(clipId, !clip.isFavorite);
      if (!mountedRef.current) return;
      applyUpdatedClips([updatedClip]);
      const current = optionsRef.current;
      const refreshes = [current.refreshFacets()];
      if (current.getQuery().favoriteFilter) {
        refreshes.push(current.refreshClips());
      }
      await Promise.all(refreshes);
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          updatedClip.isFavorite ? "已收藏素材" : "已取消收藏",
        );
      }
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `收藏更新失败：${commandErrorMessage(requestError)}`,
        );
      }
    }
  }, [applyUpdatedClips]);

  const setFavoriteForClips = useCallback(async (
    clipIds: string[],
    isFavorite: boolean,
  ): Promise<boolean> => {
    try {
      const result = await setClipsFavorite(clipIds, isFavorite);
      if (!mountedRef.current) return false;
      applyUpdatedClips(result.clips);
      const current = optionsRef.current;
      const refreshes = [current.refreshFacets()];
      if (current.getQuery().favoriteFilter) {
        refreshes.push(current.refreshClips());
      }
      await Promise.all(refreshes);
      if (!mountedRef.current) return false;
      if (result.missingIds.length > 0) {
        current.onActivityMessage(batchMissingMessage("收藏", result));
        return false;
      }
      current.onActivityMessage(
        `${isFavorite ? "已收藏" : "已取消收藏"} ${result.matched} 条素材`,
      );
      return true;
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `批量收藏失败，当前批次未更新：${commandErrorMessage(requestError)}`,
        );
      }
      return false;
    }
  }, [applyUpdatedClips]);

  const setTagForClips = useCallback(async (
    clipIds: string[],
    tagId: string,
    shouldAttach: boolean,
  ): Promise<boolean> => {
    const tagLabel = optionsRef.current.tags.find((tag) => tag.id === tagId)?.label ?? "标签";
    try {
      const result = shouldAttach
        ? await addTagToClips(clipIds, tagId)
        : await removeTagFromClips(clipIds, tagId);
      if (!mountedRef.current) return false;
      applyUpdatedClips(result.clips);
      const current = optionsRef.current;
      const refreshes = [current.refreshFacets()];
      if (String(current.getQuery().tagId ?? "") === tagId) {
        refreshes.push(current.refreshClips());
      }
      await Promise.all(refreshes);
      if (!mountedRef.current) return false;
      if (result.missingIds.length > 0) {
        current.onActivityMessage(batchMissingMessage(`标签“${tagLabel}”`, result));
        return false;
      }
      current.onActivityMessage(
        `${shouldAttach ? "已添加" : "已移除"}“${tagLabel}”：${result.matched} 条素材`,
      );
      return true;
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `批量标签更新失败，当前批次未更新：${commandErrorMessage(requestError)}`,
        );
      }
      return false;
    }
  }, [applyUpdatedClips]);

  const setTrashedForClips = useCallback(async (
    clipIds: string[],
    isTrashed: boolean,
  ): Promise<boolean> => {
    try {
      const result = await setClipsTrashed(clipIds, isTrashed);
      if (!mountedRef.current) return false;
      applyUpdatedClips(result.clips);
      const current = optionsRef.current;
      if (isTrashed) {
        current.clearSelectedClip(new Set(result.clips.map((clip) => clip.id)));
      }
      await Promise.all([current.refreshClips(), current.refreshFacets()]);
      if (!mountedRef.current) return false;
      if (result.missingIds.length > 0) {
        current.onActivityMessage(
          batchMissingMessage(isTrashed ? "移入回收站" : "恢复", result),
        );
        return false;
      }
      current.onActivityMessage(
        `${isTrashed ? "已移入回收站" : "已恢复"} ${result.matched} 条素材`,
      );
      return true;
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `批量${isTrashed ? "回收" : "恢复"}失败，当前批次未更新：${commandErrorMessage(requestError)}`,
        );
      }
      return false;
    }
  }, [applyUpdatedClips]);

  const removeClipsFromIndex = useCallback(async (
    clipIds: string[],
  ): Promise<boolean> => {
    const removedIds: string[] = [];
    const errors: string[] = [];
    for (const clipId of clipIds) {
      try {
        await removeClipFromIndex(clipId);
        removedIds.push(clipId);
      } catch (requestError) {
        errors.push(commandErrorMessage(requestError));
      }
    }
    if (!mountedRef.current) return false;
    const current = optionsRef.current;
    const removed = new Set(removedIds);
    current.removeSummaries(removed);
    for (const clipId of removed) current.removeDetail(clipId);
    current.clearSelectedClip(removed);
    if (removed.size > 0) {
      await Promise.all([current.refreshClips(), current.refreshFacets()]);
    }
    if (!mountedRef.current) return false;
    current.onActivityMessage(
      errors.length === 0
        ? `已从索引移除 ${removedIds.length} 条素材，原视频文件未删除`
        : `已移除 ${removedIds.length} 条，失败 ${errors.length} 条：${errors[0]}`,
    );
    return errors.length === 0;
  }, []);

  const permanentlyDeleteClips = useCallback(async (
    clipIds: string[],
  ): Promise<boolean> => {
    try {
      const result = await deleteClipsPermanently(clipIds);
      if (!mountedRef.current) return false;
      const current = optionsRef.current;
      const completedIds = new Set([...result.deletedIds, ...result.missingIds]);
      current.removeSummaries(completedIds);
      for (const clipId of completedIds) current.removeDetail(clipId);
      current.clearSelectedClip(new Set([...completedIds, ...result.pendingIds]));
      if (completedIds.size > 0) {
        await Promise.all([current.refreshClips(), current.refreshFacets()]);
      }
      if (!mountedRef.current) return false;

      if (result.blocked.length === 0 && result.failures.length === 0) {
        const missingNote = result.missingIds.length > 0
          ? `，另有 ${result.missingIds.length} 条记录已不存在`
          : "";
        const pendingNote = result.pendingIds.length > 0
          ? `；${result.pendingIds.length} 条已记录删除意图，等待自动重试`
          : "";
        current.onActivityMessage(
          `已永久删除 ${result.deletedIds.length} 条素材的本地视频和索引${missingNote}${pendingNote}`,
        );
        return true;
      }

      const outcomes = [`已永久删除 ${result.deletedIds.length} 条`];
      if (result.pendingIds.length > 0) {
        outcomes.push(`${result.pendingIds.length} 条已记录删除意图，等待自动重试`);
      }
      if (result.blocked.length > 0) {
        outcomes.push(
          `${result.blocked.length} 条因目标变化或安全校验被阻止：${result.blocked[0].message}`,
        );
      }
      if (result.failures.length > 0) {
        outcomes.push(`${result.failures.length} 条未进入删除队列：${result.failures[0].message}`);
      }
      current.onActivityMessage(outcomes.join("；"));
      return false;
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `永久删除失败：${commandErrorMessage(requestError)}`,
        );
      }
      return false;
    }
  }, []);

  const updateNote = useCallback(async (clipId: string, note: string): Promise<void> => {
    try {
      const updatedClip = await updateClipNote(clipId, note);
      if (!mountedRef.current) return;
      applyUpdatedClips([updatedClip]);
      const current = optionsRef.current;
      if (current.getQuery().query) {
        await current.refreshClips();
      }
      if (mountedRef.current) optionsRef.current.onActivityMessage("备注已保存");
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `备注保存失败：${commandErrorMessage(requestError)}`,
        );
      }
      throw requestError;
    }
  }, [applyUpdatedClips]);

  const toggleTag = useCallback(async (
    clipId: string,
    tagId: string,
    shouldAttach: boolean,
  ): Promise<void> => {
    try {
      const updatedClip = shouldAttach
        ? await addTagToClip(clipId, tagId)
        : await removeTagFromClip(clipId, tagId);
      if (!mountedRef.current) return;
      applyUpdatedClips([updatedClip]);
      const current = optionsRef.current;
      const refreshes = [current.refreshFacets()];
      if (String(current.getQuery().tagId ?? "") === tagId) {
        refreshes.push(current.refreshClips());
      }
      await Promise.all(refreshes);
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          shouldAttach ? "已添加标签" : "已移除标签",
        );
      }
    } catch (requestError) {
      if (mountedRef.current) {
        optionsRef.current.onActivityMessage(
          `标签更新失败：${commandErrorMessage(requestError)}`,
        );
      }
      throw requestError;
    }
  }, [applyUpdatedClips]);

  return {
    toggleFavorite,
    setFavoriteForClips,
    setTagForClips,
    setTrashedForClips,
    deleteClipsPermanently: permanentlyDeleteClips,
    removeClipsFromIndex,
    updateNote,
    toggleTag,
  };
}

export function batchMissingMessage(
  action: string,
  result: Pick<BatchMutationResult, "requested" | "matched" | "missingIds">,
): string {
  const preview = result.missingIds.slice(0, 3).join("、");
  const remainder = result.missingIds.length > 3
    ? ` 等 ${result.missingIds.length} 个 ID`
    : "";
  return `${action}部分完成：匹配 ${result.matched}/${result.requested} 条；未找到 ID：${preview}${remainder}`;
}
