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
const HIGHLIGHT_MARKER_SUFFIXES = [
  "时刻",
  "集锦",
  "高光",
  "片段",
  "剪辑",
  "回放",
  "合集",
] as const;
const HIGHLIGHT_BOUNDARY_PUNCTUATION = new Set([
  "，", "。", "！", "？", "：", "；", "、", "—", "…", "（", "）", "【", "】",
  "《", "》", "“", "”", "‘", "’",
]);

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
  return resolveVideoType(clip) === filter;
}

/**
 * Resolves one canonical product category for a clip.
 *
 * `killCount` is the number of self-kill events inside the whole exported video.
 * A kill compilation can therefore contain four, five, or six kills without being
 * an ordinary multi-kill moment. Official numeric types take precedence, and the
 * weaker count/text fallbacks are used only when no positive numeric type exists.
 */
export function resolveVideoType(
  clip: VideoTypeMetadata,
): VideoTypeFilter | null {
  const numericType = numericVideoType(clip);
  if (numericType === 2) return "kill-compilation";
  if (numericType === 3) return "death";
  if (numericType === 4) return "triple";
  if (numericType === 6) return "quad";
  if (numericType === 10) {
    if (clip.killCount === 6) return "six";
    if (clip.killCount === 5) return "five";

    const titleSource = videoTypeText(clip);
    if (containsHighlightMarker(titleSource, ["六杀", "6杀", "六连杀"])) return "six";
    if (containsFiveKillMarker(titleSource)) return "five";
    return null;
  }
  if (numericType !== null && numericType > 0) return null;

  const compilation = classifyAbsoluteTimelineCompilation(clip);
  if (compilation.hasSignal) {
    return compilation.mode === "kill"
      ? "kill-compilation"
      : compilation.mode === "death"
        ? "death"
        : null;
  }

  const titleSource = videoTypeText(clip);
  if (containsHighlightMarker(titleSource, ["六杀", "6杀", "六连杀"])) return "six";
  if (containsFiveKillMarker(titleSource)) return "five";
  if (containsHighlightMarker(titleSource, ["四杀", "4杀", "四连杀"])) return "quad";
  if (containsHighlightMarker(titleSource, ["三杀", "3杀", "三连杀"])) return "triple";
  if (clip.killCount === 6) return "six";
  if (clip.killCount === 5) return "five";
  if (clip.killCount === 4) return "quad";
  if (clip.killCount === 3) return "triple";
  return null;
}

export function resolvedVideoTypeLabel(clip: VideoTypeMetadata): string | null {
  const videoType = resolveVideoType(clip);
  return videoType ? videoTypeLabel(videoType) : null;
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
  const videoType = resolveVideoType(clip);
  return videoType && !["kill-compilation", "death"].includes(videoType)
    ? "kill"
    : null;
}

function classifyAbsoluteTimelineCompilation(
  clip: VideoTypeMetadata,
): { mode: TimelineMarkerMode | null; hasSignal: boolean } {
  const numericType = numericVideoType(clip);
  if (numericType === 2) return { mode: "kill", hasSignal: true };
  if (numericType === 3) return { mode: "death", hasSignal: true };
  if (numericType !== null && numericType > 0) {
    return { mode: null, hasSignal: false };
  }

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
  const normalized = value.trim();
  if (!/^\d+$/.test(normalized)) return null;
  const parsed = Number(normalized);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function containsFiveKillMarker(text: string): boolean {
  return /(?:^|\s)ace(?:$|\s)/.test(text)
    || containsHighlightMarker(text, ["五杀", "5杀", "五连杀"]);
}

function containsHighlightMarker(text: string, markers: readonly string[]): boolean {
  return markers.some((marker) => {
    let searchStart = 0;
    while (searchStart <= text.length) {
      const markerIndex = text.indexOf(marker, searchStart);
      if (markerIndex < 0) return false;
      if (hasHighlightMarkerBoundary(text.slice(markerIndex + marker.length))) {
        return true;
      }
      searchStart = markerIndex + marker.length;
    }
    return false;
  });
}

function hasHighlightMarkerBoundary(remainder: string): boolean {
  if (!remainder) return true;
  if (HIGHLIGHT_MARKER_SUFFIXES.some((suffix) => remainder.startsWith(suffix))) {
    return true;
  }
  const boundary = remainder[0];
  const codePoint = boundary.codePointAt(0) ?? 0;
  const isAsciiPunctuation =
    (codePoint >= 33 && codePoint <= 47)
    || (codePoint >= 58 && codePoint <= 64)
    || (codePoint >= 91 && codePoint <= 96)
    || (codePoint >= 123 && codePoint <= 126);
  return /\s/u.test(boundary)
    || isAsciiPunctuation
    || HIGHLIGHT_BOUNDARY_PUNCTUATION.has(boundary);
}

function numericVideoType(clip: VideoTypeMetadata): number | null {
  return clip.highlightType ?? parseNumericType(clip.officialVideoType);
}

function videoTypeText(clip: VideoTypeMetadata): string {
  return [
    clip.officialVideoName ?? "",
    clip.officialVideoType ?? "",
  ].join(" ").toLocaleLowerCase("zh-CN");
}
