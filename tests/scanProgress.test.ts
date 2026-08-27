import assert from "node:assert/strict";
import test from "node:test";
import { scanProgressPresentation } from "../src/lib/scanProgress.ts";
import type { ScanProgress } from "../src/types.ts";

test("file scanning maps into 10-85% and never reaches 100%", () => {
  assert.equal(scanProgressPresentation(progress("scanning", 0, 10), "running").percent, 10);
  assert.equal(scanProgressPresentation(progress("scanning", 5, 10), "running").percent, 48);
  assert.deepEqual(
    scanProgressPresentation(progress("scanning", 10, 10), "running"),
    {
      percent: 85,
      determinate: true,
      stageLabel: "正在扫描录像 · 已处理 10 / 10",
      ariaLabel: "扫描进度：正在扫描录像 · 已处理 10 / 10",
      ariaValueText: "正在扫描录像 · 已处理 10 / 10，85%",
    },
  );
});

test("database and finalization phases stay indeterminate below completion", () => {
  for (const [phase, stageLabel] of [
    ["importing", "扫描完成，正在导入数据"],
    ["metadata", "正在整理元数据"],
    ["finalizing", "正在完成收尾"],
  ] as const) {
    const result = scanProgressPresentation(progress(phase, 10, 10), "running");
    assert.equal(result.percent, null);
    assert.equal(result.determinate, false);
    assert.equal(result.stageLabel, stageLabel);
  }
});

test("100% requires both the completed status and terminal flag", () => {
  assert.equal(
    scanProgressPresentation(progress("completed", 10, 10, { status: "completed" }), "completed").percent,
    null,
  );
  assert.equal(
    scanProgressPresentation(progress("completed", 10, 10, { status: "completed", terminal: true }), "completed").percent,
    100,
  );
  assert.equal(
    scanProgressPresentation(progress("partial", 10, 10, { status: "partial", terminal: true }), "partial").percent,
    99,
  );
  assert.equal(
    scanProgressPresentation(progress("cancelled", 10, 10, { status: "cancelled", terminal: true }), "cancelled").percent,
    null,
  );
  assert.equal(
    scanProgressPresentation(progress("failed", 10, 10, { status: "failed", terminal: true }), "failed").percent,
    null,
  );
});

test("cancelling and unknown phases have truthful accessible fallbacks", () => {
  assert.equal(
    scanProgressPresentation(progress("cancelling", 2, 5, { status: "cancelling" }), "cancelling").stageLabel,
    "正在完成安全取消…",
  );
  assert.equal(
    scanProgressPresentation(progress("future-phase", 2, 5, { message: "后台正在迁移索引" }), "running").stageLabel,
    "后台正在迁移索引",
  );
});

function progress(
  phase: string,
  processed: number,
  total: number | null,
  overrides: Partial<ScanProgress> = {},
): ScanProgress {
  return {
    jobId: "scan-1",
    phase,
    currentRoot: "D:\\clips",
    source: null,
    processed,
    total,
    terminal: false,
    status: "running",
    sourceDirCount: processed,
    clipGroupCount: 0,
    clipFileCount: 0,
    message: "扫描中",
    ...overrides,
  };
}
