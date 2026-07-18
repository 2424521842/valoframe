import type { ClipMatchGroup, ClipSummary } from "../types";

export function groupClipsByMatch(clips: ClipSummary[]): ClipMatchGroup[] {
  const byMatch = new Map<string, { id: string; clips: ClipSummary[] }>();

  for (const clip of clips) {
    const id = clip.matchId?.trim() || clip.clipGroupId || `clip-${clip.id}`;
    const key = `${clip.accountId}:${id}`;
    const group = byMatch.get(key) ?? { id, clips: [] };
    group.clips.push(clip);
    byMatch.set(key, group);
  }

  // Map insertion order follows the backend's stable ordering. A later page can
  // append to an existing match without creating another header or moving it.
  return [...byMatch.values()].map(({ id, clips: groupClips }) =>
    createMatchGroup(id, groupClips),
  );
}

function createMatchGroup(
  id: string,
  groupClips: ClipSummary[],
): ClipMatchGroup {
  const chronologicalClips = groupClips
    .slice()
    .sort((left, right) => timestamp(right.modifiedAt) - timestamp(left.modifiedAt));
  const sortedClips = chronologicalClips;
  const primaryClip = sortedClips[0];
  const metadataClip =
    sortedClips.find((clip) => clip.metadataStatus === "enriched") ??
    primaryClip;
  const latestModifiedAt = sortedClips.reduce(
    (latest, clip) =>
      timestamp(clip.modifiedAt) > timestamp(latest)
        ? clip.modifiedAt
        : latest,
    primaryClip.modifiedAt,
  );

  return {
    id,
    accountId: primaryClip.accountId,
    accountDisplayName: primaryClip.accountDisplayName,
    title: `${firstText(sortedClips, "agentName") || "未知英雄"} · ${
      matchDisplayName(metadataClip)
    }`,
    subtitle: `${primaryClip.accountDisplayName} · ${
      firstText(sortedClips, "gameMode") || "未知模式"
    }`,
    clips: groupClips.slice(),
    latestModifiedAt,
    totalSizeBytes: sortedClips.reduce(
      (total, clip) => total + clip.sizeBytes,
      0,
    ),
    resultLabel: resultLabelFromClips(sortedClips),
    scoreline: firstText(sortedClips, "scoreline"),
    kda: firstText(sortedClips, "kda"),
    mapName: firstText(sortedClips, "mapName"),
    gameMode: firstText(sortedClips, "gameMode"),
    agentName: firstText(sortedClips, "agentName"),
    agentAvatarUrl: firstText(sortedClips, "agentAvatarUrl"),
  };
}

function firstText(
  clips: ClipSummary[],
  field: keyof Pick<
    ClipSummary,
    "agentName" | "mapName" | "gameMode" | "scoreline" | "kda" | "agentAvatarUrl"
  >,
): string {
  return clips.find((clip) => (clip[field] ?? "").trim())?.[field] ?? "";
}

function matchDisplayName(clip: ClipSummary): string {
  return clip.matchStartedAt || clip.clipGroupName || clip.matchId || clip.fileName;
}

function resultLabelFromClips(clips: ClipSummary[]): string {
  const clipWithResult = clips.find((clip) => typeof clip.hasWon === "boolean");

  if (clipWithResult?.hasWon === true) {
    return "胜利";
  }

  if (clipWithResult?.hasWon === false) {
    return "失败";
  }

  return resultLabelFromScoreline(firstText(clips, "scoreline"));
}

function resultLabelFromScoreline(scoreline: string): string {
  const match = scoreline.match(/(\d+)\s*[/:-]\s*(\d+)/);

  if (!match) {
    return "未知";
  }

  const left = Number(match[1]);
  const right = Number(match[2]);

  if (left > right) {
    return "胜利";
  }

  if (left < right) {
    return "失败";
  }

  return "平局";
}

function timestamp(value: string): number {
  const parsed = new Date(value).getTime();
  return Number.isNaN(parsed) ? 0 : parsed;
}
