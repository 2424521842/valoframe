import type {
  ClipListQuery,
  ClipSort,
  HighlightFilter,
  LibraryMode,
} from "../types";

export const CLIP_PAGE_SIZE = 50;
export const CLIP_SELECT_ALL_PAGE_SIZE = 200;
export const CLIP_SEARCH_DEBOUNCE_MS = 250;

export type ProductionClipQueryInput = {
  query: string;
  accountId: string;
  sourceDirId: string;
  agentName: string;
  mapName: string;
  gameMode: string;
  tagId: string;
  highlightFilter: HighlightFilter;
  fileStatus: string;
  libraryMode: LibraryMode;
  modifiedFrom?: string;
  modifiedTo?: string;
  sortBy: ClipSort;
};

/** Maps the complete production toolbar state to the 06A backend contract. */
export function buildClipListQuery(
  input: ProductionClipQueryInput,
  offset = 0,
  limit = CLIP_PAGE_SIZE,
): ClipListQuery {
  const query: ClipListQuery = { offset, limit, sortBy: input.sortBy };
  const search = normalizedFilter(input.query);
  const accountId = normalizedFilter(input.accountId);
  const agentName = normalizedFilter(input.agentName);
  const mapName = normalizedFilter(input.mapName);
  const gameMode = normalizedFilter(input.gameMode);
  const sourceDirId = numericFilterId(input.sourceDirId);
  const tagId = numericFilterId(input.tagId);
  const fileStatus = productionFileStatus(input.libraryMode, input.fileStatus);
  const modifiedFrom = inclusiveDateBoundary(input.modifiedFrom, "start");
  const modifiedTo = inclusiveDateBoundary(input.modifiedTo, "end");

  if (search) query.query = search;
  if (accountId) query.accountId = accountId;
  if (sourceDirId !== undefined) query.sourceDirId = sourceDirId;
  if (agentName) query.agentName = agentName;
  if (mapName) query.mapName = mapName;
  if (gameMode) query.gameMode = gameMode;
  if (tagId !== undefined) query.tagId = tagId;
  if (input.highlightFilter !== "all") {
    query.highlightFilter = input.highlightFilter;
  }
  if (input.libraryMode === "favorites") {
    query.favoriteFilter = "favorite";
  }
  if (fileStatus) query.fileStatus = fileStatus;
  if (modifiedFrom !== undefined) query.modifiedFrom = modifiedFrom;
  if (modifiedTo !== undefined) query.modifiedTo = modifiedTo;

  return query;
}

export function clipListQueryKey(query: ClipListQuery): string {
  const { offset: _offset, ...generationQuery } = query;
  void _offset;
  return JSON.stringify(generationQuery);
}

function productionFileStatus(libraryMode: LibraryMode, fileStatus: string): string | undefined {
  if (libraryMode === "missing") return "missing";
  if (libraryMode === "trash") return "trashed";
  return normalizedFilter(fileStatus);
}

function normalizedFilter(value: string | undefined): string | undefined {
  const normalized = value?.trim();
  return !normalized || normalized === "all" ? undefined : normalized;
}

function numericFilterId(value: string): number | undefined {
  const normalized = normalizedFilter(value);
  if (!normalized) return undefined;
  const parsed = Number(normalized);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : undefined;
}

function inclusiveDateBoundary(
  value: string | undefined,
  boundary: "start" | "end",
): number | undefined {
  const normalized = value?.trim();
  if (!normalized) return undefined;
  const date = /^\d{4}-\d{2}-\d{2}$/.test(normalized)
    ? new Date(`${normalized}T${boundary === "start" ? "00:00:00" : "23:59:59"}`)
    : new Date(normalized);
  const timestamp = date.getTime();
  return Number.isNaN(timestamp) ? undefined : Math.floor(timestamp / 1000);
}
