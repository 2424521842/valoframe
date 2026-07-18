import assert from "node:assert/strict";
import test from "node:test";
import {
  fullDriveDiscoveryActivityMessage,
  scanActivityMessage,
} from "../src/lib/scanSummary.ts";
import { DEFAULT_ACLOS_MISSING_MESSAGE } from "../src/api/backend.ts";
import type { FullDriveScanResult, ScanSummary } from "../src/types.ts";

const baseSummary: ScanSummary = {
  rootPath: "C:\\Users\\Player\\AppData\\ACLOS\\aclos-highlight",
  sourceDirCount: 4,
  clipGroupCount: 13,
  newClipCount: 2,
  updatedClipCount: 5,
  missingClipCount: 0,
  coverMissingCount: 1,
  errors: [],
  message: null,
};

const baseDiscoveryResult: FullDriveScanResult = {
  fixedDriveCount: 2,
  visitedDirectoryCount: 1_250,
  validatedSourceDirCount: 3,
  scanRootCount: 2,
  skippedDirectoryCount: 1,
  discoveryWarnings: [],
  scannedClipCount: 41,
  scanSummary: baseSummary,
};

test("summarizes empty full-drive discovery as a normal result", () => {
  assert.equal(
    fullDriveDiscoveryActivityMessage({
      ...baseDiscoveryResult,
      validatedSourceDirCount: 0,
      scanRootCount: 0,
      scannedClipCount: 0,
    }),
    "未发现标准无畏时刻素材",
  );
});

test("summarizes discovered sources and scanned videos", () => {
  assert.equal(
    fullDriveDiscoveryActivityMessage(baseDiscoveryResult),
    "全电脑发现完成：3 个素材目录，扫描 41 个视频",
  );
});

test("does not surface metadata metrics when only kill event counts are present", () => {
  const summary = {
    ...baseSummary,
    metadataEventCount: 281,
  };
  const message = scanActivityMessage(summary);

  assert.match(message, /扫描完成/);
  assert.doesNotMatch(message, /元数据/);
  assert.doesNotMatch(message, /281/);
});

test("summarizes metadata enrichment in the scan activity message", () => {
  const message = scanActivityMessage({
    ...baseSummary,
    metadataMatchCount: 13,
    metadataEnrichedClipCount: 66,
    metadataEventCount: 281,
  });

  assert.match(message, /扫描完成/);
  assert.match(message, /元数据 13 场/);
  assert.match(message, /66 个片段/);
  assert.doesNotMatch(message, /281 个事件/);
  assert.doesNotMatch(message, /击杀事件/);
});

test("keeps the missing default directory warning as the scan activity message", () => {
  assert.equal(
    scanActivityMessage({
      ...baseSummary,
      sourceDirCount: 0,
      clipGroupCount: 0,
      message:
        "Scan root not found: C:\\Users\\Player\\AppData\\ACLOS\\aclos-highlight",
    }),
    DEFAULT_ACLOS_MISSING_MESSAGE,
  );
});
