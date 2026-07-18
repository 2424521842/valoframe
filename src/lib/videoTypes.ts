import type { HighlightFilter } from "../types";

export type VideoTypeFilter = Exclude<HighlightFilter, "all">;

export const VIDEO_TYPE_FILTERS: readonly VideoTypeFilter[] = [
  "triple",
  "quad",
  "five",
  "six",
  "kill-compilation",
  "death",
];

export type VideoTypeMetadata = {
  officialVideoName?: string | null;
  officialVideoType?: string | null;
  killCount?: number | null;
  highlightType?: number | null;
  extractedText?: string | null;
  gameMode?: string | null;
};

const OFFICIAL_SCORE_GAME_MODES = new Set([
  "普通模式",
  "极速模式",
  "竞技模式",
]);
const OFFICIAL_SCORE_HIGHLIGHT_TYPES = new Set([4, 6, 10]);
const OFFICIAL_NON_SCORING_VIDEO_TYPES = new Set([
  "击杀合集",
  "击杀集锦",
  "死亡集锦",
  "死亡时刻",
  "夜市翻牌",
]);

export function videoTypeLabel(value: HighlightFilter): string {
  if (value === "triple") return "三杀时刻";
  if (value === "quad") return "四杀时刻";
  if (value === "five") return "五杀时刻";
  if (value === "six") return "六杀时刻";
  if (value === "kill-compilation") return "击杀集锦";
  if (value === "death") return "死亡时刻";
  return "全部类型";
}

export function matchesVideoType(
  clip: VideoTypeMetadata,
  filter: HighlightFilter | undefined,
): boolean {
  if (!filter || filter === "all") return true;

  const numericType = clip.highlightType ?? parseNumericType(clip.officialVideoType);
  const titleSource = [
    clip.officialVideoName ?? "",
    clip.officialVideoType ?? "",
    clip.extractedText ?? "",
  ].join(" ").toLocaleLowerCase("zh-CN");

  if (filter === "triple") {
    return clip.killCount === 3 || numericType === 4 || /三杀|3杀|三连杀/.test(titleSource);
  }
  if (filter === "quad") {
    return clip.killCount === 4 || numericType === 6 || /四杀|4杀|四连杀/.test(titleSource);
  }
  if (filter === "five") {
    return clip.killCount === 5 || /\bace\b|五杀|5杀|五连杀/.test(titleSource);
  }
  if (filter === "six") {
    return clip.killCount === 6 || /六杀|6杀|六连杀/.test(titleSource);
  }
  if (filter === "kill-compilation") {
    return numericType === 2 || /击杀合集|击杀集锦|击杀剪辑|kill compilation|kill montage/.test(titleSource);
  }

  return numericType === 3 || /死亡时刻|死亡集锦|death moment|death compilation/.test(titleSource);
}

export function expectsOfficialRoundScore(clip: VideoTypeMetadata): boolean {
  const gameMode = clip.gameMode?.trim();
  if (gameMode && !OFFICIAL_SCORE_GAME_MODES.has(gameMode)) return false;

  const officialVideoType = clip.officialVideoType?.trim() ?? "";
  if (officialVideoType.endsWith("杀时刻")) return true;
  if (OFFICIAL_NON_SCORING_VIDEO_TYPES.has(officialVideoType)) return false;

  const numericVideoType = parseNumericType(officialVideoType);
  if (numericVideoType != null) {
    return OFFICIAL_SCORE_HIGHLIGHT_TYPES.has(numericVideoType);
  }

  return clip.highlightType != null &&
    OFFICIAL_SCORE_HIGHLIGHT_TYPES.has(clip.highlightType);
}

function parseNumericType(value: string | null | undefined): number | null {
  if (!value?.trim()) return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) ? parsed : null;
}
