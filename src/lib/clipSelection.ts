import type { ClipSummary } from "../types";

export type ClipSelectionGesture = {
  additive: boolean;
  range: boolean;
};

export type ClipSelectionUpdate = {
  selectedIds: Set<string>;
  anchorId: string;
};

export function updateClipSelection(
  current: ReadonlySet<string>,
  orderedIds: readonly string[],
  clipId: string,
  anchorId: string,
  gesture: ClipSelectionGesture,
): ClipSelectionUpdate {
  if (gesture.range && anchorId) {
    const anchorIndex = orderedIds.indexOf(anchorId);
    const targetIndex = orderedIds.indexOf(clipId);
    if (anchorIndex >= 0 && targetIndex >= 0) {
      const start = Math.min(anchorIndex, targetIndex);
      const end = Math.max(anchorIndex, targetIndex);
      const selectedIds = gesture.additive ? new Set(current) : new Set<string>();
      for (const id of orderedIds.slice(start, end + 1)) selectedIds.add(id);
      return { selectedIds, anchorId };
    }
  }

  const selectedIds = gesture.additive ? new Set(current) : new Set<string>();
  if (selectedIds.has(clipId)) selectedIds.delete(clipId);
  else selectedIds.add(clipId);
  return { selectedIds, anchorId: clipId };
}

export function toggleAllVisibleClipSelection(
  current: ReadonlySet<string>,
  visibleIds: readonly string[],
): Set<string> {
  const selectedIds = new Set(current);
  const allVisibleSelected = visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  for (const id of visibleIds) {
    if (allVisibleSelected) selectedIds.delete(id);
    else selectedIds.add(id);
  }
  return selectedIds;
}

export function pruneClipSelection(
  current: ReadonlySet<string>,
  visibleIds: readonly string[],
): Set<string> {
  const visible = new Set(visibleIds);
  return new Set([...current].filter((id) => visible.has(id)));
}

export function clipTagSelectionState(
  clips: readonly ClipSummary[],
  tagId: string,
): boolean | "indeterminate" {
  const taggedCount = clips.filter((clip) => clip.tags.includes(tagId)).length;
  if (taggedCount === 0) return false;
  return taggedCount === clips.length ? true : "indeterminate";
}
