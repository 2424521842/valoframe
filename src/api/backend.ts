import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { mockClips, mockSourceDirs, mockTags } from "../data/mockData.ts";
import { TAG_COLORS } from "../lib/tags.ts";
import {
  matchesVideoType,
  VIDEO_TYPE_FILTERS,
} from "../lib/videoTypes.ts";
import type {
  BackendBatchMutationResult,
  BackendClip,
  BackendClipDetail,
  BackendClipEvent,
  BackendClipMedia,
  BackendClipPage,
  BackendClipSummary,
  BackendExportClipsResult,
  BackendPermanentDeleteResult,
  BackendRemoveClipsFromIndexResult,
  BackendReviewClipPage,
  BackendReviewClipState,
  BackendReviewDecisionMutation,
  BackendRelocateScanSourceResult,
  BackendRegisterScanSourceResult,
  BackendScanSourceRelocationPreview,
  BackendSource,
  BackendTag,
  BackendThumbnailProgress,
  BatchMutationResult,
  CancelScanResult,
  Clip,
  ClipDetail,
  ClipListQuery,
  ClipPage,
  ClipSummary,
  ClipEvent,
  ExportClipsResult,
  ClipMedia,
  FullDriveScanResult,
  LibraryFacets,
  LibraryFacetValue,
  PermanentDeleteResult,
  RemoveClipsFromIndexResult,
  RelocateScanSourceResult,
  RegisterScanSourceInput,
  RegisterScanSourceResult,
  ReviewClipPage,
  ReviewClipState,
  ReviewDecision,
  ReviewDecisionMutation,
  ReviewQueueQuery,
  ScanJobResult,
  ScanProgress,
  ScanStatus,
  ScanSummary,
  ScanSourceRelocationPreview,
  SourceDir,
  Tag,
  TagColor,
  ThumbnailEnqueueResult,
  ThumbnailProgress,
  ThumbnailQueueStatus,
} from "../types";
export {
  DEFAULT_ACLOS_MISSING_MESSAGE,
  isDefaultAclosDirMissing,
} from "../lib/scanSummary.ts";

const THUMBNAIL_TONES = ["red", "teal", "gold", "blue", "green"] as const;
const CLIP_MEDIA_PROTOCOL = "clip-media";
export const THUMBNAIL_ENQUEUE_LIMIT = 200;
const browserPreviewClipStore = new Map<string, Clip>(
  mockClips.map((clip) => [
    clip.id,
    { ...clip, tags: [...clip.tags], clipEvents: [...(clip.clipEvents ?? [])] },
  ]),
);
const browserPreviewTagStore = new Map<string, Tag>(
  mockTags.map((tag) => [tag.id, { ...tag }]),
);
const browserPreviewSourceStore = new Map<string, SourceDir>(
  mockSourceDirs.map((source) => [source.id, { ...source }]),
);
let browserPreviewTagSequence = 1;
let browserPreviewScanSequence = 1;
const browserPreviewScanSummaries = new Map<string, ScanSummary>();
let browserPreviewSourceSequence = Math.max(
  0,
  ...mockSourceDirs.map((source) => Number(source.id) || 0),
) + 1;
const AGENT_DISPLAY_NAMES: Record<string, string> = {
  brimstone: "炼狱",
  viper: "蝰蛇",
  omen: "幽影",
  killjoy: "奇乐",
  cypher: "零",
  sova: "猎枭",
  sage: "贤者",
  phoenix: "不死鸟",
  jett: "捷风",
  reyna: "芮娜",
  raze: "雷兹",
  breach: "铁臂",
  skye: "斯凯",
  yoru: "夜露",
  astra: "星礈",
  "kay/o": "K/O",
  kayo: "K/O",
  chamber: "尚勃勒",
  neon: "霓虹",
  fade: "黑梦",
  harbor: "海神",
  gekko: "盖可",
  deadlock: "钢锁",
  iso: "壹决",
  clove: "暮蝶",
  vyse: "维斯",
  tejo: "钛狐",
  waylay: "幻棱",
  miks: "迷核",
  veto: "禁灭",
  vampire: "芮娜",
  nox: "维斯",
  deadeye: "尚勃勒",
};
const LOCALIZED_AGENT_NAMES = new Set(Object.values(AGENT_DISPLAY_NAMES));
const DUPLICATE_SUFFIX_OFFICIAL_TITLES = new Set([
  "三杀时刻",
  "四杀时刻",
  "五杀时刻",
  "六杀时刻",
  "精准预判",
  "道具大师",
  "击杀合集",
  "击杀集锦",
  "死亡集锦",
  "死亡时刻",
  "残血反击",
]);

export type HighlightTitleInput = {
  officialVideoName?: string | null;
  officialVideoType?: string | null;
  killCount?: number | null;
  highlightType?: number | null;
  kda?: string | null;
};

export function displayHighlightTitle(clip: HighlightTitleInput): string {
  const officialName = clip.officialVideoName?.trim();
  if (officialName) {
    return normalizeOfficialVideoName(officialName);
  }

  const officialTypeTitle = titleFromOfficialType(
    clip.highlightType ?? parseHighlightType(clip.officialVideoType),
    clip.killCount,
  );
  if (officialTypeTitle) {
    return officialTypeTitle;
  }

  return clip.kda?.trim() ? "精彩击杀" : "高光时刻";
}

function normalizeOfficialVideoName(name: string): string {
  const duplicateSuffix = name.match(/^(?<title>.*?)(?<sequence>[1-9]\d*)$/u);
  const title = duplicateSuffix?.groups?.title;
  const sequence = Number(duplicateSuffix?.groups?.sequence);
  const normalizedName =
    title && sequence >= 2 && DUPLICATE_SUFFIX_OFFICIAL_TITLES.has(title) ? title : name;

  if (normalizedName === "击杀合集") return "击杀集锦";
  if (normalizedName === "死亡集锦") return "死亡时刻";
  return normalizedName;
}

function parseHighlightType(value: string | null | undefined): number | null {
  if (!value?.trim()) {
    return null;
  }

  const parsed = Number(value);
  return Number.isInteger(parsed) ? parsed : null;
}

function titleFromOfficialType(
  highlightType: number | null | undefined,
  killCount: number | null | undefined,
): string | null {
  if (highlightType === 2) {
    return "击杀集锦";
  }
  if (highlightType === 3) {
    return "死亡时刻";
  }
  if (highlightType === 4) {
    return "三杀时刻";
  }
  if (highlightType === 6) {
    return "四杀时刻";
  }
  if (highlightType === 10 && killCount === 5) {
    return "五杀时刻";
  }
  if (highlightType === 10 && killCount === 6) {
    return "六杀时刻";
  }

  return null;
}

/** Legacy all-at-once adapter retained until task 06B switches the production controller. */
export async function listClips(): Promise<Clip[]> {
  try {
    const clips = await invoke<BackendClip[]>("list_clips");
    return clips.map(mapBackendClip);
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return [...browserPreviewClipStore.values()];
    }

    throw error;
  }
}

/** Paginated production list contract mapped to detail-free UI summaries. */
export async function listClipPage(
  query: ClipListQuery = {},
): Promise<ClipPage> {
  try {
    const page = await invoke<BackendClipPage>("list_clip_page", { query });
    return {
      ...page,
      items: page.items.map(mapBackendClipSummary),
    };
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return browserPreviewClipPage(query);
    }

    throw error;
  }
}

/** Stable, cursor-based queue used exclusively by the quick-review workspace. */
export async function listReviewClipPage(
  query: ReviewQueueQuery = {},
): Promise<ReviewClipPage> {
  try {
    const page = await invoke<BackendReviewClipPage>("list_review_clip_page", { query });
    return {
      ...page,
      items: page.items.map(mapBackendClipSummary),
    };
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return browserPreviewReviewClipPage(query);
    }

    throw error;
  }
}

/** Exact whole-index facets; this request is intentionally independent from list pagination. */
export async function getLibraryFacets(): Promise<LibraryFacets> {
  try {
    return await invoke<LibraryFacets>("get_library_facets");
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return browserPreviewLibraryFacets();
    }

    throw error;
  }
}

/** Full detail is fetched only after a clip is selected; callers must ignore stale promises. */
export async function getClipDetail(
  clipId: string,
): Promise<ClipDetail> {
  if (isBrowserPreviewRuntime()) {
    const clip = browserPreviewClipStore.get(clipId);
    if (!clip) {
      throw {
        code: "clip-not-found",
        message: `素材不存在：${clipId}`,
        clipId,
      };
    }
    return cloneClip(clip);
  }

  try {
    const detail = await invoke<BackendClipDetail>("get_clip_detail", {
      clipId: numericClipId(clipId),
    });
    return mapBackendClip(detail);
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      const clip = browserPreviewClipStore.get(clipId);
      if (!clip) {
        throw {
          code: "clip-not-found",
          message: `素材不存在：${clipId}`,
          clipId: numericClipId(clipId),
        };
      }
      return cloneClip(clip);
    }

    throw error;
  }
}

export async function listSources(): Promise<SourceDir[]> {
  try {
    const sources = await invoke<BackendSource[]>("list_sources");
    return sources.map(mapBackendSource);
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return [...browserPreviewSourceStore.values()].map((source) => ({ ...source }));
    }

    throw error;
  }
}

export async function registerScanSource(
  input: RegisterScanSourceInput,
): Promise<RegisterScanSourceResult> {
  if (isBrowserPreviewRuntime()) {
    const normalizedRootPath = input.scanRootPath.trim().replace(/[\\/]+$/, "");
    const pathKey = normalizedRootPath.replace(/\\/g, "/").toLocaleLowerCase("en-US");
    const duplicate = [...browserPreviewSourceStore.values()].find(
      (source) => source.scanRootPath.replace(/\\/g, "/").replace(/\/+$/, "")
        .toLocaleLowerCase("en-US") === pathKey,
    );
    if (duplicate) {
      const updated = {
        ...duplicate,
        name: input.displayName.trim() || duplicate.name,
        displayName: input.displayName.trim() || duplicate.displayName,
        enabled: input.enabled,
      };
      browserPreviewSourceStore.set(updated.id, updated);
      return {
        sources: [{ ...updated }],
        createdCount: 0,
        duplicateCount: 1,
        normalizedRootPath,
        requiresOverlapConfirmation: false,
        overlaps: [],
      };
    }
    const id = String(browserPreviewSourceSequence++);
    const source: SourceDir = {
      id,
      name: input.displayName.trim(),
      displayName: input.displayName.trim(),
      path: normalizedRootPath,
      sourceKind: input.sourceKind,
      scanMode: input.sourceKind === "aclos" ? "aclos-structured" : "recursive-mp4",
      scanRootPath: normalizedRootPath,
      enabled: input.enabled,
      status: "pending",
      accessibility: false,
      lastError: null,
      clipCount: 0,
      lastScanAt: null,
    };
    browserPreviewSourceStore.set(id, source);
    return {
      sources: [{ ...source }],
      createdCount: 1,
      duplicateCount: 0,
      normalizedRootPath,
      requiresOverlapConfirmation: false,
      overlaps: [],
    };
  }
  const result = await invoke<BackendRegisterScanSourceResult>("register_scan_source", { input });
  return {
    ...result,
    sources: result.sources.map(mapBackendSource),
    overlaps: result.overlaps.map((overlap) => ({
      ...overlap,
      id: String(overlap.id),
    })),
  };
}

export async function previewScanSourceRelocation(
  sourceId: string,
  newRootPath: string,
): Promise<ScanSourceRelocationPreview> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewScanSourceRelocation(sourceId, newRootPath);
  }
  const preview = await invoke<BackendScanSourceRelocationPreview>(
    "preview_scan_source_relocation",
    {
      sourceId: numericSourceId(sourceId),
      newRootPath,
    },
  );
  return mapBackendScanSourceRelocationPreview(preview);
}

export async function relocateScanSource(
  sourceId: string,
  newRootPath: string,
): Promise<RelocateScanSourceResult> {
  if (isBrowserPreviewRuntime()) {
    const preview = browserPreviewScanSourceRelocation(sourceId, newRootPath);
    if (!preview.canRelocate) {
      throw new Error(preview.blockers[0]?.message ?? "当前来源无法重新定位");
    }
    const source = browserPreviewSourceStore.get(sourceId);
    if (!source) throw new Error(`Unknown preview source: ${sourceId}`);
    browserPreviewSourceStore.set(sourceId, {
      ...source,
      path: preview.newRootPath,
      scanRootPath: preview.newRootPath,
    });
    return {
      preview,
      relocatedClipCount: preview.expectedClipUpdateCount,
      syncJobId: null,
      syncStarted: false,
      syncStatus: null,
      syncMessage: null,
    };
  }
  const result = await invoke<BackendRelocateScanSourceResult>("relocate_scan_source", {
    sourceId: numericSourceId(sourceId),
    newRootPath,
  });
  return {
    ...result,
    preview: mapBackendScanSourceRelocationPreview(result.preview),
  };
}

export async function setScanSourceEnabled(
  sourceId: string,
  enabled: boolean,
): Promise<SourceDir> {
  if (isBrowserPreviewRuntime()) {
    const source = browserPreviewSourceStore.get(sourceId);
    if (!source) throw new Error(`Unknown preview source: ${sourceId}`);
    const updated = { ...source, enabled };
    browserPreviewSourceStore.set(sourceId, updated);
    return { ...updated };
  }
  return mapBackendSource(await invoke<BackendSource>("set_scan_source_enabled", {
    sourceId: numericSourceId(sourceId),
    enabled,
  }));
}

export async function listTags(): Promise<Tag[]> {
  try {
    const tags = await invoke<BackendTag[]>("list_tags");
    return tags.map(mapBackendTag);
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return [...browserPreviewTagStore.values()].map((tag) => ({ ...tag }));
    }

    throw error;
  }
}

export async function createTag(
  name: string,
  color: TagColor = "blue",
): Promise<Tag> {
  if (isBrowserPreviewRuntime()) {
    const normalizedName = name.trim();
    if (!normalizedName) {
      throw new Error("标签名称不能为空");
    }

    const existing = [...browserPreviewTagStore.values()].find(
      (tag) => tag.label === normalizedName,
    );
    if (existing) {
      return { ...existing };
    }

    const tag: Tag = {
      id: `preview-tag-${browserPreviewTagSequence++}`,
      label: normalizedName,
      color,
    };
    browserPreviewTagStore.set(tag.id, tag);
    return { ...tag };
  }
  const tag = await invoke<BackendTag>("create_tag", {
    name,
    color,
  });
  return mapBackendTag(tag);
}

export async function updateTag(
  tagId: string,
  name: string,
  color: TagColor,
): Promise<Tag> {
  if (isBrowserPreviewRuntime()) {
    const current = browserPreviewTagStore.get(tagId);
    if (!current) {
      throw new Error(`标签不存在：${tagId}`);
    }

    const normalizedName = name.trim();
    if (!normalizedName) {
      throw new Error("标签名称不能为空");
    }
    if (
      [...browserPreviewTagStore.values()].some(
        (tag) => tag.id !== tagId && tag.label === normalizedName,
      )
    ) {
      throw new Error(`标签名称已存在：${normalizedName}`);
    }

    const updated = { ...current, label: normalizedName, color };
    browserPreviewTagStore.set(tagId, updated);
    return { ...updated };
  }

  const tag = await invoke<BackendTag>("update_tag", {
    tagId: numericTagId(tagId),
    name,
    color,
  });
  return mapBackendTag(tag);
}

export async function deleteTag(tagId: string): Promise<void> {
  if (isBrowserPreviewRuntime()) {
    const current = browserPreviewTagStore.get(tagId);
    if (!current) {
      throw new Error(`标签不存在：${tagId}`);
    }
    browserPreviewTagStore.delete(tagId);
    for (const [clipId, clip] of browserPreviewClipStore) {
      if (clip.tags.includes(tagId)) {
        browserPreviewClipStore.set(clipId, {
          ...clip,
          tags: clip.tags.filter((candidate) => candidate !== tagId),
        });
      }
    }
    return;
  }

  await invoke("delete_tag", { tagId: numericTagId(tagId) });
}

export async function setClipFavorite(
  clipId: string,
  isFavorite: boolean,
): Promise<Clip> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => ({ ...clip, isFavorite }));
  }
  const clip = await invoke<BackendClip>("set_clip_favorite", {
    clipId: numericClipId(clipId),
    isFavorite,
  });
  return mapBackendClip(clip);
}

export async function setClipReviewDecision(
  clipId: string,
  decision: Exclude<ReviewDecision, "unreviewed">,
): Promise<ReviewDecisionMutation> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewSetReviewDecision(clipId, decision);
  }

  const result = await invoke<BackendReviewDecisionMutation>("set_clip_review_decision", {
    clipId: numericClipId(clipId),
    decision,
  });
  return mapBackendReviewDecisionMutation(result);
}

export async function restoreClipReviewState(
  clipId: string,
  expectedCurrent: ReviewClipState,
  restore: ReviewClipState,
): Promise<ReviewDecisionMutation> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewRestoreReviewState(clipId, expectedCurrent, restore);
  }

  const toBackendState = (state: ReviewClipState): BackendReviewClipState => ({
    ...state,
    clipId: numericClipId(state.clipId),
  });
  return mapBackendReviewDecisionMutation(
    await invoke<BackendReviewDecisionMutation>("restore_clip_review_state", {
      clipId: numericClipId(clipId),
      expectedCurrent: toBackendState(expectedCurrent),
      restore: toBackendState(restore),
    }),
  );
}

export async function resetClipReviewDecision(
  clipId: string,
): Promise<ReviewDecisionMutation> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewResetReviewDecision(clipId);
  }

  return mapBackendReviewDecisionMutation(
    await invoke<BackendReviewDecisionMutation>("reset_clip_review_decision", {
      clipId: numericClipId(clipId),
    }),
  );
}

export async function setClipsFavorite(
  clipIds: string[],
  isFavorite: boolean,
): Promise<BatchMutationResult> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewBatchMutation(clipIds, (clip) => ({
      clip: { ...clip, isFavorite },
      changed: clip.isFavorite !== isFavorite,
    }));
  }
  return mapBackendBatchMutationResult(
    await invoke<BackendBatchMutationResult>("set_clips_favorite", {
      clipIds: clipIds.map(numericClipId),
      isFavorite,
    }),
  );
}

export async function setClipTrashed(
  clipId: string,
  isTrashed: boolean,
): Promise<Clip> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => ({
      ...clip,
      fileStatus: isTrashed ? "trashed" : clip.isMissing ? "missing" : "available",
    }));
  }
  const clip = await invoke<BackendClip>("set_clip_trashed", {
    clipId: numericClipId(clipId),
    isTrashed,
  });
  return mapBackendClip(clip);
}

export async function setClipsTrashed(
  clipIds: string[],
  isTrashed: boolean,
): Promise<BatchMutationResult> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewBatchMutation(clipIds, (clip) => {
      const fileStatus = isTrashed
        ? "trashed"
        : clip.isMissing
          ? "missing"
          : "available";
      return {
        clip: { ...clip, fileStatus },
        changed: clip.fileStatus !== fileStatus,
      };
    });
  }
  return mapBackendBatchMutationResult(
    await invoke<BackendBatchMutationResult>("set_clips_trashed", {
      clipIds: clipIds.map(numericClipId),
      isTrashed,
    }),
  );
}

export async function removeClipFromIndex(clipId: string): Promise<void> {
  if (isBrowserPreviewRuntime()) {
    const result = browserPreviewRemoveClipsFromIndex([clipId]);
    if (result.removedIds.includes(clipId)) return;
    if (result.missingIds.includes(clipId)) throw new Error(`clip-not-found: 素材 ${clipId} 不存在`);
    const problem = result.blocked[0] ?? result.failures[0];
    throw new Error(problem ? `${problem.code}: ${problem.message}` : `无法仅移除素材 ${clipId} 的索引`);
  }
  await invoke("remove_clip_from_index", { clipId: numericClipId(clipId) });
}

export async function removeClipsFromIndex(
  clipIds: string[],
): Promise<RemoveClipsFromIndexResult> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewRemoveClipsFromIndex(clipIds);
  }
  const result = await invoke<BackendRemoveClipsFromIndexResult>(
    "remove_clips_from_index",
    { clipIds: clipIds.map(numericClipId) },
  );
  return {
    ...result,
    removedIds: result.removedIds.map(String),
    missingIds: result.missingIds.map(String),
    blocked: result.blocked.map((problem) => ({ ...problem, clipId: String(problem.clipId) })),
    failures: result.failures.map((problem) => ({ ...problem, clipId: String(problem.clipId) })),
  };
}

function browserPreviewRemoveClipsFromIndex(
  clipIds: string[],
): RemoveClipsFromIndexResult {
  const uniqueIds = [...new Set(clipIds)];
  if (uniqueIds.length > 200) {
    throw new Error("仅移除索引命令最多接受 200 个不同的素材 ID");
  }
  const result: RemoveClipsFromIndexResult = {
    requested: uniqueIds.length,
    removedIds: [],
    missingIds: [],
    blocked: [],
    failures: [],
  };
  for (const clipId of uniqueIds) {
    const clip = browserPreviewClipStore.get(clipId);
    if (!clip) {
      result.missingIds.push(clipId);
      continue;
    }
    const source = browserPreviewSourceStore.get(clip.sourceDirId);
    const eligible = clip.fileStatus === "missing"
      || clip.fileStatus === "trashed"
      || source?.status === "unavailable";
    if (!eligible) {
      result.blocked.push({
        clipId,
        code: source ? "index-removal-not-eligible" : "source-missing",
        message: source
          ? `素材 ${clipId} 仍可用且来源未明确失联`
          : `素材 ${clipId} 的来源记录不存在，无法验证仅移除索引资格`,
      });
      continue;
    }
    browserPreviewClipStore.delete(clipId);
    result.removedIds.push(clipId);
  }
  return result;
}

export async function exportClips(
  clipIds: string[],
  destinationDir: string,
): Promise<ExportClipsResult> {
  if (isBrowserPreviewRuntime()) {
    throw new Error("导出视频仅在桌面应用中可用");
  }

  const result = await invoke<BackendExportClipsResult>("export_clips", {
    clipIds: clipIds.map(numericClipId),
    destinationDir,
  });
  return {
    ...result,
    exportedIds: result.exportedIds.map(String),
    missingIds: result.missingIds.map(String),
    missingFileIds: result.missingFileIds.map(String),
    exports: result.exports.map((item) => ({
      ...item,
      clipId: String(item.clipId),
    })),
    failures: result.failures.map((failure) => ({
      ...failure,
      clipId: String(failure.clipId),
    })),
  };
}

export async function deleteClipsPermanently(
  clipIds: string[],
): Promise<PermanentDeleteResult> {
  if (isBrowserPreviewRuntime()) {
    const uniqueIds = [...new Set(clipIds)];
    const deletedIds: string[] = [];
    const missingIds: string[] = [];
    const pendingIds: string[] = [];
    const blocked: PermanentDeleteResult["blocked"] = [];
    const failures: PermanentDeleteResult["failures"] = [];
    for (const clipId of uniqueIds) {
      const clip = browserPreviewClipStore.get(clipId);
      if (!clip) {
        missingIds.push(clipId);
      } else if (clip.fileStatus !== "trashed") {
        failures.push({
          clipId,
          code: "not-trashed",
          retryable: false,
          message: "素材不在回收站，无法永久删除本地视频",
        });
      } else {
        browserPreviewClipStore.delete(clipId);
        deletedIds.push(clipId);
      }
    }
    return {
      requested: uniqueIds.length,
      deletedIds,
      missingIds,
      pendingIds,
      blocked,
      failures,
    };
  }

  const result = await invoke<BackendPermanentDeleteResult>(
    "delete_clips_permanently",
    { clipIds: clipIds.map(numericClipId) },
  );
  return {
    requested: result.requested,
    deletedIds: result.deletedIds.map(String),
    missingIds: result.missingIds.map(String),
    pendingIds: result.pendingIds.map(String),
    blocked: result.blocked.map((failure) => ({
      clipId: String(failure.clipId),
      code: failure.code,
      retryable: failure.retryable,
      message: failure.message,
    })),
    failures: result.failures.map((failure) => ({
      clipId: String(failure.clipId),
      code: failure.code,
      retryable: failure.retryable,
      message: failure.message,
    })),
  };
}

export async function updateClipNote(
  clipId: string,
  note: string,
): Promise<Clip> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => ({ ...clip, note }));
  }
  const clip = await invoke<BackendClip>("update_clip_note", {
    clipId: numericClipId(clipId),
    note,
  });
  return mapBackendClip(clip);
}

export async function addTagToClip(clipId: string, tagId: string): Promise<Clip> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => ({
      ...clip,
      tags: clip.tags.includes(tagId) ? clip.tags : [...clip.tags, tagId],
    }));
  }
  const clip = await invoke<BackendClip>("add_tag_to_clip", {
    clipId: numericClipId(clipId),
    tagId: numericTagId(tagId),
  });
  return mapBackendClip(clip);
}

export async function addTagToClips(
  clipIds: string[],
  tagId: string,
): Promise<BatchMutationResult> {
  if (isBrowserPreviewRuntime()) {
    requireBrowserPreviewTag(tagId);
    return browserPreviewBatchMutation(clipIds, (clip) => ({
      clip: clip.tags.includes(tagId)
        ? clip
        : { ...clip, tags: [...clip.tags, tagId] },
      changed: !clip.tags.includes(tagId),
    }));
  }
  return mapBackendBatchMutationResult(
    await invoke<BackendBatchMutationResult>("add_tag_to_clips", {
      clipIds: clipIds.map(numericClipId),
      tagId: numericTagId(tagId),
    }),
  );
}

export async function removeTagFromClip(
  clipId: string,
  tagId: string,
): Promise<Clip> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => ({
      ...clip,
      tags: clip.tags.filter((candidate) => candidate !== tagId),
    }));
  }
  const clip = await invoke<BackendClip>("remove_tag_from_clip", {
    clipId: numericClipId(clipId),
    tagId: numericTagId(tagId),
  });
  return mapBackendClip(clip);
}

export async function removeTagFromClips(
  clipIds: string[],
  tagId: string,
): Promise<BatchMutationResult> {
  if (isBrowserPreviewRuntime()) {
    requireBrowserPreviewTag(tagId);
    return browserPreviewBatchMutation(clipIds, (clip) => ({
      clip: clip.tags.includes(tagId)
        ? { ...clip, tags: clip.tags.filter((candidate) => candidate !== tagId) }
        : clip,
      changed: clip.tags.includes(tagId),
    }));
  }
  return mapBackendBatchMutationResult(
    await invoke<BackendBatchMutationResult>("remove_tag_from_clips", {
      clipIds: clipIds.map(numericClipId),
      tagId: numericTagId(tagId),
    }),
  );
}

export async function scanDefaultAclosDir(): Promise<ScanJobResult<ScanSummary>> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewScanJob("%APPDATA%\\ACLOS\\aclos-highlight");
  }
  return invoke<ScanJobResult<ScanSummary>>("scan_default_aclos_dir");
}

export async function scanCustomDir(path: string): Promise<ScanJobResult<ScanSummary>> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewScanJob(path);
  }
  return invoke<ScanJobResult<ScanSummary>>("scan_custom_dir", { path });
}

export async function scanRoots(paths: string[]): Promise<ScanJobResult<ScanSummary>> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewScanJob(paths.join("; "));
  }
  return invoke<ScanJobResult<ScanSummary>>("scan_roots", { paths });
}

export async function syncScanSource(
  sourceId: string,
): Promise<ScanJobResult<ScanSummary>> {
  if (isBrowserPreviewRuntime()) {
    const source = browserPreviewSourceStore.get(sourceId);
    if (!source) throw new Error(`Unknown preview source: ${sourceId}`);
    return browserPreviewScanJob(source.scanRootPath);
  }
  return invoke<ScanJobResult<ScanSummary>>("sync_scan_source", {
    sourceId: numericSourceId(sourceId),
  });
}

export async function syncEnabledSources(): Promise<ScanJobResult<ScanSummary>> {
  if (isBrowserPreviewRuntime()) {
    const roots = [...browserPreviewSourceStore.values()]
      .filter((source) => source.enabled)
      .map((source) => source.scanRootPath);
    return browserPreviewScanJob(roots.join("; "));
  }
  return invoke<ScanJobResult<ScanSummary>>("sync_enabled_sources");
}

export async function requestStartupSourceSync(): Promise<void> {
  if (isBrowserPreviewRuntime()) return;
  return invoke<void>("request_startup_source_sync");
}

export async function discoverAndScanFixedDrives(): Promise<ScanJobResult<FullDriveScanResult>> {
  if (isBrowserPreviewRuntime()) {
    const scanSummary = await browserPreviewScanSummary("本机固定磁盘");
    return browserPreviewJobResult({
      fixedDriveCount: 2,
      visitedDirectoryCount: 1842,
      validatedSourceDirCount: scanSummary.sourceDirCount,
      scanRootCount: 2,
      skippedDirectoryCount: 14,
      discoveryWarnings: [],
      scannedClipCount: mockClips.length,
      scanSummary,
    });
  }
  return invoke<ScanJobResult<FullDriveScanResult>>("discover_and_scan_fixed_drives");
}

export async function getScanStatus(): Promise<ScanStatus> {
  if (isBrowserPreviewRuntime()) {
    return {
      jobId: null,
      phase: null,
      currentRoot: null,
      source: null,
      processed: 0,
      total: null,
      message: "当前没有扫描任务",
      terminal: false,
      status: "idle",
    };
  }
  return invoke<ScanStatus>("get_scan_status");
}

export async function getScanSummary(jobId?: string): Promise<ScanSummary | null> {
  if (isBrowserPreviewRuntime()) {
    if (jobId) return browserPreviewScanSummaries.get(jobId) ?? null;
    const jobIds = [...browserPreviewScanSummaries.keys()];
    const latestJobId = jobIds[jobIds.length - 1];
    return latestJobId ? browserPreviewScanSummaries.get(latestJobId) ?? null : null;
  }
  return invoke<ScanSummary | null>("get_scan_summary", {
    jobId: jobId ?? null,
  });
}

export async function cancelScan(jobId: string): Promise<CancelScanResult> {
  if (isBrowserPreviewRuntime()) {
    return {
      accepted: false,
      reason: "not-running",
      jobId,
      activeJobId: null,
      status: "idle",
      message: "当前没有扫描任务",
    };
  }
  return invoke<CancelScanResult>("cancel_scan", { jobId });
}

export async function listenToScanProgress(
  onProgress: (progress: ScanProgress) => void,
): Promise<() => void> {
  if (typeof window === "undefined" || isBrowserPreviewRuntime()) {
    return () => {};
  }

  return listen<ScanProgress>("scan-progress", (event) => {
    onProgress(event.payload);
  });
}

export async function ensureClipThumbnails(
  clipIds: readonly string[],
): Promise<ThumbnailEnqueueResult> {
  if (isBrowserPreviewRuntime() || clipIds.length === 0) {
    return emptyThumbnailEnqueueResult();
  }
  const numericIds = thumbnailCommandClipIds(clipIds);
  return invoke<ThumbnailEnqueueResult>("ensure_clip_thumbnails", {
    clipIds: numericIds,
  });
}

export async function retryClipThumbnails(
  clipIds: readonly string[],
): Promise<ThumbnailEnqueueResult> {
  if (isBrowserPreviewRuntime() || clipIds.length === 0) {
    return emptyThumbnailEnqueueResult();
  }
  const numericIds = thumbnailCommandClipIds(clipIds);
  return invoke<ThumbnailEnqueueResult>("retry_clip_thumbnails", {
    clipIds: numericIds,
  });
}

export async function getThumbnailStatus(): Promise<ThumbnailQueueStatus> {
  if (isBrowserPreviewRuntime()) {
    return emptyThumbnailQueueStatus();
  }
  return invoke<ThumbnailQueueStatus>("get_thumbnail_status");
}

export async function listenToThumbnailProgress(
  onProgress: (progress: ThumbnailProgress) => void,
): Promise<() => void> {
  if (typeof window === "undefined" || isBrowserPreviewRuntime()) {
    return () => {};
  }

  return listen<BackendThumbnailProgress>("thumbnail-progress", (event) => {
    onProgress({
      clipId: String(event.payload.clipId),
      status: event.payload.status,
      revision: event.payload.revision,
      errorCode: event.payload.errorCode,
    });
  });
}

export async function getClipMedia(clipId: string): Promise<ClipMedia> {
  let response: BackendClipMedia;

  if (isBrowserPreviewRuntime()) {
    return {
      clipId,
      playable: false,
      mediaUrl: null,
      message: "浏览器预览模式",
    };
  }

  try {
    response = await invoke<BackendClipMedia>("get_clip_media", {
      clipId: numericClipId(clipId),
    });
  } catch (error) {
    if (shouldUseBrowserPreviewFallback(error)) {
      return {
        clipId,
        playable: false,
        mediaUrl: null,
        message: "浏览器预览模式",
      };
    }

    throw error;
  }

  return {
    clipId: String(response.clipId),
    playable: response.playable,
    mediaUrl: response.mediaPath ? mediaUrlFromPath(response.mediaPath) : null,
    message: response.message,
  };
}

export async function openClipLocation(clipId: string): Promise<void> {
  if (isBrowserPreviewRuntime()) {
    browserPreviewClip(clipId, (clip) => clip);
    return;
  }
  await invoke("open_clip_location", {
    clipId: numericClipId(clipId),
  });
}

export async function openClipExternally(clipId: string): Promise<void> {
  if (isBrowserPreviewRuntime()) {
    browserPreviewClip(clipId, (clip) => clip);
    return;
  }
  await invoke("open_clip_externally", {
    clipId: numericClipId(clipId),
  });
}

export async function copyClipPath(clipId: string): Promise<string> {
  if (isBrowserPreviewRuntime()) {
    return browserPreviewClip(clipId, (clip) => clip).filePath;
  }
  return invoke<string>("copy_clip_path", {
    clipId: numericClipId(clipId),
  });
}

export function mediaPathForClipId(clipId: string): string {
  return `clip/${encodeURIComponent(clipId)}`;
}

export function coverPathForClipId(clipId: string): string {
  return `cover/${encodeURIComponent(clipId)}`;
}

export function coverUrlForClipId(
  clipId: string,
  revision: string | null | undefined = null,
): string {
  const url = mediaUrlFromPath(coverPathForClipId(clipId));
  const normalizedRevision = revision?.trim();
  return normalizedRevision
    ? `${url}?v=${encodeURIComponent(normalizedRevision)}`
    : url;
}

export function mediaUrlFromPath(mediaPath: string): string {
  if (typeof window === "undefined" || isBrowserPreviewRuntime()) {
    return mediaPath;
  }
  return convertFileSrc(mediaPath, CLIP_MEDIA_PROTOCOL);
}

export function mapBackendClip(clip: BackendClip): Clip {
  const sourceDirId = String(clip.sourceDirId);
  const sourceDisplayName = `来源 ${sourceDirId}`;
  const account = accountIdentityFromBackendClip(clip, sourceDisplayName);
  const modifiedAt = normalizeDateValue(clip.modifiedAt ?? clip.recordedAt);
  const matchStartedAt = normalizeOptionalDateValue(clip.matchStartedAt);
  const recordedAt = normalizeOptionalDateValue(clip.recordedAt);
  const clipEvents = (clip.clipEvents ?? []).map(mapBackendClipEvent);
  const thumbnailStatus = clip.thumbnailStatus?.trim() || (clip.coverPath ? "ready" : undefined);
  const thumbnailRevision = clip.thumbnailRevision?.trim() || null;
  const hasThumbnail = Boolean(clip.coverPath) || thumbnailStatus === "ready" || Boolean(thumbnailRevision);

  return {
    id: String(clip.id),
    fileName: clip.fileName,
    filePath: clip.videoPath,
    sourceDirId,
    sourceDirName: sourceDisplayName,
    sourceDirPath: clip.scanRootPath,
    sourceKind: clip.sourceKind,
    scanMode: clip.scanMode,
    scanRootPath: clip.scanRootPath,
    sourceRelativeDir: clip.sourceRelativeDir,
    clipGroupId: clip.clipGroupId === null ? null : String(clip.clipGroupId),
    clipGroupName: clip.clipGroupName?.trim() || clip.fileName,
    ...account,
    agentName: normalizeAgentName(clip.agentName),
    mapName: clip.mapName?.trim() || "",
    gameMode: clip.gameMode?.trim() || "",
    metadataStatus: clip.metadataStatus?.trim() || "not_found",
    matchId: clip.matchId?.trim() || "",
    matchAccountId: clip.matchAccountId?.trim() || "",
    scoreline: clip.scoreline?.trim() || "",
    kda: clip.kda?.trim() || "",
    agentAvatarUrl: clip.agentAvatarUrl?.trim() || "",
    roundLabel: clip.roundLabel?.trim() || "",
    weaponName: clip.weaponName?.trim() || "",
    killCount: clip.killCount ?? null,
    matchStartedAt,
    combatScore: clip.combatScore ?? null,
    hasWon: typeof clip.hasWon === "boolean" ? clip.hasWon : null,
    officialVideoName: clip.officialVideoName?.trim() || null,
    officialVideoType: clip.officialVideoType?.trim() || null,
    highlightType: clip.highlightType ?? null,
    roundScore: clip.roundScore ?? null,
    metadataSource: clip.metadataSource?.trim() || null,
    eventCount: clip.eventCount ?? 0,
    clipEvents,
    createdAt: matchStartedAt ?? recordedAt ?? normalizeDateValue(clip.modifiedAt),
    modifiedAt,
    sizeBytes: clip.fileSize,
    durationMs: clip.durationMs,
    isFavorite: clip.favorite,
    reviewDecision: clip.reviewDecision,
    reviewedAt: normalizeOptionalDateValue(clip.reviewedAt),
    isMissing: clip.status !== "available",
    fileStatus: clip.status || "available",
    tags: clip.tagIds.map(String),
    note: clip.note ?? "",
    extractedText: clip.extractedText ?? "",
    thumbnailTone:
      THUMBNAIL_TONES[Math.abs(clip.id) % THUMBNAIL_TONES.length],
    thumbnailUrl: hasThumbnail
      ? coverUrlForClipId(String(clip.id), thumbnailRevision)
      : null,
    thumbnailStatus,
    thumbnailRevision,
  };
}

export function mapBackendClipSummary(clip: BackendClipSummary): ClipSummary {
  const mapped = mapBackendClip({
    ...clip,
    normalizedPath: "",
    extension: "",
    note: null,
    extractedText: null,
    roundLabel: null,
    weaponName: null,
    eventCount: null,
    clipEvents: [],
  });

  return {
    ...toClipSummary(mapped),
    sourceDirName: clip.sourceDirName.trim() || mapped.sourceDirName,
    sourceDirPath: clip.sourceDirPath,
    accountSourceName: clip.sourceDirName.trim() || mapped.accountSourceName,
  };
}

export function toClipSummary(clip: Clip): ClipSummary {
  const summary = { ...clip };
  const detailFields = summary as Partial<Clip>;
  delete detailFields.roundLabel;
  delete detailFields.weaponName;
  delete detailFields.eventCount;
  delete detailFields.clipEvents;
  delete detailFields.note;
  delete detailFields.extractedText;
  return summary;
}

export function mapBackendBatchMutationResult(
  result: BackendBatchMutationResult,
): BatchMutationResult {
  return {
    requested: result.requested,
    matched: result.matched,
    updated: result.updated,
    missingIds: result.missingIds.map(String),
    clips: result.clips.map(mapBackendClip),
  };
}

export function mapBackendReviewDecisionMutation(
  result: BackendReviewDecisionMutation,
): ReviewDecisionMutation {
  return {
    changed: result.changed,
    before: mapBackendReviewClipState(result.before),
    after: mapBackendReviewClipState(result.after),
  };
}

function mapBackendReviewClipState(state: BackendReviewClipState): ReviewClipState {
  return {
    clipId: String(state.clipId),
    reviewDecision: state.reviewDecision,
    // This value is also an opaque CAS token for restore_clip_review_state.
    // Preserve the backend bytes instead of normalizing its timestamp shape.
    reviewedAt: state.reviewedAt,
    favorite: state.favorite,
  };
}

function mapBackendClipEvent(event: BackendClipEvent): ClipEvent {
  return {
    id: String(event.id),
    eventType: event.eventType?.trim() || "event",
    videoTimeMs: event.videoTimeMs ?? null,
    eventTime: event.eventTime?.trim() || null,
    roundId: event.roundId ?? null,
    playerName: event.playerName?.trim() || "",
    weaponName: event.weaponName?.trim() || "",
    killerName: event.killerName?.trim() || "",
    killedName: event.killedName?.trim() || "",
    killerIsMe: event.killerIsMe === true,
    killedIsMe: event.killedIsMe === true,
  };
}

function normalizeAgentName(value: string | null | undefined): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) {
    return "";
  }

  const directLocalizedName = AGENT_DISPLAY_NAMES[trimmed.toLowerCase()];
  if (directLocalizedName) {
    return directLocalizedName;
  }
  if (LOCALIZED_AGENT_NAMES.has(trimmed)) {
    return trimmed;
  }

  const pathTail = trimmed
    .replace(/\\/g, "/")
    .split("/")
    .pop()
    ?.split(".")
    .pop()
    ?.replace(/^Default__/, "")
    .replace(/_Primary(?:Asset|DataAsset)?(?:_C)?$/i, "")
    .replace(/_PC_C.*$/i, "")
    .split("_")[0]
    ?.trim();
  const lookupValue = pathTail || trimmed;
  const localizedName = AGENT_DISPLAY_NAMES[lookupValue.toLowerCase()];

  if (localizedName) {
    return localizedName;
  }

  return LOCALIZED_AGENT_NAMES.has(trimmed) ? trimmed : "";
}

export function mapBackendTag(tag: BackendTag): Tag {
  return {
    id: String(tag.id),
    label: tag.name,
    color: isTagColor(tag.color) ? tag.color : "blue",
  };
}

export function mapBackendSource(source: BackendSource): SourceDir {
  const displayName = source.displayName.trim() || `来源 ${source.id}`;

  return {
    id: String(source.id),
    name: displayName,
    displayName,
    path: source.path,
    sourceKind: source.sourceKind,
    scanMode: source.scanMode,
    scanRootPath: source.scanRootPath,
    enabled: source.enabled,
    status: source.status,
    accessibility: source.accessibility,
    lastError: source.lastError,
    clipCount: source.clipCount,
    lastScanAt: source.lastScanAt,
  };
}

export function mapBackendScanSourceRelocationPreview(
  preview: BackendScanSourceRelocationPreview,
): ScanSourceRelocationPreview {
  return {
    ...preview,
    sourceId: String(preview.sourceId),
    affectedSources: preview.affectedSources.map((source) => ({
      ...source,
      id: String(source.id),
    })),
  };
}

function browserPreviewScanSourceRelocation(
  sourceId: string,
  newRootPath: string,
): ScanSourceRelocationPreview {
  const source = browserPreviewSourceStore.get(sourceId);
  if (!source) throw new Error(`Unknown preview source: ${sourceId}`);
  const oldRootPath = trimDirectorySeparators(source.scanRootPath);
  const normalizedNewRootPath = trimDirectorySeparators(newRootPath.trim());
  if (!normalizedNewRootPath) throw new Error("请选择新的来源根目录");
  const exactPathMatchCount = source.clipCount;
  const sameRoot = oldRootPath.replace(/\\/g, "/").toLocaleLowerCase("en-US") ===
    normalizedNewRootPath.replace(/\\/g, "/").toLocaleLowerCase("en-US");
  const blockers = [] as ScanSourceRelocationPreview["blockers"];
  if (sameRoot) {
    blockers.push({ code: "same-root", message: "新目录与当前来源根目录相同" });
  }
  if (exactPathMatchCount === 0) {
    blockers.push({ code: "zero-trusted-matches", message: "新目录中没有可信匹配，不能提交重新定位" });
  }
  const newSourcePath = replacePreviewRoot(
    source.path,
    oldRootPath,
    normalizedNewRootPath,
  );
  return {
    sourceId,
    oldRootPath,
    newRootPath: normalizedNewRootPath,
    affectedSources: [{
      id: source.id,
      displayName: source.displayName,
      oldSourcePath: source.path,
      newSourcePath,
      clipCount: source.clipCount,
    }],
    exactPathMatchCount,
    identityMatchCount: 0,
    legacyFingerprintMatchCount: 0,
    unmatchedCount: 0,
    newCandidateCount: 0,
    expectedClipUpdateCount: exactPathMatchCount,
    expectedGroupUpdateCount: 0,
    expectedCoverUpdateCount: 0,
    expectedMetadataReferenceUpdateCount: 0,
    conflicts: [],
    blockers,
    canRelocate: blockers.length === 0,
  };
}

function trimDirectorySeparators(path: string): string {
  return path.replace(/[\\/]+$/, "");
}

function replacePreviewRoot(path: string, oldRoot: string, newRoot: string): string {
  const normalizedPath = path.replace(/\\/g, "/");
  const normalizedOldRoot = oldRoot.replace(/\\/g, "/");
  if (!normalizedPath.toLocaleLowerCase("en-US").startsWith(
    normalizedOldRoot.toLocaleLowerCase("en-US"),
  )) {
    return newRoot;
  }
  const suffix = normalizedPath.slice(normalizedOldRoot.length).replace(/^\/+/, "");
  const separator = newRoot.includes("\\") ? "\\" : "/";
  return suffix ? `${newRoot}${separator}${suffix.replace(/\//g, separator)}` : newRoot;
}

export function mergeClipsWithSources(
  clips: readonly Clip[],
  sources: readonly SourceDir[],
): Clip[] {
  const sourceById = new Map(sources.map((source) => [source.id, source]));

  return clips.map((clip) => {
    const source = sourceById.get(clip.sourceDirId);
    if (!source) {
      return clip;
    }

    const placeholderName = `来源 ${clip.sourceDirId}`;
    const shouldUseSourceForAccount =
      clip.accountIdentitySource === "source-dir" &&
      (!clip.accountDisplayName.trim() || clip.accountDisplayName === placeholderName);
    const accountDisplayName = shouldUseSourceForAccount
      ? source.displayName
      : clip.accountDisplayName;

    return {
      ...clip,
      sourceDirName: source.displayName,
      sourceDirPath: source.path,
      sourceKind: source.sourceKind,
      scanMode: source.scanMode,
      scanRootPath: source.scanRootPath,
      accountName: shouldUseSourceForAccount ? accountDisplayName : clip.accountName,
      accountDisplayName,
      accountSourceName: source.displayName,
    };
  });
}

export function commandErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") {
      return message;
    }
  }

  return String(error);
}

export function scanCommandErrorCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return null;
  }
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" ? code : null;
}

export function scanCommandErrorJobId(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("jobId" in error)) {
    return null;
  }
  const jobId = (error as { jobId?: unknown }).jobId;
  return typeof jobId === "string" ? jobId : null;
}

export function scanCommandErrorActiveJobId(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("activeJobId" in error)) {
    return null;
  }
  const activeJobId = (error as { activeJobId?: unknown }).activeJobId;
  return typeof activeJobId === "string" ? activeJobId : null;
}

function accountIdentityFromBackendClip(
  clip: BackendClip,
  sourceDisplayName: string,
): Pick<
  Clip,
  | "accountId"
  | "accountIdentitySource"
  | "openid"
  | "accountName"
  | "accountDisplayName"
  | "accountSourceName"
  | "accountDetectedBy"
  | "playerName"
> {
  const matchAccountId = cleanAccountLabel(clip.matchAccountId);
  const openid = cleanAccountLabel(clip.openid);
  const accountIdentitySource = isAccountIdentitySource(clip.accountIdentitySource)
    ? clip.accountIdentitySource
    : matchAccountId
      ? "match-account-id"
      : openid
        ? "openid"
        : "source-dir";
  const accountId = cleanAccountLabel(clip.accountIdentityKey) ||
    (matchAccountId
      ? `match-account-${matchAccountId}`
      : openid
        ? `match-account-${openid}`
        : `source-${clip.sourceDirId}`);
  const identityDisplayName = matchAccountId || openid;
  const accountDisplayName =
    cleanAccountLabel(clip.accountDisplayName) ||
    cleanAccountLabel(clip.accountName) ||
    cleanAccountLabel(clip.playerName) ||
    (identityDisplayName ? `账号 ${identityDisplayName}` : sourceDisplayName);
  const accountName = cleanAccountLabel(clip.accountName) || accountDisplayName;
  const playerName = cleanAccountLabel(clip.playerName) || accountName;

  return {
    accountId,
    accountIdentitySource,
    openid: openid || null,
    accountName,
    accountDisplayName,
    accountSourceName: sourceDisplayName,
    accountDetectedBy: accountIdentitySource === "source-dir" ? "source-dir" : "metadata",
    playerName,
  };
}

function cleanAccountLabel(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

function isAccountIdentitySource(
  value: string | null | undefined,
): value is Clip["accountIdentitySource"] {
  return value === "match-account-id" || value === "openid" || value === "source-dir";
}

function normalizeDateValue(value: string | null): string {
  if (!value) {
    return new Date(0).toISOString();
  }

  if (/^\d+$/.test(value)) {
    return new Date(Number(value) * 1000).toISOString();
  }

  const parsedDate = new Date(value);

  if (Number.isNaN(parsedDate.getTime())) {
    return new Date(0).toISOString();
  }

  return parsedDate.toISOString();
}

function normalizeOptionalDateValue(value: string | null | undefined): string | null {
  if (!value) {
    return null;
  }

  const normalized = normalizeDateValue(value);
  return normalized === new Date(0).toISOString() ? null : normalized;
}

function numericClipId(clipId: string): number {
  const parsedClipId = Number(clipId);

  if (!Number.isSafeInteger(parsedClipId)) {
    throw new Error(`Invalid clip id: ${clipId}`);
  }

  return parsedClipId;
}

function numericSourceId(sourceId: string): number {
  const parsedSourceId = Number(sourceId);

  if (!Number.isSafeInteger(parsedSourceId)) {
    throw new Error(`Invalid source id: ${sourceId}`);
  }

  return parsedSourceId;
}

function thumbnailCommandClipIds(clipIds: readonly string[]): number[] {
  if (clipIds.length > THUMBNAIL_ENQUEUE_LIMIT) {
    throw new Error(`一次最多处理 ${THUMBNAIL_ENQUEUE_LIMIT} 个缩略图`);
  }
  return clipIds.map(numericClipId);
}

function emptyThumbnailEnqueueResult(): ThumbnailEnqueueResult {
  return {
    requested: 0,
    queued: 0,
    alreadyQueued: 0,
    skipped: 0,
  };
}

function emptyThumbnailQueueStatus(): ThumbnailQueueStatus {
  return {
    generatorStatus: "unknown",
    pendingCount: 0,
    runningCount: 0,
    readyCount: 0,
    failedCount: 0,
    unavailableCount: 0,
    evictedCount: 0,
    cacheBytes: 0,
    processingClipId: null,
    lastErrorCode: null,
  };
}

function numericTagId(tagId: string): number {
  const parsedTagId = Number(tagId);

  if (!Number.isSafeInteger(parsedTagId)) {
    throw new Error(`Invalid tag id: ${tagId}`);
  }

  return parsedTagId;
}

function isTagColor(value: string | null): value is TagColor {
  return TAG_COLORS.some((color) => color === value);
}

function shouldUseBrowserPreviewFallback(error: unknown): boolean {
  return (
    isBrowserPreviewRuntime() &&
    commandErrorMessage(error).toLowerCase().includes("invoke")
  );
}

function isBrowserPreviewRuntime(): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  const tauriWindow = window as Window & {
    __TAURI_INTERNALS__?: unknown;
  };

  return !tauriWindow.__TAURI_INTERNALS__;
}

async function browserPreviewScanSummary(rootPath: string): Promise<ScanSummary> {
  await new Promise((resolve) => window.setTimeout(resolve, 320));
  return {
    rootPath,
    sourceDirCount: new Set(mockClips.map((clip) => clip.sourceDirId)).size,
    clipGroupCount: new Set(
      mockClips.map((clip) => clip.matchId || clip.clipGroupId || clip.id),
    ).size,
    newClipCount: 0,
    updatedClipCount: mockClips.length,
    missingClipCount: mockClips.filter((clip) => clip.isMissing).length,
    coverMissingCount: mockClips.filter((clip) => !clip.thumbnailUrl).length,
    metadataMatchCount: new Set(
      mockClips.map((clip) => clip.matchId || clip.clipGroupId || clip.id),
    ).size,
    metadataEnrichedClipCount: mockClips.filter(
      (clip) => clip.metadataStatus === "enriched",
    ).length,
    metadataEventCount: mockClips.reduce(
      (total, clip) => total + (clip.clipEvents?.length ?? 0),
      0,
    ),
    metadataWarningCount: 0,
    errors: [],
    message: "浏览器预览扫描完成",
  };
}

async function browserPreviewScanJob(
  rootPath: string,
): Promise<ScanJobResult<ScanSummary>> {
  return browserPreviewJobResult(await browserPreviewScanSummary(rootPath));
}

function browserPreviewJobResult<T>(result: T): ScanJobResult<T> {
  const jobId = `preview-scan-${browserPreviewScanSequence++}`;
  const summary = browserPreviewSummaryFromResult(result);
  if (summary) {
    browserPreviewScanSummaries.set(jobId, summary);
    while (browserPreviewScanSummaries.size > 32) {
      const oldestJobId = browserPreviewScanSummaries.keys().next().value;
      if (typeof oldestJobId !== "string") break;
      browserPreviewScanSummaries.delete(oldestJobId);
    }
  }
  return {
    jobId,
    status: "completed",
    result,
    message: "浏览器预览扫描完成",
  };
}

function browserPreviewSummaryFromResult<T>(result: T): ScanSummary | null {
  if (!result || typeof result !== "object") return null;
  if ("newClipCount" in result) return result as unknown as ScanSummary;
  if ("scanSummary" in result) {
    const scanSummary = (result as { scanSummary?: unknown }).scanSummary;
    return scanSummary && typeof scanSummary === "object"
      ? scanSummary as ScanSummary
      : null;
  }
  return null;
}

function browserPreviewClip(
  clipId: string,
  update: (clip: Clip) => Clip,
): Clip {
  const clip = browserPreviewClipStore.get(clipId);
  if (!clip) throw new Error(`Unknown preview clip: ${clipId}`);
  const updated = update(clip);
  browserPreviewClipStore.set(clipId, updated);
  return updated;
}

function browserPreviewSetReviewDecision(
  clipId: string,
  decision: Exclude<ReviewDecision, "unreviewed">,
): ReviewDecisionMutation {
  const clip = browserPreviewClipStore.get(clipId);
  if (!clip) throw new Error(`Unknown preview clip: ${clipId}`);

  const before = browserPreviewReviewState(clip);
  if (before.reviewDecision === decision) {
    return { before, after: before, changed: false };
  }
  const updated = {
    ...clip,
    reviewDecision: decision,
    reviewedAt: new Date().toISOString(),
    isFavorite: decision === "liked" ? true : clip.isFavorite,
  };
  browserPreviewClipStore.set(clipId, updated);
  const after = browserPreviewReviewState(updated);
  return {
    before,
    after,
    changed:
      before.reviewDecision !== after.reviewDecision ||
      before.reviewedAt !== after.reviewedAt ||
      before.favorite !== after.favorite,
  };
}

function browserPreviewRestoreReviewState(
  clipId: string,
  expectedCurrent: ReviewClipState,
  restore: ReviewClipState,
): ReviewDecisionMutation {
  const clip = browserPreviewClipStore.get(clipId);
  if (!clip) throw new Error(`Unknown preview clip: ${clipId}`);
  if (expectedCurrent.clipId !== clipId || restore.clipId !== clipId) {
    throw new Error("撤销状态与素材不匹配");
  }
  const before = browserPreviewReviewState(clip);
  if (!browserPreviewReviewStatesEqual(before, expectedCurrent)) {
    throw new Error("素材状态已经变化，无法安全撤销");
  }
  const updated = {
    ...clip,
    reviewDecision: restore.reviewDecision,
    reviewedAt: restore.reviewedAt,
    isFavorite: restore.favorite,
  };
  browserPreviewClipStore.set(clipId, updated);
  const after = browserPreviewReviewState(updated);
  return {
    before,
    after,
    changed: !browserPreviewReviewStatesEqual(before, after),
  };
}

function browserPreviewResetReviewDecision(clipId: string): ReviewDecisionMutation {
  const clip = browserPreviewClipStore.get(clipId);
  if (!clip) throw new Error(`Unknown preview clip: ${clipId}`);
  const before = browserPreviewReviewState(clip);
  const updated = { ...clip, reviewDecision: "unreviewed" as const, reviewedAt: null };
  browserPreviewClipStore.set(clipId, updated);
  const after = browserPreviewReviewState(updated);
  return {
    before,
    after,
    changed: before.reviewDecision !== "unreviewed" || before.reviewedAt !== null,
  };
}

function browserPreviewReviewState(clip: Clip): ReviewClipState {
  return {
    clipId: clip.id,
    reviewDecision: clip.reviewDecision,
    reviewedAt: clip.reviewedAt,
    favorite: clip.isFavorite,
  };
}

function browserPreviewReviewStatesEqual(
  left: ReviewClipState,
  right: ReviewClipState,
): boolean {
  return (
    left.clipId === right.clipId &&
    left.reviewDecision === right.reviewDecision &&
    left.reviewedAt === right.reviewedAt &&
    left.favorite === right.favorite
  );
}

function browserPreviewReviewClipPage(query: ReviewQueueQuery): ReviewClipPage {
  const limit = Math.max(1, Math.min(200, query.limit ?? 3));
  const snapshotMaxClipId = query.snapshotMaxClipId ?? browserPreviewClipStore.size;
  const sourceIds = new Set((query.sourceDirIds ?? []).map(String));
  const tagIds = new Set((query.tagIds ?? []).map(String));
  const cursor = decodeBrowserReviewCursor(query.cursor, query, snapshotMaxClipId);
  const allCandidates = [...browserPreviewClipStore.values()]
    .map((clip) => ({
      clip,
      ordinal: browserPreviewClipOrdinal(clip.id),
      recordedAt: browserPreviewRecordedAt(clip),
    }))
    .filter(({ clip, ordinal, recordedAt }) => {
      const fileStatus = clip.fileStatus || (clip.isMissing ? "missing" : "available");
      return (
        ordinal <= snapshotMaxClipId &&
        fileStatus === "available" &&
        clip.reviewDecision === "unreviewed" &&
        (!query.accountId || query.accountId === "all" || clip.accountId === query.accountId) &&
        (!query.agentName || query.agentName === "all" || clip.agentName === query.agentName) &&
        (!query.mapName || query.mapName === "all" || clip.mapName === query.mapName) &&
        (!query.gameMode || query.gameMode === "all" || clip.gameMode === query.gameMode) &&
        (sourceIds.size === 0 || sourceIds.has(clip.sourceDirId)) &&
        (tagIds.size === 0 || clip.tags.some((tagId) => tagIds.has(tagId))) &&
        (query.recordedFrom === undefined || recordedAt >= query.recordedFrom) &&
        (query.recordedTo === undefined || recordedAt <= query.recordedTo)
      );
    })
    .sort((left, right) => right.recordedAt - left.recordedAt || right.ordinal - left.ordinal);
  const afterCursor = cursor
    ? allCandidates.filter(
        (candidate) =>
          candidate.recordedAt < cursor.recordedAt ||
          (candidate.recordedAt === cursor.recordedAt && candidate.ordinal < cursor.ordinal),
      )
    : allCandidates;
  const selected = afterCursor.slice(0, limit);
  const last = selected[selected.length - 1];
  const hasMore = afterCursor.length > selected.length;

  return {
    items: selected.map(({ clip }) => toClipSummary(clip)),
    snapshotMaxClipId,
    candidateCount: allCandidates.length,
    limit,
    hasMore,
    nextCursor: hasMore && last
      ? encodeBrowserReviewCursor(last.recordedAt, last.ordinal, query, snapshotMaxClipId)
      : null,
  };
}

function browserPreviewClipOrdinal(clipId: string): number {
  let ordinal = 0;
  for (const key of browserPreviewClipStore.keys()) {
    ordinal += 1;
    if (key === clipId) return ordinal;
  }
  return 0;
}

function browserPreviewRecordedAt(clip: Clip): number {
  const timestamp = new Date(clip.createdAt || clip.modifiedAt).getTime();
  return Number.isFinite(timestamp) ? Math.floor(timestamp / 1_000) : 0;
}

function encodeBrowserReviewCursor(
  recordedAt: number,
  ordinal: number,
  query: ReviewQueueQuery,
  snapshotMaxClipId: number,
): string {
  return `browser:${snapshotMaxClipId}:${browserReviewQuerySignature(query)}:${recordedAt}:${ordinal}`;
}

function decodeBrowserReviewCursor(
  value: string | undefined,
  query: ReviewQueueQuery,
  snapshotMaxClipId: number,
): { recordedAt: number; ordinal: number } | null {
  if (!value) return null;
  const match = /^browser:(\d+):([0-9a-f]+):(-?\d+):(\d+)$/.exec(value);
  if (!match) throw new Error("无效的旧版挑片游标");
  if (
    Number(match[1]) !== snapshotMaxClipId ||
    match[2] !== browserReviewQuerySignature(query)
  ) {
    throw new Error("旧版挑片游标与当前快照或筛选条件不匹配");
  }
  return { recordedAt: Number(match[3]), ordinal: Number(match[4]) };
}

function browserReviewQuerySignature(query: ReviewQueueQuery): string {
  const normalized = JSON.stringify({
    accountId: query.accountId ?? null,
    agentName: query.agentName ?? null,
    mapName: query.mapName ?? null,
    gameMode: query.gameMode ?? null,
    sourceDirIds: [...(query.sourceDirIds ?? [])].sort((left, right) => left - right),
    tagIds: [...(query.tagIds ?? [])].sort((left, right) => left - right),
    recordedFrom: query.recordedFrom ?? null,
    recordedTo: query.recordedTo ?? null,
  });
  let hash = 2_166_136_261;
  for (let index = 0; index < normalized.length; index += 1) {
    hash ^= normalized.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }
  return (hash >>> 0).toString(16);
}

function browserPreviewClipPage(query: ClipListQuery): ClipPage {
  const offset = query.offset ?? 0;
  const limit = query.limit ?? 50;
  const normalizedQuery = query.query?.trim().toLocaleLowerCase("zh-CN") ?? "";
  const items = [...browserPreviewClipStore.values()]
    .filter((clip) => {
      const searchable = [
        clip.fileName,
        clip.filePath,
        clip.sourceDirName,
        clip.accountDisplayName,
        clip.playerName,
        clip.agentName,
        clip.mapName,
        clip.gameMode,
        clip.scoreline,
        clip.kda,
        clip.note,
        clip.extractedText,
        ...clip.tags.map((tagId) => browserPreviewTagStore.get(tagId)?.label ?? tagId),
      ].join(" ").toLocaleLowerCase("zh-CN");
      const modifiedAt = Math.floor(new Date(clip.modifiedAt).getTime() / 1000);
      const fileStatus = clip.fileStatus || (clip.isMissing ? "missing" : "available");
      return (
        (!normalizedQuery || searchable.includes(normalizedQuery)) &&
        (!query.accountId || query.accountId === "all" || clip.accountId === query.accountId) &&
        (query.sourceDirId === undefined || clip.sourceDirId === String(query.sourceDirId)) &&
        (!query.agentName || query.agentName === "all" || clip.agentName === query.agentName) &&
        (!query.mapName || query.mapName === "all" || clip.mapName === query.mapName) &&
        (!query.gameMode || query.gameMode === "all" || clip.gameMode === query.gameMode) &&
        (query.tagId === undefined || clip.tags.includes(String(query.tagId))) &&
        browserPreviewMatchesHighlight(clip, query.highlightFilter) &&
        (query.favoriteFilter !== "favorite" || clip.isFavorite) &&
        (query.favoriteFilter !== "not-favorite" || !clip.isFavorite) &&
        (!query.reviewDecision || clip.reviewDecision === query.reviewDecision) &&
        (query.fileStatus ? fileStatus === query.fileStatus : fileStatus !== "trashed") &&
        (!query.metadataStatus || clip.metadataStatus === query.metadataStatus) &&
        (query.modifiedFrom === undefined || modifiedAt >= query.modifiedFrom) &&
        (query.modifiedTo === undefined || modifiedAt <= query.modifiedTo) &&
        (query.sizeMinBytes === undefined || clip.sizeBytes >= query.sizeMinBytes) &&
        (query.sizeMaxBytes === undefined || clip.sizeBytes <= query.sizeMaxBytes)
      );
    })
    .sort((left, right) => compareBrowserPreviewClips(left, right, query.sortBy));
  const pageItems = items.slice(offset, offset + limit).map(toClipSummary);
  const nextOffset = offset + pageItems.length;

  return {
    items: pageItems,
    offset,
    limit,
    totalCount: items.length,
    hasMore: nextOffset < items.length,
    nextOffset: nextOffset < items.length ? nextOffset : null,
  };
}

function browserPreviewLibraryFacets(): LibraryFacets {
  const clips = [...browserPreviewClipStore.values()];
  const isActive = (clip: Clip) => previewFileStatus(clip) !== "trashed";
  const fileStatuses = browserPreviewValueFacets(
    clips,
    (clip) => previewFileStatus(clip),
  );
  const metadataStatuses = browserPreviewValueFacets(
    clips,
    (clip) => clip.metadataStatus?.trim() || "not_found",
  );
  const agents = browserPreviewValueFacets(clips, (clip) => clip.agentName.trim());
  const maps = browserPreviewValueFacets(clips, (clip) => clip.mapName.trim());
  const gameModes = browserPreviewValueFacets(clips, (clip) => clip.gameMode.trim());
  const killTypes = VIDEO_TYPE_FILTERS
    .map((value): LibraryFacetValue => ({
      value,
      count: clips.filter((clip) => browserPreviewMatchesHighlight(clip, value)).length,
      activeCount: clips.filter(
        (clip) => isActive(clip) && browserPreviewMatchesHighlight(clip, value),
      ).length,
    }))
    .filter((facet) => facet.count > 0);

  const accountsByKey = new Map<string, {
    facet: LibraryFacets["accounts"][number];
    displayTimestamp: number;
    displayClipId: string;
  }>();
  for (const clip of clips) {
    const key = clip.accountId.trim() || `source-${clip.sourceDirId}`;
    const displayName = clip.accountDisplayName.trim();
    const timestamp = previewUnixTimestamp(clip.modifiedAt) ?? 0;
    const existing = accountsByKey.get(key);
    if (!existing) {
      accountsByKey.set(key, {
        facet: {
          accountIdentityKey: key,
          accountDisplayName: displayName || key,
          count: 1,
          activeCount: isActive(clip) ? 1 : 0,
        },
        displayTimestamp: displayName ? timestamp : Number.NEGATIVE_INFINITY,
        displayClipId: clip.id,
      });
      continue;
    }
    existing.facet.count += 1;
    existing.facet.activeCount += isActive(clip) ? 1 : 0;
    if (
      displayName &&
      (timestamp > existing.displayTimestamp ||
        (timestamp === existing.displayTimestamp && clip.id > existing.displayClipId))
    ) {
      existing.facet.accountDisplayName = displayName;
      existing.displayTimestamp = timestamp;
      existing.displayClipId = clip.id;
    }
  }
  const accounts = [...accountsByKey.values()]
    .map(({ facet }) => facet)
    .sort((left, right) =>
      right.count - left.count ||
      left.accountDisplayName.localeCompare(right.accountDisplayName, "zh-CN") ||
      left.accountIdentityKey.localeCompare(right.accountIdentityKey),
    );

  const sourcesById = new Map<string, LibraryFacets["sourceDirs"][number]>();
  for (const clip of clips) {
    const sourceDirId = clip.sourceDirId;
    const existing = sourcesById.get(sourceDirId) ?? {
      sourceDirId,
      count: 0,
      activeCount: 0,
    };
    existing.count += 1;
    existing.activeCount += isActive(clip) ? 1 : 0;
    sourcesById.set(sourceDirId, existing);
  }
  const sourceDirs = [...sourcesById.values()].sort((left, right) =>
    right.count - left.count || String(left.sourceDirId).localeCompare(String(right.sourceDirId)),
  );

  const tagCounts = new Map<string, { count: number; activeCount: number }>();
  for (const clip of clips) {
    for (const tagId of new Set(clip.tags)) {
      const existing = tagCounts.get(tagId) ?? { count: 0, activeCount: 0 };
      existing.count += 1;
      existing.activeCount += isActive(clip) ? 1 : 0;
      tagCounts.set(tagId, existing);
    }
  }
  const tags = [...tagCounts].map(([id, counts]) => {
    const tag = browserPreviewTagStore.get(id);
    return {
      id,
      name: tag?.label ?? id,
      color: tag?.color ?? null,
      ...counts,
    };
  }).sort((left, right) =>
    right.count - left.count ||
    left.name.localeCompare(right.name, "zh-CN") ||
    String(left.id).localeCompare(String(right.id)),
  );

  const recordedTimestamps = clips
    .map((clip) => previewUnixTimestamp(clip.createdAt))
    .filter((value): value is number => value !== null);
  const modifiedTimestamps = clips
    .map((clip) => previewUnixTimestamp(clip.modifiedAt))
    .filter((value): value is number => value !== null);
  const activeClips = clips.filter(isActive);
  const sizeBytes = clips.map((clip) => clip.sizeBytes);

  return {
    totalCount: clips.length,
    activeCount: activeClips.length,
    favoriteCount: clips.filter((clip) => clip.isFavorite).length,
    activeFavoriteCount: activeClips.filter((clip) => clip.isFavorite).length,
    trashedCount: clips.filter((clip) => previewFileStatus(clip) === "trashed").length,
    taggedCount: clips.filter((clip) => new Set(clip.tags).size > 0).length,
    activeTaggedCount: activeClips.filter((clip) => new Set(clip.tags).size > 0).length,
    totalSizeBytes: clips.reduce((total, clip) => total + clip.sizeBytes, 0),
    activeSizeBytes: activeClips.reduce((total, clip) => total + clip.sizeBytes, 0),
    sizeBytesMin: previewTimestampBound(sizeBytes, "min"),
    sizeBytesMax: previewTimestampBound(sizeBytes, "max"),
    recentCount: activeClips.filter((clip) => previewIsToday(clip.modifiedAt)).length,
    recordedAtMin: previewTimestampBound(recordedTimestamps, "min"),
    recordedAtMax: previewTimestampBound(recordedTimestamps, "max"),
    modifiedAtMin: previewTimestampBound(modifiedTimestamps, "min"),
    modifiedAtMax: previewTimestampBound(modifiedTimestamps, "max"),
    fileStatuses,
    metadataStatuses,
    accounts,
    sourceDirs,
    agents,
    maps,
    gameModes,
    killTypes,
    tags,
  };
}

function browserPreviewValueFacets(
  clips: readonly Clip[],
  valueForClip: (clip: Clip) => string,
): LibraryFacetValue[] {
  const counts = new Map<string, LibraryFacetValue>();
  for (const clip of clips) {
    const value = valueForClip(clip).trim();
    if (!value) continue;
    const facet = counts.get(value) ?? { value, count: 0, activeCount: 0 };
    facet.count += 1;
    facet.activeCount += previewFileStatus(clip) === "trashed" ? 0 : 1;
    counts.set(value, facet);
  }
  return [...counts.values()].sort(compareLibraryFacetValues);
}

function compareLibraryFacetValues(left: LibraryFacetValue, right: LibraryFacetValue): number {
  return right.count - left.count ||
    left.value.localeCompare(right.value, "zh-CN") ||
    left.value.localeCompare(right.value);
}

function previewFileStatus(clip: Clip): string {
  return clip.fileStatus || (clip.isMissing ? "missing" : "available");
}

function previewUnixTimestamp(value: string): number | null {
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) ? Math.floor(timestamp / 1_000) : null;
}

function previewTimestampBound(values: readonly number[], kind: "min" | "max"): number | null {
  if (values.length === 0) return null;
  return kind === "min" ? Math.min(...values) : Math.max(...values);
}

function previewIsToday(value: string): boolean {
  const date = new Date(value);
  const today = new Date();
  return !Number.isNaN(date.getTime()) &&
    date.getFullYear() === today.getFullYear() &&
    date.getMonth() === today.getMonth() &&
    date.getDate() === today.getDate();
}

function browserPreviewMatchesHighlight(
  clip: Clip,
  filter: ClipListQuery["highlightFilter"],
): boolean {
  return matchesVideoType(clip, filter);
}

function compareBrowserPreviewClips(
  left: Clip,
  right: Clip,
  sortBy: ClipListQuery["sortBy"],
): number {
  const modifiedDelta = new Date(left.modifiedAt).getTime() - new Date(right.modifiedAt).getTime();
  const idDelta = left.id.localeCompare(right.id, "zh-CN", { numeric: true });
  if (sortBy === "modified-asc") return modifiedDelta || idDelta;
  if (sortBy === "size-desc") return right.sizeBytes - left.sizeBytes || idDelta;
  if (sortBy === "size-asc") return left.sizeBytes - right.sizeBytes || idDelta;
  if (sortBy === "name-asc") {
    return left.fileName.localeCompare(right.fileName, "zh-CN", { numeric: true }) || idDelta;
  }
  return -modifiedDelta || idDelta;
}

function cloneClip(clip: Clip): Clip {
  return {
    ...clip,
    tags: [...clip.tags],
    clipEvents: clip.clipEvents?.map((event) => ({ ...event })),
  };
}

function browserPreviewBatchMutation(
  clipIds: readonly string[],
  update: (clip: Clip) => { clip: Clip; changed: boolean },
): BatchMutationResult {
  const uniqueIds = [...new Set(clipIds)];
  const missingIds: string[] = [];
  const clips: Clip[] = [];
  let updated = 0;

  for (const clipId of uniqueIds) {
    const current = browserPreviewClipStore.get(clipId);
    if (!current) {
      missingIds.push(clipId);
      continue;
    }

    const mutation = update(current);
    if (mutation.changed) {
      browserPreviewClipStore.set(clipId, mutation.clip);
      updated += 1;
    }
    clips.push(mutation.clip);
  }

  return {
    requested: uniqueIds.length,
    matched: clips.length,
    updated,
    missingIds,
    clips,
  };
}

function requireBrowserPreviewTag(tagId: string): void {
  if (!browserPreviewTagStore.has(tagId)) {
    throw new Error(`标签不存在：${tagId}`);
  }
}
