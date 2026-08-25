import assert from "node:assert/strict";
import test from "node:test";
import { formatBytes, formatDateTime } from "../src/lib/formatters.ts";

test("formatDateTime renders ISO timestamps", () => {
  assert.match(formatDateTime("2026-07-02T22:06:00Z"), /\d{2}\/\d{2}/);
});

test("formatDateTime accepts bare unix-second strings from raw storage rows", () => {
  const unixSeconds = "1751491083";
  assert.equal(
    formatDateTime(unixSeconds),
    formatDateTime(new Date(1_751_491_083 * 1_000).toISOString()),
  );
});

test("formatDateTime never throws on unparsable or empty input", () => {
  for (const value of ["", "   ", "not-a-date", null, undefined]) {
    assert.equal(formatDateTime(value), "时间未知");
  }
});

test("formatBytes keeps its existing unit thresholds", () => {
  assert.equal(formatBytes(84_500_000), "85 MB");
  assert.equal(formatBytes(2_500_000_000), "2.5 GB");
});
