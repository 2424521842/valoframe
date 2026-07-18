import assert from "node:assert/strict";
import test from "node:test";
import { mockClips, mockTags } from "../src/data/mockData.ts";
import { groupClipsByMatch } from "../src/lib/accountGrouping.ts";
import { applyClipFilters } from "../src/lib/clipFilters.ts";
import {
  buildLibrarySearchSuggestionGroups,
  dateRangeForPreset,
} from "../src/lib/libraryFlow.ts";
import {
  mergeScanSummaries,
  mergeScanTargets,
  scanTargetFromPath,
} from "../src/lib/scanTargets.ts";

test("manual scan targets are staged and deduplicated against indexed sources", () => {
  const manual = scanTargetFromPath("D:\\VALO Clips\\");
  const targets = mergeScanTargets(
    [{ id: "1", name: "wonderfulVideos1001", path: "D:\\VALO Clips\\wonderfulVideos1001" }],
    [manual],
  );

  assert.equal(targets.length, 1);
  assert.equal(targets[0].path, "D:\\VALO Clips");
  assert.equal(targets[0].origin, "manual");
});

test("removing a scan path excludes both indexed and manual representations", () => {
  const targets = mergeScanTargets(
    [{ id: "1", name: "wonderfulVideos1001", path: "D:\\Highlights\\wonderfulVideos1001" }],
    [scanTargetFromPath("D:\\Highlights")],
    new Set(["d:\\highlights"]),
  );

  assert.deepEqual(targets, []);
});

test("multi-directory scan summaries combine into one user-visible result", () => {
  const summary = mergeScanSummaries([
    {
      rootPath: "D:\\A",
      sourceDirCount: 1,
      clipGroupCount: 2,
      newClipCount: 3,
      updatedClipCount: 1,
      missingClipCount: 0,
      coverMissingCount: 2,
      metadataEventCount: 4,
      errors: [],
      message: "A 完成",
    },
    {
      rootPath: "E:\\B",
      sourceDirCount: 2,
      clipGroupCount: 3,
      newClipCount: 4,
      updatedClipCount: 2,
      missingClipCount: 1,
      coverMissingCount: 1,
      metadataEventCount: 5,
      errors: ["一个非致命警告"],
      message: "B 完成",
    },
  ]);

  assert.equal(summary.rootPath, "多个扫描目录");
  assert.equal(summary.sourceDirCount, 3);
  assert.equal(summary.clipGroupCount, 5);
  assert.equal(summary.newClipCount, 7);
  assert.equal(summary.metadataEventCount, 9);
  assert.deepEqual(summary.errors, ["一个非致命警告"]);
});

test("date presets produce real filter boundaries", () => {
  const now = new Date("2026-07-13T12:00:00+08:00");
  assert.deepEqual(dateRangeForPreset("week", now), {
    modifiedFrom: "2026-07-07",
    modifiedTo: "2026-07-13",
  });
  assert.deepEqual(dateRangeForPreset("month", now), {
    modifiedFrom: "2026-06-14",
    modifiedTo: "2026-07-13",
  });
});

test("search suggestions group real filter dimensions and remove duplicates", () => {
  const groups = buildLibrarySearchSuggestionGroups({
    accounts: ["FixtureBravo#TEST", "FixtureBravo#TEST", ""],
    agents: ["捷风", "芮娜"],
    maps: ["源工重镇"],
    tags: ["三杀", "三杀", "残局"],
  });

  assert.deepEqual(groups, [
    { label: "账号", values: ["FixtureBravo#TEST"] },
    { label: "英雄", values: ["捷风", "芮娜"] },
    { label: "地图", values: ["源工重镇"] },
    { label: "标签", values: ["三杀", "残局"] },
  ]);
});

test("highlight-type filtering changes the clip result set", () => {
  const filtered = applyClipFilters(mockClips, {
    query: "",
    sourceDirId: "all",
    tagId: "all",
    highlightFilter: "triple",
    tags: mockTags,
    sortBy: "modified-desc",
  });

  assert.ok(filtered.length > 0);
  assert.ok(filtered.every((clip) => clip.killCount === 3 || clip.highlightType === 4));
});

test("recycled clips stay out of normal results and appear only in the recycle-bin filter", () => {
  const active = { ...mockClips[0], id: "active-clip", fileStatus: "available" };
  const trashed = { ...mockClips[1], id: "trashed-clip", fileStatus: "trashed" };
  const defaults = {
    query: "",
    sourceDirId: "all",
    tagId: "all",
    tags: mockTags,
    sortBy: "modified-desc" as const,
  };

  assert.deepEqual(applyClipFilters([active, trashed], defaults).map((clip) => clip.id), ["active-clip"]);
  assert.deepEqual(
    applyClipFilters([active, trashed], { ...defaults, fileStatus: "trashed" }).map((clip) => clip.id),
    ["trashed-clip"],
  );
});

test("library grouping is flat by match and carries account identity into every header", () => {
  const groups = groupClipsByMatch(mockClips);
  assert.ok(groups.length > 1);
  assert.ok(groups.every((group) => group.accountDisplayName.length > 0));
  assert.equal(groups.reduce((count, group) => count + group.clips.length, 0), mockClips.length);
});
