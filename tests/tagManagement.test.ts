import assert from "node:assert/strict";
import test from "node:test";
import { mockClips, mockTags } from "../src/data/mockData.ts";
import {
  countTagUsage,
  filterCustomTags,
  mergeTagsWithFacets,
  removeTagFromClipCollection,
} from "../src/lib/tags.ts";

test("counts each tag once per clip for management usage totals", () => {
  const counts = countTagUsage([
    { ...mockClips[0], tags: ["triple", "triple", "smoke"] },
    { ...mockClips[1], tags: ["triple"] },
  ]);

  assert.equal(counts.get("triple"), 2);
  assert.equal(counts.get("smoke"), 1);
});

test("sorts and searches user-created tags without a system-tag tier", () => {
  const sorted = filterCustomTags(
    [
      { id: "custom", label: "精彩残局", color: "green" },
      { id: "review", label: "复盘", color: "gold" },
      { id: "entry", label: "突破", color: "red" },
    ],
    "",
  );

  assert.deepEqual(sorted.map((tag) => tag.label), ["复盘", "精彩残局", "突破"]);
  assert.deepEqual(
    filterCustomTags(sorted, "残局").map((tag) => tag.id),
    ["custom"],
  );
});

test("removing a managed tag clears clip links without mutating other clip data", () => {
  const source = mockClips.slice(0, 2);
  const removedTagId = source[0].tags[0];
  const originalTags = [...source[0].tags];
  const updated = removeTagFromClipCollection(source, removedTagId);

  assert.deepEqual(source[0].tags, originalTags);
  assert.equal(updated[0].tags.includes(removedTagId), false);
  assert.equal(updated[0].filePath, source[0].filePath);
  assert.equal(updated[1], source[1]);
});

test("mock catalog contains only user-created organizational labels", () => {
  assert.ok(mockTags.length > 0);
  assert.equal(
    mockTags.some((tag) =>
      ["三杀时刻", "四杀时刻", "五杀时刻", "六杀时刻", "击杀集锦", "死亡时刻"].includes(tag.label)
    ),
    false,
  );
});

test("fresh tag edits take precedence over stale facet label and color data", () => {
  const tags = [{ id: "7", label: "精选复盘", color: "green" as const }];
  const merged = mergeTagsWithFacets(
    tags,
    [{
      id: 7,
      name: "复盘",
      color: "blue",
      count: 3,
      activeCount: 3,
    }],
    "all",
  );

  assert.deepEqual(merged, tags);
});
