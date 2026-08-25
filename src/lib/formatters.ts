export function formatBytes(bytes: number): string {
  if (bytes < 1_000_000) {
    return `${Math.round(bytes / 1_000)} KB`;
  }

  if (bytes < 1_000_000_000) {
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
}

/**
 * Formats a timestamp for display. Accepts ISO strings and bare unix-second strings (the raw
 * `clips.modified_at` / `pending_manual_clips.modified_at` storage format), and returns a
 * placeholder instead of throwing on unparsable input — `Intl.DateTimeFormat.format` raises
 * `Invalid time value` for an invalid `Date`, which would unmount the whole workspace tree.
 */
export function formatDateTime(value: string | null | undefined): string {
  const parsed = parseDisplayDate(value);
  if (!parsed) {
    return "时间未知";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(parsed);
}

function parseDisplayDate(value: string | null | undefined): Date | null {
  const normalized = value?.trim();
  if (!normalized) {
    return null;
  }

  const parsed = /^\d+$/.test(normalized)
    ? new Date(Number(normalized) * 1_000)
    : new Date(normalized);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}
