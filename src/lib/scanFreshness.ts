import type { SourceDir } from "../types";

const LOCAL_DAY_MS = 86_400_000;
const ISO_UTC_PATTERN = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/;

export const SCAN_FRESHNESS_WARNING_DAYS = 7;

export type ScanFreshnessIssue = "invalid" | "future" | null;

export type SourceScanFreshness = {
  daysSinceScan: number | null;
  label: string;
  needsAttention: boolean;
  issue: ScanFreshnessIssue;
};

export type ScanFreshnessSummary = {
  overdueSourceCount: number;
  neverScannedSourceCount: number;
  longestDaysSinceScan: number | null;
  message: string | null;
  needsAttention: boolean;
};

/**
 * Computes freshness from local calendar dates instead of elapsed 24-hour periods. Converting
 * each local date tuple to a UTC ordinal keeps midnight and DST boundaries deterministic.
 */
export function sourceScanFreshness(
  lastScanAt: string | null | undefined,
  now: Date = new Date(),
  timeZone?: string,
): SourceScanFreshness {
  const scannedAt = parseIsoUtc(lastScanAt);
  if (!scannedAt) {
    return {
      daysSinceScan: null,
      label: "尚未完成首次扫描",
      needsAttention: true,
      issue: lastScanAt ? "invalid" : null,
    };
  }

  const rawDays = localDayOrdinal(now, timeZone) - localDayOrdinal(scannedAt, timeZone);
  const issue = rawDays < 0 ? "future" : null;
  const daysSinceScan = Math.max(0, rawDays);
  return {
    daysSinceScan,
    label: daysSinceScan === 0 ? "今天扫描" : `${daysSinceScan} 天未扫描`,
    needsAttention: daysSinceScan >= SCAN_FRESHNESS_WARNING_DAYS,
    issue,
  };
}

export function summarizeScanFreshness(
  sources: readonly Pick<SourceDir, "enabled" | "lastScanAt">[],
  now: Date = new Date(),
  timeZone?: string,
): ScanFreshnessSummary {
  let overdueSourceCount = 0;
  let neverScannedSourceCount = 0;
  let longestDaysSinceScan: number | null = null;

  for (const source of sources) {
    if (!source.enabled) continue;
    const freshness = sourceScanFreshness(source.lastScanAt, now, timeZone);
    if (!freshness.needsAttention) continue;

    overdueSourceCount += 1;
    if (freshness.daysSinceScan === null) {
      neverScannedSourceCount += 1;
    } else {
      longestDaysSinceScan = Math.max(
        longestDaysSinceScan ?? freshness.daysSinceScan,
        freshness.daysSinceScan,
      );
    }
  }

  const message = formatScanFreshnessSummary({
    overdueSourceCount,
    neverScannedSourceCount,
    longestDaysSinceScan,
  });
  return {
    overdueSourceCount,
    neverScannedSourceCount,
    longestDaysSinceScan,
    message,
    needsAttention: overdueSourceCount > 0,
  };
}

export function formatScanFreshnessSummary(
  summary: Pick<
    ScanFreshnessSummary,
    "overdueSourceCount" | "neverScannedSourceCount" | "longestDaysSinceScan"
  >,
): string | null {
  if (summary.overdueSourceCount <= 0) return null;

  let message = `${summary.overdueSourceCount} 个视频来源需要扫描`;
  if (summary.longestDaysSinceScan !== null) {
    message += `，最长 ${summary.longestDaysSinceScan} 天未扫描`;
  }
  if (summary.neverScannedSourceCount > 0) {
    message += `，其中 ${summary.neverScannedSourceCount} 个尚未完成首次扫描`;
  }
  return message;
}

function parseIsoUtc(value: string | null | undefined): Date | null {
  if (!value) return null;
  const match = ISO_UTC_PATTERN.exec(value);
  if (!match) return null;
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return null;
  const parsed = new Date(timestamp);
  const expected = match.slice(1, 7).map(Number);
  const actual = [
    parsed.getUTCFullYear(),
    parsed.getUTCMonth() + 1,
    parsed.getUTCDate(),
    parsed.getUTCHours(),
    parsed.getUTCMinutes(),
    parsed.getUTCSeconds(),
  ];
  return expected.every((part, index) => part === actual[index]) ? parsed : null;
}

function localDayOrdinal(value: Date, timeZone?: string): number {
  if (timeZone) {
    const parts = new Intl.DateTimeFormat("en-US-u-ca-gregory", {
      timeZone,
      year: "numeric",
      month: "numeric",
      day: "numeric",
    }).formatToParts(value);
    const numericPart = (type: "year" | "month" | "day") => Number(
      parts.find((part) => part.type === type)?.value,
    );
    return Math.trunc(
      Date.UTC(numericPart("year"), numericPart("month") - 1, numericPart("day")) /
        LOCAL_DAY_MS,
    );
  }
  return Math.trunc(
    Date.UTC(value.getFullYear(), value.getMonth(), value.getDate()) / LOCAL_DAY_MS,
  );
}
