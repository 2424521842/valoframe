import type {
  ClipSummary,
  LibraryTagFacet,
  Tag,
  TagColor,
} from "../types";

export const TAG_COLORS: readonly TagColor[] = [
  "red",
  "teal",
  "gold",
  "blue",
  "green",
];

export function countTagUsage(clips: readonly ClipSummary[]): Map<string, number> {
  const counts = new Map<string, number>();

  for (const clip of clips) {
    for (const tagId of new Set(clip.tags)) {
      counts.set(tagId, (counts.get(tagId) ?? 0) + 1);
    }
  }

  return counts;
}

export function removeTagFromClipCollection(
  clips: readonly ClipSummary[],
  tagId: string,
): ClipSummary[] {
  return clips.map((clip) =>
    clip.tags.includes(tagId)
      ? { ...clip, tags: clip.tags.filter((candidate) => candidate !== tagId) }
      : clip,
  );
}

export function filterCustomTags(
  tags: readonly Tag[],
  query: string,
): Tag[] {
  const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");

  return tags
    .filter((tag) =>
      !normalizedQuery || tag.label.toLocaleLowerCase("zh-CN").includes(normalizedQuery),
    )
    .sort((left, right) => left.label.localeCompare(right.label, "zh-CN"));
}

export function mergeTagsWithFacets(
  tags: readonly Tag[],
  facets: readonly LibraryTagFacet[] | undefined,
  selectedTagId: string,
): Tag[] {
  const merged = tags.map((tag) => ({ ...tag }));
  const knownIds = new Set(merged.map((tag) => tag.id));

  for (const facet of facets ?? []) {
    const id = String(facet.id);
    if (knownIds.has(id)) continue;

    knownIds.add(id);
    merged.push({
      id,
      label: facet.name,
      color: facetTagColor(facet.color),
    });
  }

  if (selectedTagId !== "all" && !knownIds.has(selectedTagId)) {
    merged.push({ id: selectedTagId, label: selectedTagId, color: "blue" });
  }

  return merged;
}

function facetTagColor(value: string | null): TagColor {
  return value === "red" ||
    value === "teal" ||
    value === "gold" ||
    value === "blue" ||
    value === "green"
    ? value
    : "blue";
}
