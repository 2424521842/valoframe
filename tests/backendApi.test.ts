import assert from "node:assert/strict";
import test from "node:test";
import {
  coverPathForClipId,
  coverUrlForClipId,
  isDefaultAclosDirMissing,
  mapBackendClip,
  mapBackendClipSummary,
  mapBackendSource,
  mapBackendTag,
  mediaPathForClipId,
  mergeClipsWithSources,
  scanCommandErrorActiveJobId,
  toClipSummary,
} from "../src/api/backend.ts";
import type { BackendClip, BackendClipSummary, BackendSource, BackendTag, ScanSummary } from "../src/types.ts";

const backendClip: BackendClip = {
  id: 42,
  sourceDirId: 7,
  clipGroupId: 3,
  clipGroupName: "2026-07-02 22:34",
  videoPath:
    "C:\\Users\\Player\\AppData\\ACLOS\\aclos-highlight\\wonderfulVideos-main\\group-a\\ace.mp4",
  normalizedPath:
    "c:/users/player/appdata/aclos/aclos-highlight/wonderfulvideos-main/group-a/ace.mp4",
  fileName: "ace.mp4",
  extension: "mp4",
  fileSize: 123_456_789,
  modifiedAt: "1782634272",
  durationMs: null,
  recordedAt: null,
  coverPath: null,
  coverSource: "missing",
  status: "available",
  favorite: true,
  note: "1v3 clutch",
  extractedText: "ACE highlight transcript",
  accountIdentityKey: "match-account-1001",
  accountIdentitySource: "match-account-id",
  accountDisplayName: "FixtureAlpha#0001",
  openid: null,
  accountName: "FixtureAlpha#0001",
  playerName: "FixtureAlpha#0001",
  agentName: "芮娜",
  mapName: "天枢之阙",
  gameMode: "竞技模式",
  metadataStatus: "enriched",
  matchId: "match-a-001",
  matchAccountId: "1001",
  scoreline: "11/13",
  kda: "36/17/6",
  agentAvatarUrl: "https://assets.example/reyna.png",
  roundLabel: "R03",
  weaponName: "Vandal",
  killCount: 3,
  matchStartedAt: "2026-06-28T08:00:00Z",
  combatScore: 287,
  hasWon: false,
  eventCount: 12,
  clipEvents: [
    {
      id: 901,
      eventType: "kill",
      videoTimeMs: 6_000,
      eventTime: "2026-06-28T08:01:00Z",
      roundId: 3,
      playerName: "FixtureAlpha#0001",
      weaponName: "Vandal",
      killerName: "FixtureAlpha#0001",
      killedName: "Opponent#0001",
      killerIsMe: true,
    },
  ],
  tagIds: [1, 2],
};

const backendSource: BackendSource = {
  id: 7,
  path: "D:\\Direct Clips",
  displayName: "wonderfulVideos-main",
  enabled: true,
  status: "available",
  accessibility: true,
  lastError: null,
  clipCount: 1,
  lastScanAt: "2026-07-02 22:35:00",
};

const backendTag: BackendTag = {
  id: 1,
  name: "ACE",
  color: "red",
};

test("reads the active job id from an already-running command error", () => {
  assert.equal(scanCommandErrorActiveJobId({ activeJobId: "scan-existing" }), "scan-existing");
  assert.equal(scanCommandErrorActiveJobId({ activeJobId: null }), null);
  assert.equal(scanCommandErrorActiveJobId({ jobId: "scan-failed" }), null);
});

test("maps backend clips into the frontend clip contract", () => {
  const clip = mergeClipsWithSources(
    [mapBackendClip(backendClip)],
    [mapBackendSource(backendSource)],
  )[0];

  assert.equal(clip.id, "42");
  assert.equal(clip.fileName, "ace.mp4");
  assert.equal(clip.filePath, backendClip.videoPath);
  assert.equal(clip.sourceDirId, "7");
  assert.equal(clip.sourceDirName, "wonderfulVideos-main");
  assert.equal(clip.sourceDirPath, "D:\\Direct Clips");
  assert.equal(clip.clipGroupId, "3");
  assert.equal(clip.clipGroupName, "2026-07-02 22:34");
  assert.equal(clip.accountDisplayName, "FixtureAlpha#0001");
  assert.equal(clip.accountId, "match-account-1001");
  assert.equal(clip.accountIdentitySource, "match-account-id");
  assert.equal(clip.openid, null);
  assert.equal(clip.accountDetectedBy, "metadata");
  assert.equal(clip.accountSourceName, "wonderfulVideos-main");
  assert.equal(clip.playerName, "FixtureAlpha#0001");
  assert.equal(clip.agentName, "芮娜");
  assert.equal(clip.mapName, "天枢之阙");
  assert.equal(clip.gameMode, "竞技模式");
  assert.equal(clip.metadataStatus, "enriched");
  assert.equal(clip.matchId, "match-a-001");
  assert.equal(clip.matchAccountId, "1001");
  assert.equal(clip.scoreline, "11/13");
  assert.equal(clip.kda, "36/17/6");
  assert.equal(clip.agentAvatarUrl, "https://assets.example/reyna.png");
  assert.equal(clip.roundLabel, "R03");
  assert.equal(clip.weaponName, "Vandal");
  assert.equal(clip.killCount, 3);
  assert.equal(clip.matchStartedAt, "2026-06-28T08:00:00.000Z");
  assert.equal(clip.combatScore, 287);
  assert.equal(clip.hasWon, false);
  assert.equal(clip.eventCount, 12);
  assert.deepEqual(clip.clipEvents, [
    {
      id: "901",
      eventType: "kill",
      videoTimeMs: 6_000,
      eventTime: "2026-06-28T08:01:00Z",
      roundId: 3,
      playerName: "FixtureAlpha#0001",
      weaponName: "Vandal",
      killerName: "FixtureAlpha#0001",
      killedName: "Opponent#0001",
      killerIsMe: true,
    },
  ]);
  assert.deepEqual(Object.keys(clip).filter((key) => key.endsWith("Events")), ["clipEvents"]);
  assert.equal(clip.sizeBytes, 123_456_789);
  assert.equal(clip.durationMs, null);
  assert.equal(clip.modifiedAt, "2026-06-28T08:11:12.000Z");
  assert.equal(clip.isFavorite, true);
  assert.equal(clip.isMissing, false);
  assert.equal(clip.fileStatus, "available");
  assert.deepEqual(clip.tags, ["1", "2"]);
  assert.equal(clip.note, "1v3 clutch");
  assert.equal(clip.extractedText, "ACE highlight transcript");
  assert.equal(clip.thumbnailUrl, null);
});

test("maps generated thumbnails to revisioned protocol URLs without requiring a source cover", () => {
  const clip = mapBackendClip({
    ...backendClip,
    coverPath: null,
    thumbnailStatus: "ready",
    thumbnailRevision: "rev 1",
  });

  assert.equal(clip.thumbnailStatus, "ready");
  assert.equal(clip.thumbnailRevision, "rev 1");
  assert.match(clip.thumbnailUrl ?? "", /cover\/42\?v=rev%201$/);

  const sourceCover = mapBackendClip({
    ...backendClip,
    coverPath: "D:\\covers\\cover-ace.jpeg",
    coverSource: "file",
    thumbnailStatus: "unavailable",
    thumbnailRevision: null,
  });
  assert.match(sourceCover.thumbnailUrl ?? "", /cover\/42$/);
});

test("maps list payloads without retaining detail-only note, OCR, or event fields", () => {
  const backendSummary: BackendClipSummary = {
    id: backendClip.id,
    sourceDirId: backendClip.sourceDirId,
    sourceDirPath: backendSource.path,
    sourceDirName: backendSource.displayName,
    clipGroupId: backendClip.clipGroupId,
    clipGroupName: backendClip.clipGroupName,
    videoPath: backendClip.videoPath,
    fileName: backendClip.fileName,
    fileSize: backendClip.fileSize,
    modifiedAt: backendClip.modifiedAt,
    durationMs: backendClip.durationMs,
    recordedAt: backendClip.recordedAt,
    coverPath: backendClip.coverPath,
    coverSource: backendClip.coverSource,
    thumbnailStatus: "ready",
    thumbnailRevision: "summary-rev",
    status: backendClip.status,
    favorite: backendClip.favorite,
    accountIdentityKey: backendClip.accountIdentityKey,
    accountIdentitySource: backendClip.accountIdentitySource,
    accountDisplayName: backendClip.accountDisplayName,
    openid: backendClip.openid,
    accountName: backendClip.accountName,
    playerName: backendClip.playerName,
    agentName: backendClip.agentName,
    mapName: backendClip.mapName,
    gameMode: backendClip.gameMode,
    metadataStatus: backendClip.metadataStatus,
    matchId: backendClip.matchId,
    matchAccountId: backendClip.matchAccountId,
    scoreline: backendClip.scoreline,
    kda: backendClip.kda,
    agentAvatarUrl: backendClip.agentAvatarUrl,
    killCount: backendClip.killCount,
    matchStartedAt: backendClip.matchStartedAt,
    combatScore: backendClip.combatScore,
    hasWon: backendClip.hasWon,
    officialVideoName: backendClip.officialVideoName,
    officialVideoType: backendClip.officialVideoType,
    highlightType: backendClip.highlightType,
    roundScore: backendClip.roundScore,
    metadataSource: backendClip.metadataSource,
    tagIds: backendClip.tagIds,
  };
  const summary = mapBackendClipSummary(backendSummary);

  assert.equal(summary.sourceDirName, "wonderfulVideos-main");
  assert.equal(summary.sourceDirPath, backendSource.path);
  assert.equal(summary.thumbnailStatus, "ready");
  assert.equal(summary.thumbnailRevision, "summary-rev");
  assert.match(summary.thumbnailUrl ?? "", /cover\/42\?v=summary-rev$/);
  for (const detailField of ["note", "extractedText", "clipEvents", "eventCount", "roundLabel", "weaponName"]) {
    assert.equal(detailField in summary, false, `${detailField} must not survive summary mapping`);
  }
  const fromFullClip = toClipSummary(mapBackendClip(backendClip));
  assert.equal(fromFullClip.id, summary.id);
  assert.equal("note" in fromFullClip, false);
});

test("maps official WonderfulDb metadata", () => {
  const clip = mapBackendClip({
    ...backendClip,
    officialVideoName: "六杀时刻",
    officialVideoType: "五杀时刻",
    highlightType: 10,
    roundScore: 1670,
    metadataSource: "wonderful_db",
    clipEvents: [
      {
        id: 1,
        eventType: "kill",
        videoTimeMs: 6000,
        eventTime: "2026-07-03 17:20:55.495",
        roundId: 2,
        playerName: "Tester #1001",
        weaponName: "Vandal",
        killerName: "Tester",
        killedName: "Enemy",
        killerIsMe: true,
      },
    ],
  });

  assert.equal(clip.officialVideoName, "六杀时刻");
  assert.equal(clip.officialVideoType, "五杀时刻");
  assert.equal(clip.highlightType, 10);
  assert.equal(clip.roundScore, 1670);
  assert.equal(clip.metadataSource, "wonderful_db");
  assert.deepEqual(clip.clipEvents, [
    {
      id: "1",
      eventType: "kill",
      videoTimeMs: 6000,
      eventTime: "2026-07-03 17:20:55.495",
      roundId: 2,
      playerName: "Tester #1001",
      weaponName: "Vandal",
      killerName: "Tester",
      killedName: "Enemy",
      killerIsMe: true,
    },
  ]);
});

test("maps missing or blank backend event types to a neutral event", () => {
  const clip = mapBackendClip({
    ...backendClip,
    clipEvents: [
      { ...backendClip.clipEvents[0], eventType: "" },
      { ...backendClip.clipEvents[0], id: 902, eventType: undefined as unknown as string },
    ],
  });

  assert.deepEqual(
    clip.clipEvents.map((event) => event.eventType),
    ["event", "event"],
  );
});

test("uses match account id as the account fallback before source directory names", () => {
  const clip = mapBackendClip({
    ...backendClip,
    accountIdentityKey: "match-account-1001",
    accountIdentitySource: "match-account-id",
    accountDisplayName: "账号 1001",
    accountName: null,
    playerName: null,
    matchAccountId: "1001",
  });

  assert.equal(clip.accountId, "match-account-1001");
  assert.equal(clip.accountDisplayName, "账号 1001");
  assert.equal(clip.accountDetectedBy, "metadata");
  assert.equal(clip.accountSourceName, "来源 7");
});

test("keeps backend display names separate from stable account identity", () => {
  const clip = mapBackendClip({
    ...backendClip,
    accountDisplayName: "FixtureBravo#0002",
    accountName: "FixtureBravo#0002",
    playerName: "FixtureBravo",
  });

  assert.equal(clip.accountId, "match-account-1001");
  assert.equal(clip.accountDisplayName, "FixtureBravo#0002");
  assert.equal(clip.playerName, "FixtureBravo");
});

test("normalizes English agent names to Chinese display names", () => {
  const clip = mapBackendClip({
    ...backendClip,
    agentName: "Jett",
  });

  assert.equal(clip.agentName, "捷风");
});

test("normalizes KAY/O before treating slashes as asset paths", () => {
  const clip = mapBackendClip({
    ...backendClip,
    agentName: "KAY/O",
  });

  assert.equal(clip.agentName, "K/O");
});

test("keeps localized Chinese agent names from the backend", () => {
  const clip = mapBackendClip({
    ...backendClip,
    agentName: "贤者",
  });

  assert.equal(clip.agentName, "贤者");
});

test("normalizes the current Miks agent name", () => {
  const clip = mapBackendClip({
    ...backendClip,
    agentName: "miks",
  });

  assert.equal(clip.agentName, "迷核");
});

test("drops unknown agent names instead of showing raw backend values", () => {
  const clip = mapBackendClip({
    ...backendClip,
    agentName: "future-agent",
  });

  assert.equal(clip.agentName, "");
});

test("maps backend tags into the frontend tag contract", () => {
  assert.deepEqual(mapBackendTag(backendTag), {
    id: "1",
    label: "ACE",
    color: "red",
  });

  assert.deepEqual(
    mapBackendTag({ ...backendTag, id: 2, name: "Custom", color: null }),
    {
      id: "2",
      label: "Custom",
      color: "blue",
    },
  );
});

test("maps source DTOs without requiring clips, including zero-clip sources", () => {
  assert.deepEqual(mapBackendSource({ ...backendSource, clipCount: 0 }), {
    id: "7",
    name: "wonderfulVideos-main",
    displayName: "wonderfulVideos-main",
    path: "D:\\Direct Clips",
    enabled: true,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 0,
    lastScanAt: "2026-07-02 22:35:00",
  });
});

test("detects the missing default ACLOS directory summary", () => {
  const summary: ScanSummary = {
    rootPath: "C:\\Users\\Player\\AppData\\ACLOS\\aclos-highlight",
    sourceDirCount: 0,
    clipGroupCount: 0,
    newClipCount: 0,
    updatedClipCount: 0,
    missingClipCount: 0,
    coverMissingCount: 0,
    errors: [],
    message:
      "Scan root not found: C:\\Users\\Player\\AppData\\ACLOS\\aclos-highlight",
  };

  assert.equal(isDefaultAclosDirMissing(summary), true);
});

test("builds clip media paths from clip ids instead of local file paths", () => {
  assert.equal(mediaPathForClipId("42"), "clip/42");
  assert.equal(coverPathForClipId("42"), "cover/42");
  assert.match(coverUrlForClipId("42", "revision/2"), /cover\/42\?v=revision%2F2$/);
  assert.equal(mediaPathForClipId("C:\\Clips\\ace.mp4"), "clip/C%3A%5CClips%5Cace.mp4");
});
