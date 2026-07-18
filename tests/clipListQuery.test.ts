import assert from "node:assert/strict";
import test from "node:test";
import {
  buildClipListQuery,
  CLIP_PAGE_SIZE,
  clipListQueryKey,
} from "../src/lib/clipListQuery.ts";

test("production query mapping omits defaults and uses the bounded first page", () => {
  assert.deepEqual(buildClipListQuery({
    query: "  ",
    accountId: "all",
    sourceDirId: "all",
    agentName: "all",
    mapName: "all",
    gameMode: "all",
    tagId: "all",
    highlightFilter: "all",
    fileStatus: "all",
    libraryMode: "all",
    sortBy: "modified-desc",
  }), {
    offset: 0,
    limit: CLIP_PAGE_SIZE,
    sortBy: "modified-desc",
  });
});

test("production query mapping carries every search, filter, mode, date, and sort value", () => {
  const mapped = buildClipListQuery({
    query: "  FixtureAlpha  ",
    accountId: "match-account-1001",
    sourceDirId: "12",
    agentName: "芮娜",
    mapName: "源工重镇",
    gameMode: "竞技模式",
    tagId: "7",
    highlightFilter: "five",
    fileStatus: "missing",
    libraryMode: "favorites",
    modifiedFrom: "2026-07-01",
    modifiedTo: "2026-07-15",
    sortBy: "size-desc",
  }, 100, 50);

  assert.deepEqual(mapped, {
    offset: 100,
    limit: 50,
    sortBy: "size-desc",
    query: "FixtureAlpha",
    accountId: "match-account-1001",
    sourceDirId: 12,
    agentName: "芮娜",
    mapName: "源工重镇",
    gameMode: "竞技模式",
    tagId: 7,
    highlightFilter: "five",
    favoriteFilter: "favorite",
    fileStatus: "missing",
    modifiedFrom: Math.floor(new Date("2026-07-01T00:00:00").getTime() / 1000),
    modifiedTo: Math.floor(new Date("2026-07-15T23:59:59").getTime() / 1000),
  });
});

test("missing and trash modes override the legacy file-status selector", () => {
  const base = {
    query: "",
    accountId: "all",
    sourceDirId: "not-a-number",
    agentName: "all",
    mapName: "all",
    gameMode: "all",
    tagId: "not-a-number",
    highlightFilter: "all" as const,
    fileStatus: "available",
    sortBy: "name-asc" as const,
  };

  assert.equal(buildClipListQuery({ ...base, libraryMode: "missing" }).fileStatus, "missing");
  assert.equal(buildClipListQuery({ ...base, libraryMode: "trash" }).fileStatus, "trashed");
  assert.equal(buildClipListQuery({ ...base, libraryMode: "all" }).sourceDirId, undefined);
  assert.equal(buildClipListQuery({ ...base, libraryMode: "all" }).tagId, undefined);
});

test("query generation keys ignore only the page offset", () => {
  const first = { offset: 0, limit: 50, query: "ace", sortBy: "modified-desc" as const };
  assert.equal(clipListQueryKey(first), clipListQueryKey({ ...first, offset: 50 }));
  assert.notEqual(clipListQueryKey(first), clipListQueryKey({ ...first, sortBy: "name-asc" }));
});
