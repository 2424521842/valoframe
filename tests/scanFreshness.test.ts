import assert from "node:assert/strict";
import test from "node:test";
import {
  sourceScanFreshness,
  summarizeScanFreshness,
} from "../src/lib/scanFreshness.ts";

test("formats first scan, today, six days, and the seven-day threshold", () => {
  const now = new Date("2026-08-09T12:00:00Z");
  assert.equal(sourceScanFreshness(null, now, "UTC").label, "尚未完成首次扫描");
  assert.deepEqual(sourceScanFreshness("2026-08-09T00:01:00Z", now, "UTC"), {
    daysSinceScan: 0,
    label: "今天扫描",
    needsAttention: false,
    issue: null,
  });
  assert.equal(
    sourceScanFreshness("2026-08-03T23:59:00Z", now, "UTC").needsAttention,
    false,
  );
  assert.deepEqual(sourceScanFreshness("2026-08-02T23:59:00Z", now, "UTC"), {
    daysSinceScan: 7,
    label: "7 天未扫描",
    needsAttention: true,
    issue: null,
  });
  assert.deepEqual(sourceScanFreshness("2026-07-10T23:59:00Z", now, "UTC"), {
    daysSinceScan: 30,
    label: "30 天未扫描",
    needsAttention: true,
    issue: null,
  });
});

test("uses local calendar days across midnight and daylight-saving changes", () => {
  assert.equal(
    sourceScanFreshness(
      "2026-08-08T15:59:00Z",
      new Date("2026-08-08T16:01:00Z"),
      "Asia/Shanghai",
    ).daysSinceScan,
    1,
  );
  assert.equal(
    sourceScanFreshness(
      "2026-03-08T06:30:00Z",
      new Date("2026-03-09T05:30:00Z"),
      "America/New_York",
    ).daysSinceScan,
    1,
    "a 23-hour DST day is still one local calendar day",
  );
});

test("treats malformed and impossible UTC timestamps as never scanned", () => {
  const now = new Date("2026-08-09T12:00:00Z");
  for (const value of [
    "2026-08-09 00:00:00",
    "2026-13-01T00:00:00Z",
    "2025-02-29T00:00:00Z",
    "2026-02-30T00:00:00Z",
    "not-a-date",
  ]) {
    const freshness = sourceScanFreshness(value, now, "UTC");
    assert.equal(freshness.label, "尚未完成首次扫描", value);
    assert.equal(freshness.issue, "invalid", value);
    assert.equal(freshness.needsAttention, true, value);
  }
  assert.equal(
    sourceScanFreshness("2024-02-29T00:00:00Z", now, "UTC").issue,
    null,
  );
});

test("clamps future local dates to today and exposes a diagnostic issue", () => {
  assert.deepEqual(
    sourceScanFreshness(
      "2026-08-10T00:00:00Z",
      new Date("2026-08-09T12:00:00Z"),
      "UTC",
    ),
    {
      daysSinceScan: 0,
      label: "今天扫描",
      needsAttention: false,
      issue: "future",
    },
  );
});

test("aggregates only enabled overdue sources without inventing a longest age", () => {
  const now = new Date("2026-08-09T12:00:00Z");
  const summary = summarizeScanFreshness([
    { enabled: true, lastScanAt: "2026-07-31T22:00:00Z" },
    { enabled: true, lastScanAt: null },
    { enabled: false, lastScanAt: null },
    { enabled: true, lastScanAt: "2026-08-03T00:00:00Z" },
  ], now, "UTC");
  assert.deepEqual(summary, {
    overdueSourceCount: 2,
    neverScannedSourceCount: 1,
    longestDaysSinceScan: 9,
    message: "2 个视频来源需要扫描，最长 9 天未扫描，其中 1 个尚未完成首次扫描",
    needsAttention: true,
  });

  assert.equal(
    summarizeScanFreshness(
      [{ enabled: true, lastScanAt: null }],
      now,
      "UTC",
    ).message,
    "1 个视频来源需要扫描，其中 1 个尚未完成首次扫描",
  );

  assert.deepEqual(
    summarizeScanFreshness(
      [
        { enabled: false, lastScanAt: null },
        { enabled: false, lastScanAt: "2026-07-01T00:00:00Z" },
      ],
      now,
      "UTC",
    ),
    {
      overdueSourceCount: 0,
      neverScannedSourceCount: 0,
      longestDaysSinceScan: null,
      message: null,
      needsAttention: false,
    },
  );
});
