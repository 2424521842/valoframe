import type { LibraryMode } from "../types";

export type ActiveFilterKey =
  | "mode"
  | "query"
  | "account"
  | "source"
  | "agent"
  | "map"
  | "game-mode"
  | "tag"
  | "file-status";

export type ActiveFilterDescriptor = {
  key: ActiveFilterKey;
  label: string;
};

export type ActiveFilterInput = {
  libraryMode: LibraryMode;
  query: string;
  accountId: string;
  accountLabel: string;
  sourceDirId: string;
  sourceDirLabel: string;
  agentName: string;
  mapName: string;
  gameMode: string;
  tagId: string;
  tagLabel: string;
  fileStatus: string;
};

const modeLabels: Record<Exclude<LibraryMode, "all">, string> = {
  today: "今日时刻",
  favorites: "收藏",
  missing: "文件丢失",
  trash: "回收站",
};

export function transitionLibraryMode(
  fileStatus: string,
  libraryMode: LibraryMode,
): Pick<ActiveFilterInput, "libraryMode" | "fileStatus"> {
  return {
    libraryMode,
    fileStatus: libraryMode === "missing" || libraryMode === "trash" ? "all" : fileStatus,
  };
}

export function transitionFileStatus(
  libraryMode: LibraryMode,
  fileStatus: string,
): Pick<ActiveFilterInput, "libraryMode" | "fileStatus"> {
  return {
    libraryMode:
      (libraryMode === "missing" || libraryMode === "trash") && fileStatus !== "all"
        ? "all"
        : libraryMode,
    fileStatus,
  };
}

export function fileStatusLabel(status: string): string {
  const labels: Record<string, string> = {
    available: "可用",
    missing: "文件丢失",
    inaccessible: "不可访问",
    unsupported: "不支持",
    trashed: "回收站",
  };
  return labels[status] ?? status;
}

export function deriveActiveFilters(
  input: ActiveFilterInput,
): ActiveFilterDescriptor[] {
  const filters: ActiveFilterDescriptor[] = [];
  const query = input.query.trim();

  if (input.libraryMode !== "all") {
    filters.push({ key: "mode", label: modeLabels[input.libraryMode] });
  }
  if (query) filters.push({ key: "query", label: `搜索：${query}` });
  if (input.accountId !== "all") {
    filters.push({ key: "account", label: `账号：${input.accountLabel}` });
  }
  if (input.sourceDirId !== "all") {
    filters.push({ key: "source", label: `来源：${input.sourceDirLabel}` });
  }
  if (input.agentName !== "all") {
    filters.push({ key: "agent", label: `英雄：${input.agentName}` });
  }
  if (input.mapName !== "all") {
    filters.push({ key: "map", label: `地图：${input.mapName}` });
  }
  if (input.gameMode !== "all") {
    filters.push({ key: "game-mode", label: `模式：${input.gameMode}` });
  }
  if (input.tagId !== "all") {
    filters.push({ key: "tag", label: `标签：${input.tagLabel}` });
  }
  if (input.fileStatus !== "all") {
    filters.push({
      key: "file-status",
      label: `状态：${fileStatusLabel(input.fileStatus)}`,
    });
  }
  return filters;
}
