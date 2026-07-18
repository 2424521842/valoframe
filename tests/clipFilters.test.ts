import assert from "node:assert/strict";
import test from "node:test";
import type { Clip, Tag } from "../src/types.ts";
import { applyClipFilters } from "../src/lib/clipFilters.ts";

const clips: Clip[] = [
  {
    id: "clip-1",
    fileName: "2026-06-28-haven-clutch.mp4",
    filePath: "C:\\Clips\\Ranked\\2026-06-28-haven-clutch.mp4",
    sourceDirId: "source-ranked",
    sourceDirName: "Ranked Highlights",
    sourceDirPath: "C:\\Clips\\Ranked",
    clipGroupId: "match-1",
    clipGroupName: "2026-06-28 19:12",
    accountId: "FixtureAlpha#0001",
    accountName: "FixtureAlpha#0001",
    accountDisplayName: "FixtureAlpha#0001",
    accountSourceName: "Ranked Highlights",
    accountDetectedBy: "metadata",
    playerName: "FixtureAlpha#0001",
    agentName: "芮娜",
    mapName: "天枢之阙",
    gameMode: "竞技模式",
    scoreline: "13/10",
    kda: "24/13/7",
    createdAt: "2026-06-28T19:12:00+08:00",
    modifiedAt: "2026-06-28T19:14:00+08:00",
    sizeBytes: 186_000_000,
    durationMs: 42_000,
    isFavorite: true,
    isMissing: false,
    fileStatus: "available",
    tags: ["clutch", "haven"],
    note: "1v3 retake",
    extractedText: "Haven defensive lockdown",
    thumbnailTone: "red",
    thumbnailUrl: null,
  },
  {
    id: "clip-2",
    fileName: "2026-06-29-bind-pistol.mp4",
    filePath: "D:\\Valorant\\2026-06-29-bind-pistol.mp4",
    sourceDirId: "source-imported",
    sourceDirName: "Manual Import",
    sourceDirPath: "D:\\Valorant",
    clipGroupId: "match-2",
    clipGroupName: "2026-06-29 21:05",
    accountId: "FixtureBravo#0002",
    accountName: "FixtureBravo#0002",
    accountDisplayName: "FixtureBravo#0002",
    accountSourceName: "Manual Import",
    accountDetectedBy: "metadata",
    playerName: "FixtureBravo#0002",
    agentName: "捷风",
    mapName: "源工重镇",
    gameMode: "极速模式",
    scoreline: "5/3",
    kda: "14/8/2",
    createdAt: "2026-06-29T21:05:00+08:00",
    modifiedAt: "2026-06-29T21:07:00+08:00",
    sizeBytes: 142_000_000,
    durationMs: 28_000,
    isFavorite: false,
    isMissing: false,
    fileStatus: "available",
    tags: ["pistol"],
    note: "fast B hit",
    extractedText: "orb pickup and entry timing",
    thumbnailTone: "gold",
    thumbnailUrl: null,
  },
  {
    id: "clip-3",
    fileName: "split-retake-smoke.mp4",
    filePath: "E:\\Archive\\split-retake-smoke.mp4",
    sourceDirId: "source-archive",
    sourceDirName: "Archive",
    sourceDirPath: "E:\\Archive",
    clipGroupId: "match-3",
    clipGroupName: "2026-06-20 13:30",
    accountId: "FixtureAlpha#0001",
    accountName: "FixtureAlpha#0001",
    accountDisplayName: "FixtureAlpha#0001",
    accountSourceName: "Archive",
    accountDetectedBy: "metadata",
    playerName: "FixtureAlpha#0001",
    agentName: "蝰蛇",
    mapName: "双塔迷城",
    gameMode: "竞技模式",
    scoreline: "9/13",
    kda: "17/16/9",
    createdAt: "2026-06-20T13:30:00+08:00",
    modifiedAt: "2026-06-20T13:35:00+08:00",
    sizeBytes: 244_000_000,
    durationMs: 51_000,
    isFavorite: true,
    isMissing: true,
    fileStatus: "missing",
    tags: ["retake", "smoke"],
    note: "missing drive sample",
    extractedText: "lineup smoke from defender spawn",
    thumbnailTone: "teal",
    thumbnailUrl: null,
  },
];

const tags: Tag[] = [
  { id: "clutch", label: "Clutch", color: "red" },
  { id: "haven", label: "Haven", color: "blue" },
  { id: "pistol", label: "Pistol", color: "gold" },
  { id: "retake", label: "Retake", color: "teal" },
  { id: "smoke", label: "Smoke", color: "green" },
];

test("filters clips by query, source, tag, favorite, missing status, and sort order", () => {
  const result = applyClipFilters(clips, {
    query: "retake",
    sourceDirId: "source-archive",
    tagId: "retake",
    favoritesOnly: true,
    missingOnly: true,
    tags,
    sortBy: "size-desc",
  });

  assert.deepEqual(
    result.map((clip) => clip.id),
    ["clip-3"],
  );
});

test("searches file name, path, source alias, account metadata, tag name, note, and extracted text", () => {
  const baseOptions = {
    sourceDirId: "all",
    tagId: "all",
    favoritesOnly: false,
    missingOnly: false,
    tags,
    sortBy: "modified-desc" as const,
  };

  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "bind-pistol" }).map(
      (clip) => clip.id,
    ),
    ["clip-2"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "E:\\Archive" }).map(
      (clip) => clip.id,
    ),
    ["clip-3"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "Manual Import" }).map(
      (clip) => clip.id,
    ),
    ["clip-2"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "FixtureAlpha" }).map(
      (clip) => clip.id,
    ),
    ["clip-1", "clip-3"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "捷风" }).map(
      (clip) => clip.id,
    ),
    ["clip-2"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "天枢之阙" }).map(
      (clip) => clip.id,
    ),
    ["clip-1"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "Smoke" }).map(
      (clip) => clip.id,
    ),
    ["clip-3"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "fast B" }).map(
      (clip) => clip.id,
    ),
    ["clip-2"],
  );
  assert.deepEqual(
    applyClipFilters(clips, { ...baseOptions, query: "defender spawn" }).map(
      (clip) => clip.id,
    ),
    ["clip-3"],
  );
});

test("filters clips by selected account", () => {
  const result = applyClipFilters(clips, {
    query: "",
    accountId: "FixtureAlpha#0001",
    sourceDirId: "all",
    tagId: "all",
    favoriteFilter: "all",
    fileStatus: "all",
    tags,
    sortBy: "modified-desc",
  });

  assert.deepEqual(
    result.map((clip) => clip.id),
    ["clip-1", "clip-3"],
  );
});

test("filters by favorite status, file status, modified date range, and file size range", () => {
  const result = applyClipFilters(clips, {
    query: "",
    sourceDirId: "all",
    tagId: "all",
    favoriteFilter: "not-favorite",
    fileStatus: "available",
    modifiedFrom: "2026-06-29",
    modifiedTo: "2026-06-30",
    sizeMinBytes: 100_000_000,
    sizeMaxBytes: 150_000_000,
    tags,
    sortBy: "modified-desc",
  });

  assert.deepEqual(
    result.map((clip) => clip.id),
    ["clip-2"],
  );
});

test("sorts clips by newest modification date by default", () => {
  const result = applyClipFilters(clips, {
    query: "",
    sourceDirId: "all",
    tagId: "all",
    favoritesOnly: false,
    missingOnly: false,
    tags,
    sortBy: "modified-desc",
  });

  assert.deepEqual(
    result.map((clip) => clip.id),
    ["clip-2", "clip-1", "clip-3"],
  );
});
