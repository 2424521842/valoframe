import type { LibraryFacets } from "../../src/types";

export function libraryFacets(
  overrides: Partial<LibraryFacets> = {},
): LibraryFacets {
  return {
    totalCount: 0,
    activeCount: 0,
    favoriteCount: 0,
    activeFavoriteCount: 0,
    trashedCount: 0,
    taggedCount: 0,
    activeTaggedCount: 0,
    totalSizeBytes: 0,
    activeSizeBytes: 0,
    sizeBytesMin: null,
    sizeBytesMax: null,
    recentCount: 0,
    recordedAtMin: null,
    recordedAtMax: null,
    modifiedAtMin: null,
    modifiedAtMax: null,
    fileStatuses: [],
    metadataStatuses: [],
    accounts: [],
    sourceDirs: [],
    agents: [],
    maps: [],
    gameModes: [],
    killTypes: [],
    tags: [],
    ...overrides,
  };
}
