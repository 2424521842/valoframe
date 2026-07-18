import type {
  Clip,
  ClipSort,
  FavoriteFilter,
  HighlightFilter,
  Tag,
} from "../types";
import { matchesVideoType } from "./videoTypes";

export type ClipFilterOptions = {
  query: string;
  accountId?: string;
  sourceDirId: string;
  agentName?: string;
  mapName?: string;
  gameMode?: string;
  tagId: string;
  highlightFilter?: HighlightFilter;
  favoriteFilter?: FavoriteFilter;
  fileStatus?: string;
  modifiedFrom?: string;
  modifiedTo?: string;
  sizeMinBytes?: number | null;
  sizeMaxBytes?: number | null;
  tags?: Tag[];
  favoritesOnly?: boolean;
  missingOnly?: boolean;
  sortBy: ClipSort;
};

export function applyClipFilters(
  clips: Clip[],
  options: ClipFilterOptions,
): Clip[] {
  const query = options.query.trim().toLowerCase();
  const tagLabelById = new Map(
    (options.tags ?? []).map((tag) => [tag.id, tag.label]),
  );
  const modifiedFrom = startOfDayTimestamp(options.modifiedFrom);
  const modifiedTo = endOfDayTimestamp(options.modifiedTo);

  return clips
    .filter(
      (clip) =>
        matchesQuery(clip, query, tagLabelById) &&
        matchesAccount(clip, options.accountId) &&
        matchesSource(clip, options.sourceDirId) &&
        matchesExactValue(clip.agentName, options.agentName) &&
        matchesExactValue(clip.mapName, options.mapName) &&
        matchesExactValue(clip.gameMode, options.gameMode) &&
        matchesTag(clip, options.tagId) &&
        matchesVideoType(clip, options.highlightFilter) &&
        matchesFavorite(clip, options) &&
        matchesFileStatus(clip, options) &&
        matchesDateRange(clip, modifiedFrom, modifiedTo) &&
        matchesSizeRange(clip, options),
    )
    .slice()
    .sort((left, right) => compareClips(left, right, options.sortBy));
}

function matchesQuery(
  clip: Clip,
  query: string,
  tagLabelById: ReadonlyMap<string, string>,
): boolean {
  if (query.length === 0) {
    return true;
  }

  const searchableFields = [
    clip.fileName,
    clip.filePath,
    clip.sourceDirName,
    clip.accountDisplayName,
    clip.accountSourceName,
    clip.playerName,
    clip.agentName,
    clip.mapName,
    clip.gameMode,
    clip.scoreline,
    clip.kda,
    clip.note,
    clip.extractedText,
    ...clip.tags.map((tagId) => tagLabelById.get(tagId) ?? tagId),
  ];

  return searchableFields.some((field) =>
    field.toLowerCase().includes(query),
  );
}

function matchesAccount(clip: Clip, accountId: string | undefined): boolean {
  return !accountId || accountId === "all" || clip.accountId === accountId;
}

function matchesSource(clip: Clip, sourceDirId: string): boolean {
  return sourceDirId === "all" || clip.sourceDirId === sourceDirId;
}

function matchesExactValue(value: string, selectedValue: string | undefined): boolean {
  return !selectedValue || selectedValue === "all" || value === selectedValue;
}

function matchesTag(clip: Clip, tagId: string): boolean {
  return tagId === "all" || clip.tags.includes(tagId);
}

function matchesFavorite(clip: Clip, options: ClipFilterOptions): boolean {
  if (options.favoriteFilter === "favorite") {
    return clip.isFavorite;
  }

  if (options.favoriteFilter === "not-favorite") {
    return !clip.isFavorite;
  }

  return !options.favoritesOnly || clip.isFavorite;
}

function matchesFileStatus(clip: Clip, options: ClipFilterOptions): boolean {
  const fileStatus = clip.fileStatus || (clip.isMissing ? "missing" : "available");

  if (options.fileStatus && options.fileStatus !== "all") {
    return fileStatus === options.fileStatus;
  }

  if (fileStatus === "trashed") {
    return false;
  }

  return !options.missingOnly || fileStatus === "missing" || clip.isMissing;
}

function matchesDateRange(
  clip: Clip,
  modifiedFrom: number | null,
  modifiedTo: number | null,
): boolean {
  if (modifiedFrom === null && modifiedTo === null) {
    return true;
  }

  const modifiedAt = timestamp(clip.modifiedAt);

  if (Number.isNaN(modifiedAt)) {
    return false;
  }

  if (modifiedFrom !== null && modifiedAt < modifiedFrom) {
    return false;
  }

  return modifiedTo === null || modifiedAt <= modifiedTo;
}

function matchesSizeRange(clip: Clip, options: ClipFilterOptions): boolean {
  const minBytes = options.sizeMinBytes;
  const maxBytes = options.sizeMaxBytes;

  if (typeof minBytes === "number" && clip.sizeBytes < minBytes) {
    return false;
  }

  return !(typeof maxBytes === "number" && clip.sizeBytes > maxBytes);
}

function compareClips(left: Clip, right: Clip, sortBy: ClipSort): number {
  switch (sortBy) {
    case "modified-asc":
      return timestamp(left.modifiedAt) - timestamp(right.modifiedAt);
    case "size-desc":
      return right.sizeBytes - left.sizeBytes;
    case "size-asc":
      return left.sizeBytes - right.sizeBytes;
    case "name-asc":
      return left.fileName.localeCompare(right.fileName, "zh-CN", {
        numeric: true,
        sensitivity: "base",
      });
    case "modified-desc":
    default:
      return timestamp(right.modifiedAt) - timestamp(left.modifiedAt);
  }
}

function startOfDayTimestamp(value: string | undefined): number | null {
  return dateBoundaryTimestamp(value, "00:00:00.000");
}

function endOfDayTimestamp(value: string | undefined): number | null {
  return dateBoundaryTimestamp(value, "23:59:59.999");
}

function dateBoundaryTimestamp(
  value: string | undefined,
  timePart: string,
): number | null {
  const trimmedValue = value?.trim();

  if (!trimmedValue) {
    return null;
  }

  const timestampValue = timestamp(
    /^\d{4}-\d{2}-\d{2}$/.test(trimmedValue)
      ? `${trimmedValue}T${timePart}`
      : trimmedValue,
  );

  return Number.isNaN(timestampValue) ? null : timestampValue;
}

function timestamp(value: string): number {
  return new Date(value).getTime();
}
