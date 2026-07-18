import assert from "node:assert/strict";
import test from "node:test";
import { displayHighlightTitle } from "../src/api/backend.ts";

test("official highlight title preserves an unsuffixed official name", () => {
  assert.equal(displayHighlightTitle({ officialVideoName: "六杀时刻" }), "六杀时刻");
});

test("official highlight title removes a trailing duplicate number from a kill name", () => {
  assert.equal(displayHighlightTitle({ officialVideoName: "三杀时刻2" }), "三杀时刻");
});

test("official highlight title removes a trailing duplicate number from a semantic name", () => {
  assert.equal(displayHighlightTitle({ officialVideoName: "精准预判3" }), "精准预判");
});

test("official highlight title derives a five-kill name from the official type", () => {
  assert.equal(
    displayHighlightTitle({ officialVideoName: "", killCount: 5, highlightType: 10 }),
    "五杀时刻",
  );
});

test("triple and quad official types do not require a duplicated kill count", () => {
  assert.equal(displayHighlightTitle({ highlightType: 4 }), "三杀时刻");
  assert.equal(displayHighlightTitle({ highlightType: 6 }), "四杀时刻");
});

test("official highlight title keeps type two as a kill compilation regardless of count", () => {
  assert.equal(
    displayHighlightTitle({ officialVideoName: "", killCount: 75, highlightType: 2 }),
    "击杀集锦",
  );
});

test("legacy official compilation names use the current video type labels", () => {
  assert.equal(displayHighlightTitle({ officialVideoName: "击杀合集" }), "击杀集锦");
  assert.equal(displayHighlightTitle({ officialVideoName: "死亡集锦" }), "死亡时刻");
});

test("official highlight title derives a six-kill name from type ten", () => {
  assert.equal(
    displayHighlightTitle({ officialVideoName: "", killCount: 6, highlightType: 10 }),
    "六杀时刻",
  );
});

test("official highlight title preserves semantic numbers inside a title", () => {
  assert.equal(displayHighlightTitle({ officialVideoName: "1v3残局" }), "1v3残局");
  assert.equal(displayHighlightTitle({ officialVideoName: "2024年度高光" }), "2024年度高光");
  assert.equal(displayHighlightTitle({ officialVideoName: "排位日记5" }), "排位日记5");
  assert.equal(displayHighlightTitle({ officialVideoName: "挑战赛季3" }), "挑战赛季3");
});

test("official highlight title follows official metadata, KDA, then generic priority", () => {
  assert.equal(
    displayHighlightTitle({
      officialVideoName: "六杀时刻",
      highlightType: 2,
      kda: "18/7/4",
    }),
    "六杀时刻",
  );
  assert.equal(
    displayHighlightTitle({ highlightType: 3, kda: "18/7/4" }),
    "死亡时刻",
  );
  assert.equal(displayHighlightTitle({ kda: "18/7/4" }), "精彩击杀");
  assert.equal(displayHighlightTitle({}), "高光时刻");
});
