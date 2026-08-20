import assetManifest from "../data/valorantAssets.json";

type ValorantAssetEntry = {
  displayName: string;
  uuid: string;
  aliases: string[];
  relativePath: string;
};

const LOCAL_ASSET_BASE = `/${assetManifest.assetRoot.replace(/^public\//, "")}`;
const MAP_PATH_BY_NAME = buildAssetIndex(assetManifest.maps);
const AGENT_PATH_BY_NAME = buildAssetIndex(assetManifest.agents);

export const bundledValorantAssetUrls = Object.freeze([
  ...assetManifest.agents.map(
    ({ relativePath }) => `${LOCAL_ASSET_BASE}/${relativePath}`,
  ),
  ...assetManifest.maps.map(
    ({ relativePath }) => `${LOCAL_ASSET_BASE}/${relativePath}`,
  ),
]);

/** Canonical localized agent labels in manifest order; used by manual import forms. */
export const valorantAgentDisplayNames: readonly string[] = Object.freeze(
  assetManifest.agents.map(({ displayName }) => displayName),
);

/** Canonical localized map labels in manifest order; used by manual import forms. */
export const valorantMapDisplayNames: readonly string[] = Object.freeze(
  assetManifest.maps.map(({ displayName }) => displayName),
);

export function valorantMapListViewIconUrl(mapName: string): string {
  const relativePath = MAP_PATH_BY_NAME.get(normalizeAssetName(mapName));
  return relativePath ? `${LOCAL_ASSET_BASE}/${relativePath}` : "";
}

export function valorantAgentDisplayIconUrl(agentName: string): string {
  const relativePath = AGENT_PATH_BY_NAME.get(normalizeAssetName(agentName));
  return relativePath ? `${LOCAL_ASSET_BASE}/${relativePath}` : "";
}

function buildAssetIndex(entries: readonly ValorantAssetEntry[]): ReadonlyMap<string, string> {
  const index = new Map<string, string>();

  for (const { displayName, aliases, relativePath } of entries) {
    for (const name of [displayName, ...aliases]) {
      index.set(normalizeAssetName(name), relativePath);
    }
  }

  return index;
}

function normalizeAssetName(value: string): string {
  return value.trim().toLocaleLowerCase("zh-CN").replace(/[\s_-]+/g, "");
}
