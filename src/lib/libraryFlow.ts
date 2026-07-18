import type { LibraryDatePreset } from "../types";

export type LibrarySearchSuggestionGroup = {
  label: "账号" | "英雄" | "地图" | "标签";
  values: string[];
};

type LibrarySearchSuggestionInput = {
  accounts: string[];
  agents: string[];
  maps: string[];
  tags: string[];
};

export function buildLibrarySearchSuggestionGroups({
  accounts,
  agents,
  maps,
  tags,
}: LibrarySearchSuggestionInput): LibrarySearchSuggestionGroup[] {
  return [
    suggestionGroup("账号", accounts),
    suggestionGroup("英雄", agents),
    suggestionGroup("地图", maps),
    suggestionGroup("标签", tags),
  ].filter((group) => group.values.length > 0);
}

export function dateRangeForPreset(
  preset: LibraryDatePreset,
  now = new Date(),
): { modifiedFrom?: string; modifiedTo?: string } {
  if (preset === "all") return {};

  const end = dateInputValue(now);
  const start = new Date(now);

  if (preset === "week") {
    start.setDate(start.getDate() - 6);
  } else if (preset === "month") {
    start.setDate(start.getDate() - 29);
  }

  return {
    modifiedFrom: dateInputValue(start),
    modifiedTo: end,
  };
}

function dateInputValue(value: Date): string {
  const year = value.getFullYear();
  const month = String(value.getMonth() + 1).padStart(2, "0");
  const day = String(value.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function suggestionGroup(
  label: LibrarySearchSuggestionGroup["label"],
  values: string[],
): LibrarySearchSuggestionGroup {
  return {
    label,
    values: [...new Set(values.map((value) => value.trim()).filter(Boolean))],
  };
}
