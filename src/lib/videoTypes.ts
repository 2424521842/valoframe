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
const KILL_COMPILATION_PATTERN = /击杀合集|击杀集锦|击杀剪辑|kill compilation|kill montage/;
const DEATH_COMPILATION_PATTERN = /死亡时刻|死亡集锦|death moment|death compilation/;
const ORDINARY_MULTI_KILL_PATTERN =
  /三杀|3杀|三连杀|四杀|4杀|四连杀|\bace\b|五杀|5杀|五连杀|六杀|6杀|六连杀/;
const ORDINARY_MULTI_KILL_COUNTS = new Set([3, 4, 5, 6]);

export type TimelineMarkerMode = "kill" | "death";

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

  const numericType = numericVideoType(clip);
  const titleSource = videoTypeText(clip);

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
    return absoluteTimelineCompilationMode(clip) === "kill";
  }

  return absoluteTimelineCompilationMode(clip) === "death";
}

/**
 * Classifies only compilation exports whose event offsets use absolute video time.
 * Keep this separate from marker eligibility so ordinary multi-kills never enter
 * the kill-compilation/death filters or inherit compilation time semantics.
 */
export function absoluteTimelineCompilationMode(
  clip: VideoTypeMetadata,
): TimelineMarkerMode | null {
  return classifyAbsoluteTimelineCompilation(clip).mode;
}

/** Returns the video categories that may expose markers in the preview timeline. */
export function previewTimelineMarkerMode(
  clip: VideoTypeMetadata,
): TimelineMarkerMode | null {
  const compilation = classifyAbsoluteTimelineCompilation(clip);
  if (compilation.hasSignal) return compilation.mode;

  const numericType = numericVideoType(clip);
  if (numericType !== null && OFFICIAL_SCORE_HIGHLIGHT_TYPES.has(numericType)) {
    return "kill";
  }
  if (
    clip.killCount != null &&
    ORDINARY_MULTI_KILL_COUNTS.has(clip.killCount)
  ) {
    return "kill";
  }
  return ORDINARY_MULTI_KILL_PATTERN.test(videoTypeText(clip)) ? "kill" : null;
}

function classifyAbsoluteTimelineCompilation(
  clip: VideoTypeMetadata,
): { mode: TimelineMarkerMode | null; hasSignal: boolean } {
  const numericType = numericVideoType(clip);
  if (numericType === 2) return { mode: "kill", hasSignal: true };
  if (numericType === 3) return { mode: "death", hasSignal: true };

  const titleSource = videoTypeText(clip);
  const matchesKillCompilation = KILL_COMPILATION_PATTERN.test(titleSource);
  const matchesDeathCompilation = DEATH_COMPILATION_PATTERN.test(titleSource);
  const hasSignal = matchesKillCompilation || matchesDeathCompilation;
  if (matchesKillCompilation === matchesDeathCompilation) {
    return { mode: null, hasSignal };
  }
  return {
    mode: matchesKillCompilation ? "kill" : "death",
    hasSignal,
  };
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

function numericVideoType(clip: VideoTypeMetadata): number | null {
  return clip.highlightType ?? parseNumericType(clip.officialVideoType);
}

function videoTypeText(clip: VideoTypeMetadata): string {
  return [
    clip.officialVideoName ?? "",
    clip.officialVideoType ?? "",
    clip.extractedText ?? "",
  ].join(" ").toLocaleLowerCase("zh-CN");
}
