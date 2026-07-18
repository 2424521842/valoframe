import assert from "node:assert/strict";
import test from "node:test";
import { normalizeCustomScanPath } from "../src/lib/customScan.ts";

test("normalizes custom scan prompt paths", () => {
  assert.equal(normalizeCustomScanPath("  D:\\五位时刻  "), "D:\\五位时刻");
  assert.equal(normalizeCustomScanPath(""), null);
  assert.equal(normalizeCustomScanPath("   "), null);
  assert.equal(normalizeCustomScanPath(null), null);
});
