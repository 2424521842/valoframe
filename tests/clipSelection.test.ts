import assert from "node:assert/strict";
import test from "node:test";
import { mockClips } from "../src/data/mockData.ts";
import {
  clipTagSelectionState,
  pruneClipSelection,
  toggleAllVisibleClipSelection,
  updateClipSelection,
} from "../src/lib/clipSelection.ts";

test("ctrl-style selection toggles one clip without clearing prior choices", () => {
  const result = updateClipSelection(new Set(["a"]), ["a", "b", "c"], "b", "a", {
    additive: true,
    range: false,
  });
  assert.deepEqual([...result.selectedIds], ["a", "b"]);
  assert.equal(result.anchorId, "b");
});

test("shift selection selects a contiguous visible range from the anchor", () => {
  const result = updateClipSelection(new Set(["a"]), ["a", "b", "c", "d"], "c", "a", {
    additive: false,
    range: true,
  });
  assert.deepEqual([...result.selectedIds], ["a", "b", "c"]);
  assert.equal(result.anchorId, "a");
});

test("select-all toggles only the current visible result and pruning removes hidden choices", () => {
  const selected = toggleAllVisibleClipSelection(new Set(["hidden"]), ["a", "b"]);
  assert.deepEqual([...selected], ["hidden", "a", "b"]);
  assert.deepEqual([...pruneClipSelection(selected, ["a", "b"])], ["a", "b"]);
  assert.deepEqual([...toggleAllVisibleClipSelection(new Set(["a", "b"]), ["a", "b"])], []);
});

test("tag selection reports unchecked, mixed, and checked states", () => {
  const first = { ...mockClips[0], tags: ["review"] };
  const second = { ...mockClips[1], tags: [] };
  assert.equal(clipTagSelectionState([first, second], "review"), "indeterminate");
  assert.equal(clipTagSelectionState([first], "review"), true);
  assert.equal(clipTagSelectionState([second], "review"), false);
});
