import assert from "node:assert/strict";
import test from "node:test";
import {
  STANDARD_MAP_NAMES,
  deriveMapOptions,
} from "../src/lib/maps.ts";

const expectedStandardMaps = [
  "天枢云阙",
  "盐海矿镇",
  "幽邃地窟",
  "日落之城",
  "莲华古城",
  "深海明珠",
  "裂变峡谷",
  "微风岛屿",
  "森寒冬港",
  "亚海悬城",
  "霓虹町",
  "隐世修所",
  "源工重镇",
];

test("standard map catalog contains every current 5v5 map without clip data", () => {
  assert.deepEqual(STANDARD_MAP_NAMES, expectedStandardMaps);
  assert.deepEqual(deriveMapOptions([]), expectedStandardMaps);
});

test("deriveMapOptions keeps the catalog stable and appends newly observed maps", () => {
  assert.deepEqual(
    deriveMapOptions([
      " 源工重镇 ",
      "未来地图",
      "测试地图",
      "未来地图",
      "",
    ]),
    [...expectedStandardMaps, "测试地图", "未来地图"],
  );
});

test("deriveMapOptions does not expose obsolete incorrect Summit labels", () => {
  assert.equal(deriveMapOptions(["幽邃迷境", "迷邃幽境"]).length, 13);
});
