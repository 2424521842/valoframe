/**
 * Pure ad slot logic: validity windows and weighted rotation.
 *
 * Creative fields arrive from a third-party manifest, so the backend validates them before they
 * are cached (see `src-tauri/src/ads.rs`). This layer only decides what to show and when.
 */

export type AdSlot = "valoframe-sidebar" | "valoframe-library";

export const AD_SLOTS: Readonly<Record<"sidebar" | "library", AdSlot>> = Object.freeze({
  sidebar: "valoframe-sidebar",
  library: "valoframe-library",
});

export type AdCreative = {
  creativeId: string;
  title: string;
  body: string | null;
  advertiserName: string;
  weight: number;
  startAt: string | null;
  endAt: string | null;
  /** `clip-media` protocol path for the cached image; never a vendor URL. */
  imagePath: string;
};

/** Creatives whose flight window covers `now`. Missing bounds mean "no bound". */
export function activeCreatives(
  creatives: readonly AdCreative[],
  now: Date = new Date(),
): AdCreative[] {
  const timestamp = now.getTime();
  if (!Number.isFinite(timestamp)) return [];

  return creatives.filter((creative) => {
    const start = parseTimestamp(creative.startAt);
    const end = parseTimestamp(creative.endAt);
    if (start !== null && timestamp < start) return false;
    if (end !== null && timestamp > end) return false;
    return true;
  });
}

/**
 * Picks a creative by weight. `rotationIndex` walks a deterministic expanded list rather than
 * sampling randomly, so repeated renders of the same slot stay stable and are testable.
 */
export function selectCreative(
  creatives: readonly AdCreative[],
  rotationIndex: number,
): AdCreative | null {
  if (creatives.length === 0) return null;

  const expanded: AdCreative[] = [];
  for (const creative of creatives) {
    // Guard against a manifest weight that survived as a non-integer or absurd value.
    const weight = clampWeight(creative.weight);
    for (let index = 0; index < weight; index += 1) {
      expanded.push(creative);
    }
  }
  if (expanded.length === 0) return null;

  const safeIndex = Number.isFinite(rotationIndex)
    ? Math.abs(Math.trunc(rotationIndex)) % expanded.length
    : 0;
  return expanded[safeIndex] ?? null;
}

/** Weight is expressed in tens so a 100/50 split does not expand into 150 array entries. */
function clampWeight(weight: number): number {
  if (!Number.isFinite(weight) || weight <= 0) return 1;
  return Math.max(1, Math.min(100, Math.round(weight / 10)));
}

function parseTimestamp(value: string | null): number | null {
  if (value === null || value.trim() === "") return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * Splits a user-configured allowlist into hostnames.
 *
 * The backend re-validates every entry before opening a browser; this only tidies input so a
 * stray scheme or trailing slash does not silently block all clicks.
 */
export function parseAllowedHosts(raw: string): string[] {
  return raw
    .split(/[\s,;]+/)
    .map((entry) => entry.trim().toLowerCase())
    .filter((entry) => entry !== "")
    .map((entry) => entry.replace(/^https?:\/\//, "").replace(/\/.*$/, ""))
    .filter((entry) => /^[a-z0-9.-]+$/.test(entry) && entry.includes("."))
    .filter((entry, index, all) => all.indexOf(entry) === index);
}
