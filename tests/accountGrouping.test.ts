import assert from "node:assert/strict";
import test from "node:test";
import { groupClipsByMatch } from "../src/lib/accountGrouping.ts";
import type { Clip } from "../src/types.ts";

const baseClip: Clip = {
  id: "clip-1",
  fileName: "clip.mp4",
  filePath: "D:\\Highlights\\wonderfulVideos-main\\match-a\\clip.mp4",
  sourceDirId: "source-1",
  sourceDirName: "wonderfulVideos-main",
  sourceDirPath: "D:\\Highlights\\wonderfulVideos-main",
  clipGroupId: "match-a",
  clipGroupName: "2026-07-02 22:34",
  accountId: "FixtureAlpha#0001",
  accountName: "FixtureAlpha#0001",
  accountDisplayName: "FixtureAlpha#0001",
  accountSourceName: "wonderfulVideos-main",
  accountDetectedBy: "metadata",
  playerName: "FixtureAlpha#0001",
  agentName: "芮娜",
  agentAvatarUrl: "https://assets.example/reyna.png",
  mapName: "天枢之阙",
  gameMode: "竞技模式",
  scoreline: "11/13",
  kda: "36/17/6",
  hasWon: false,
  matchId: "match-a",
  clipEvents: [],
  createdAt: "2026-07-02T22:34:00+08:00",
  modifiedAt: "2026-07-02T22:35:00+08:00",
  sizeBytes: 75_000_000,
  durationMs: 39_000,
  isFavorite: true,
  isMissing: false,
  fileStatus: "available",
  tags: ["quadra"],
  note: "A 点四杀",
  extractedText: "玩家昵称 FixtureAlpha#0001 地图 天枢之阙",
  thumbnailTone: "red",
  thumbnailUrl: null,
};

test("groups clips by stable account and real match id", () => {
  const groups = groupClipsByMatch([
    {
      ...baseClip,
      id: "clip-a",
      clipGroupId: "folder-a",
      matchId: "match-shared",
      scoreline: "13/11",
      hasWon: false,
    },
    {
      ...baseClip,
      id: "clip-b",
      clipGroupId: "folder-b",
      clipGroupName: "2026-07-02 22:35",
      matchId: "match-shared",
      scoreline: "13/11",
      hasWon: false,
      modifiedAt: "2026-07-02T22:36:00+08:00",
    },
    {
      ...baseClip,
      id: "clip-c",
      accountId: "other-account",
      accountDisplayName: "另一个账号",
      matchId: "match-shared",
    },
  ]);

  assert.equal(groups.length, 2);
  assert.equal(groups[0].id, "match-shared");
  assert.equal(groups[0].clips.length, 2);
  assert.equal(groups[0].resultLabel, "失败");
  assert.equal(groups[1].accountId, "other-account");
});

test("paginated clips merge a match across page boundaries without duplicate headers", () => {
  const groups = groupClipsByMatch([
    { ...baseClip, id: "page-1-a", matchId: "match-shared" },
    { ...baseClip, id: "page-1-b", matchId: "match-other" },
    { ...baseClip, id: "page-2-a", matchId: "match-shared" },
  ]);

  assert.deepEqual(groups.map((group) => group.id), ["match-shared", "match-other"]);
  assert.deepEqual(groups[0].clips.map((clip) => clip.id), ["page-1-a", "page-2-a"]);
});
