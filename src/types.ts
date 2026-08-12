export type SourceKind = "aclos" | "nvidia" | "tracker" | "generic";

export type ScanMode = "aclos-structured" | "recursive-mp4";

export type ReviewDecision = "unreviewed" | "liked" | "disliked";

/**
 * A decision made inside a single quick-pick session. It deliberately does not
 * share the legacy clip-level `ReviewDecision` values above: a quick-pick is a
 * temporary editorial decision, never a favorite, tag, or recycle-bin action.
 */
export type ReviewItemDecision = "unreviewed" | "selected" | "pending" | "skipped";

export type ReviewSessionStatus = "active" | "completed";

export type ReviewSessionSort =
  | "library"
  | "latest"
  | "oldest"
  | "kills"
  | "score";

export type ReviewCandidateScope = "all" | "not-selected" | "recent";

export type SourceDir = {
  id: string;
  name: string;
  displayName: string;
  path: string;
  sourceKind: SourceKind;
  scanMode: ScanMode;
  scanRootPath: string;
  enabled: boolean;
  status: string;
  accessibility: boolean;
  lastError: string | null;
  clipCount: number;
  lastScanAt: string | null;
};

export type RegisterScanSourceInput = {
  sourceKind: SourceKind;
  scanRootPath: string;
  displayName: string;
  enabled: boolean;
  allowOverlap?: boolean;
};

export type ScanSourceOverlap = {
  id: string;
  displayName: string;
  sourceKind: SourceKind;
  scanRootPath: string;
};

export type RegisterScanSourceResult = {
  sources: SourceDir[];
  createdCount: number;
  duplicateCount: number;
  normalizedRootPath: string;
  requiresOverlapConfirmation: boolean;
  overlaps: ScanSourceOverlap[];
};

export type SourceRelocationConflict = {
  code: string;
  message: string;
  oldClipIds: string[];
  candidatePaths: string[];
};

export type SourceRelocationBlocker = {
  code: string;
  message: string;
};

export type AffectedRelocationSource = {
  id: string;
  displayName: string;
  oldSourcePath: string;
  newSourcePath: string;
  clipCount: number;
};

export type ScanSourceRelocationPreview = {
  sourceId: string;
  oldRootPath: string;
  newRootPath: string;
  affectedSources: AffectedRelocationSource[];
  exactPathMatchCount: number;
  identityMatchCount: number;
  legacyFingerprintMatchCount: number;
  unmatchedCount: number;
  newCandidateCount: number;
  expectedClipUpdateCount: number;
  expectedGroupUpdateCount: number;
  expectedCoverUpdateCount: number;
  expectedMetadataReferenceUpdateCount: number;
  conflicts: SourceRelocationConflict[];
  blockers: SourceRelocationBlocker[];
  canRelocate: boolean;
};

export type RelocateScanSourceResult = {
  preview: ScanSourceRelocationPreview;
  relocatedClipCount: number;
  syncJobId: string | null;
  syncStarted: boolean;
  syncStatus: Extract<
    ScanJobStatus,
    "completed" | "partial" | "cancelled" | "failed"
  > | null;
  syncMessage: string | null;
};

export type AccountIdentitySource = "match-account-id" | "openid" | "source-dir";

export type TagColor = "red" | "teal" | "gold" | "blue" | "green";

export type Tag = {
  id: string;
  label: string;
  color: TagColor;
};

export type ThumbnailTone = "red" | "teal" | "gold" | "blue" | "green";

export type ThumbnailStatus =
  | "pending"
  | "running"
  | "ready"
  | "failed"
  | "unavailable"
  | "evicted"
  | string;

export type Clip = {
  id: string;
  fileName: string;
  filePath: string;
  sourceDirId: string;
  sourceDirName: string;
  sourceDirPath: string;
  sourceKind: SourceKind;
  scanMode: ScanMode;
  scanRootPath: string;
  sourceRelativeDir: string;
  clipGroupId: string | null;
  clipGroupName: string;
  accountId: string;
  accountIdentitySource: AccountIdentitySource;
  openid: string | null;
  accountName: string;
  accountDisplayName: string;
  accountSourceName: string;
  accountDetectedBy: "metadata" | "source-dir" | "fallback";
  playerName: string;
  agentName: string;
  mapName: string;
  gameMode: string;
  metadataStatus?: string;
  matchId?: string;
  matchAccountId?: string;
  scoreline: string;
  kda: string;
  agentAvatarUrl?: string;
  roundLabel?: string;
  weaponName?: string;
  killCount?: number | null;
  matchStartedAt?: string | null;
  combatScore?: number | null;
  hasWon?: boolean | null;
  officialVideoName?: string | null;
  officialVideoType?: string | null;
  highlightType?: number | null;
  roundScore?: number | null;
  metadataSource?: string | null;
  eventCount?: number;
  clipEvents?: ClipEvent[];
  createdAt: string;
  modifiedAt: string;
  sizeBytes: number;
  durationMs: number | null;
  isFavorite: boolean;
  reviewDecision: ReviewDecision;
  reviewedAt: string | null;
  isMissing: boolean;
  fileStatus: string;
  tags: string[];
  note: string;
  extractedText: string;
  thumbnailTone: ThumbnailTone;
  thumbnailUrl: string | null;
  thumbnailStatus?: ThumbnailStatus;
  thumbnailRevision?: string | null;
};

/**
 * Production list payload. Detail-only text and event fields are intentionally
 * absent so paginated library state cannot accidentally retain full clips.
 */
export type ClipSummary = Omit<
  Clip,
  | "roundLabel"
  | "weaponName"
  | "eventCount"
  | "clipEvents"
  | "note"
  | "extractedText"
>;

export type ClipDetail = Clip;

export type ClipEvent = {
  id: string;
  eventType: string;
  videoTimeMs: number | null;
  eventTime: string | null;
  roundId: number | null;
  playerName: string;
  weaponName: string;
  killerName: string;
  killedName: string;
  killerIsMe: boolean;
  killedIsMe: boolean;
};

export type AccountSummary = {
  id: string;
  displayName: string;
  sourceName: string;
  clipCount: number;
  missingCount: number;
  favoriteCount: number;
  sizeBytes: number;
  lastModifiedAt: string;
  detectedBy: Clip["accountDetectedBy"];
};

export type ClipMatchGroup = {
  id: string;
  accountId: string;
  accountDisplayName: string;
  title: string;
  subtitle: string;
  clips: ClipSummary[];
  latestModifiedAt: string;
  totalSizeBytes: number;
  resultLabel: string;
  scoreline: string;
  kda: string;
  mapName: string;
  gameMode: string;
  agentName: string;
  agentAvatarUrl: string;
};

export type ClipSort =
  | "modified-desc"
  | "modified-asc"
  | "size-desc"
  | "size-asc"
  | "name-asc";

export type FavoriteFilter = "all" | "favorite" | "not-favorite";

export type LibraryMode = "all" | "today" | "favorites" | "missing" | "trash";

export type AppScreen = "scan" | "library" | "review" | "tags" | "settings" | "preview";

export type LibraryViewMode = "grid" | "list";

export type LibraryDatePreset = "all" | "today" | "week" | "month";

export type HighlightFilter =
  | "all"
  | "triple"
  | "quad"
  | "five"
  | "six"
  | "kill-compilation"
  | "death";

export type ScanTarget = {
  id: string;
  name: string;
  path: string;
  origin: "indexed" | "manual";
};

export type BackendClip = {
  id: number;
  sourceDirId: number;
  sourceKind: SourceKind;
  scanMode: ScanMode;
  scanRootPath: string;
  sourceRelativeDir: string;
  clipGroupId: number | null;
  clipGroupName?: string | null;
  videoPath: string;
  normalizedPath: string;
  fileName: string;
  extension: string;
  fileSize: number;
  modifiedAt: string | null;
  durationMs: number | null;
  recordedAt: string | null;
  coverPath: string | null;
  coverSource: string;
  thumbnailStatus?: string | null;
  thumbnailRevision?: string | null;
  status: string;
  favorite: boolean;
  reviewDecision: ReviewDecision;
  reviewedAt: string | null;
  note: string | null;
  extractedText?: string | null;
  accountIdentityKey: string;
  accountIdentitySource: AccountIdentitySource;
  accountDisplayName: string;
  openid: string | null;
  accountName?: string | null;
  playerName?: string | null;
  agentName?: string | null;
  mapName?: string | null;
  gameMode?: string | null;
  metadataStatus?: string | null;
  matchId?: string | null;
  matchAccountId?: string | null;
  scoreline?: string | null;
  kda?: string | null;
  agentAvatarUrl?: string | null;
  roundLabel?: string | null;
  weaponName?: string | null;
  killCount?: number | null;
  matchStartedAt?: string | null;
  combatScore?: number | null;
  hasWon?: boolean | null;
  officialVideoName?: string | null;
  officialVideoType?: string | null;
  highlightType?: number | null;
  roundScore?: number | null;
  metadataSource?: string | null;
  eventCount?: number | null;
  clipEvents?: BackendClipEvent[] | null;
  tagIds: number[];
};

/** Lightweight payload returned by `list_clip_page`; detail-only text and events are omitted. */
export type BackendClipSummary = Omit<
  BackendClip,
  | "normalizedPath"
  | "extension"
  | "note"
  | "extractedText"
  | "roundLabel"
  | "weaponName"
  | "eventCount"
  | "clipEvents"
> & {
  sourceDirPath: string;
  sourceDirName: string;
};

export type ClipListQuery = {
  offset?: number;
  limit?: number;
  query?: string;
  /** Stable `accountIdentityKey`, not a display label. */
  accountId?: string;
  sourceDirId?: number;
  agentName?: string;
  mapName?: string;
  gameMode?: string;
  tagId?: number;
  highlightFilter?: HighlightFilter;
  favoriteFilter?: FavoriteFilter;
  reviewDecision?: ReviewDecision;
  fileStatus?: string;
  metadataStatus?: string;
  /** Inclusive Unix timestamp in seconds. */
  modifiedFrom?: number;
  /** Inclusive Unix timestamp in seconds. */
  modifiedTo?: number;
  sizeMinBytes?: number;
  sizeMaxBytes?: number;
  sortBy?: ClipSort;
};

/** A frozen, serializable record of one quick-pick round. */
export type ReviewSessionFilters = {
  /** The library query at the moment the session was started, without pagination. */
  query: Omit<ClipListQuery, "offset" | "limit" | "reviewDecision">;
  labels: string[];
  sort: ReviewSessionSort;
  candidateScope: ReviewCandidateScope;
};

export type ReviewSessionItem = {
  videoId: string;
  decision: ReviewItemDecision;
};

export type ReviewSession = {
  id: string;
  createdAt: string;
  updatedAt: string;
  filters: ReviewSessionFilters;
  totalCount: number;
  /** Index in `items` of the active candidate; `totalCount` means the pass is complete. */
  currentIndex: number;
  status: ReviewSessionStatus;
  items: ReviewSessionItem[];
};

export type ReviewQueueQuery = {
  accountId?: string;
  agentName?: string;
  mapName?: string;
  gameMode?: string;
  sourceDirIds?: number[];
  tagIds?: number[];
  /** Inclusive Unix timestamp in seconds, using the effective recorded time. */
  recordedFrom?: number;
  /** Inclusive Unix timestamp in seconds, using the effective recorded time. */
  recordedTo?: number;
  snapshotMaxClipId?: number;
  cursor?: string;
  limit?: number;
};

export type BackendClipPage = {
  items: BackendClipSummary[];
  offset: number;
  limit: number;
  totalCount: number;
  hasMore: boolean;
  nextOffset: number | null;
};

export type ClipPage = Omit<BackendClipPage, "items"> & {
  items: ClipSummary[];
};

export type BackendReviewClipPage = {
  items: BackendClipSummary[];
  snapshotMaxClipId: number;
  candidateCount: number;
  limit: number;
  hasMore: boolean;
  nextCursor: string | null;
};

export type ReviewClipPage = Omit<BackendReviewClipPage, "items"> & {
  items: ClipSummary[];
};

export type BackendReviewClipState = {
  clipId: number;
  reviewDecision: ReviewDecision;
  reviewedAt: string | null;
  favorite: boolean;
};

export type ReviewClipState = Omit<BackendReviewClipState, "clipId"> & {
  clipId: string;
};

export type BackendReviewDecisionMutation = {
  before: BackendReviewClipState;
  after: BackendReviewClipState;
  changed: boolean;
};

export type ReviewDecisionMutation = Omit<
  BackendReviewDecisionMutation,
  "before" | "after"
> & {
  before: ReviewClipState;
  after: ReviewClipState;
};

/** Whole-index counts include trashed clips; activeCount excludes only trashed clips. */
export type LibraryFacetValue = {
  value: string;
  count: number;
  activeCount: number;
};

export type LibraryAccountFacet = {
  accountIdentityKey: string;
  accountDisplayName: string;
  count: number;
  activeCount: number;
};

export type LibrarySourceFacet = {
  sourceDirId: number | string;
  count: number;
  activeCount: number;
};

export type LibraryTagFacet = {
  id: number | string;
  name: string;
  color: string | null;
  count: number;
  activeCount: number;
};

export type LibraryFacets = {
  /** Every indexed clip, including missing and trashed records. */
  totalCount: number;
  /** Every indexed clip except records in the recycle bin. */
  activeCount: number;
  favoriteCount: number;
  activeFavoriteCount: number;
  trashedCount: number;
  taggedCount: number;
  activeTaggedCount: number;
  totalSizeBytes: number;
  activeSizeBytes: number;
  sizeBytesMin: number | null;
  sizeBytesMax: number | null;
  recentCount: number;
  recordedAtMin: number | null;
  recordedAtMax: number | null;
  modifiedAtMin: number | null;
  modifiedAtMax: number | null;
  fileStatuses: LibraryFacetValue[];
  metadataStatuses: LibraryFacetValue[];
  accounts: LibraryAccountFacet[];
  sourceDirs: LibrarySourceFacet[];
  agents: LibraryFacetValue[];
  maps: LibraryFacetValue[];
  gameModes: LibraryFacetValue[];
  killTypes: LibraryFacetValue[];
  tags: LibraryTagFacet[];
};

export type BackendClipDetail = BackendClip & {
  tags: BackendTag[];
};

export type ClipDetailCommandError = {
  code: "clip-not-found" | "database-error" | string;
  message: string;
  clipId: number;
};

export type BackendSource = {
  id: number;
  path: string;
  displayName: string;
  sourceKind: SourceKind;
  scanMode: ScanMode;
  scanRootPath: string;
  enabled: boolean;
  status: string;
  accessibility: boolean;
  lastError: string | null;
  clipCount: number;
  lastScanAt: string | null;
};

export type BackendScanSourceOverlap = Omit<ScanSourceOverlap, "id"> & {
  id: number;
};

export type BackendRegisterScanSourceResult = Omit<
  RegisterScanSourceResult,
  "sources" | "overlaps"
> & {
  sources: BackendSource[];
  overlaps: BackendScanSourceOverlap[];
};

export type BackendAffectedRelocationSource = Omit<AffectedRelocationSource, "id"> & {
  id: number;
};

export type BackendScanSourceRelocationPreview = Omit<
  ScanSourceRelocationPreview,
  "sourceId" | "affectedSources"
> & {
  sourceId: number;
  affectedSources: BackendAffectedRelocationSource[];
};

export type BackendRelocateScanSourceResult = Omit<
  RelocateScanSourceResult,
  "preview"
> & {
  preview: BackendScanSourceRelocationPreview;
};

export type BackendClipEvent = {
  id: number;
  eventType: string;
  videoTimeMs?: number | null;
  eventTime?: string | null;
  roundId?: number | null;
  playerName?: string | null;
  weaponName?: string | null;
  killerName?: string | null;
  killedName?: string | null;
  killerIsMe: boolean;
  killedIsMe: boolean;
};

export type BackendTag = {
  id: number;
  name: string;
  color: string | null;
};

export type BackendBatchMutationResult = {
  requested: number;
  matched: number;
  updated: number;
  missingIds: number[];
  clips: BackendClip[];
};

export type BatchMutationResult = {
  requested: number;
  matched: number;
  updated: number;
  missingIds: string[];
  clips: Clip[];
};

export type BackendIndexRemovalProblem = {
  clipId: number;
  code: string;
  message: string;
};

export type BackendRemoveClipsFromIndexResult = {
  requested: number;
  removedIds: number[];
  missingIds: number[];
  blocked: BackendIndexRemovalProblem[];
  failures: BackendIndexRemovalProblem[];
};

export type IndexRemovalProblem = Omit<BackendIndexRemovalProblem, "clipId"> & {
  clipId: string;
};

export type RemoveClipsFromIndexResult = Omit<
  BackendRemoveClipsFromIndexResult,
  "removedIds" | "missingIds" | "blocked" | "failures"
> & {
  removedIds: string[];
  missingIds: string[];
  blocked: IndexRemovalProblem[];
  failures: IndexRemovalProblem[];
};

export type BackendPermanentDeleteFailure = {
  clipId: number;
  code: string;
  retryable: boolean;
  message: string;
};

export type BackendPermanentDeleteResult = {
  requested: number;
  deletedIds: number[];
  missingIds: number[];
  pendingIds: number[];
  blocked: BackendPermanentDeleteFailure[];
  failures: BackendPermanentDeleteFailure[];
};

export type PermanentDeleteFailure = {
  clipId: string;
  code: string;
  retryable: boolean;
  message: string;
};

export type PermanentDeleteResult = {
  requested: number;
  deletedIds: string[];
  missingIds: string[];
  pendingIds: string[];
  blocked: PermanentDeleteFailure[];
  failures: PermanentDeleteFailure[];
};

export type BackendClipExport = {
  clipId: number;
  fileName: string;
  destinationPath: string;
  bytesCopied: number;
};

export type BackendClipExportFailure = {
  clipId: number;
  code: string;
  message: string;
};

export type BackendExportClipsResult = {
  requested: number;
  exported: number;
  failed: number;
  destinationDir: string;
  exportedIds: number[];
  missingIds: number[];
  missingFileIds: number[];
  exports: BackendClipExport[];
  failures: BackendClipExportFailure[];
};

export type ClipExport = Omit<BackendClipExport, "clipId"> & {
  clipId: string;
};

export type ClipExportFailure = Omit<BackendClipExportFailure, "clipId"> & {
  clipId: string;
};

export type ExportClipsResult = Omit<
  BackendExportClipsResult,
  "exportedIds" | "missingIds" | "missingFileIds" | "exports" | "failures"
> & {
  exportedIds: string[];
  missingIds: string[];
  missingFileIds: string[];
  exports: ClipExport[];
  failures: ClipExportFailure[];
};

export type ScanSummary = {
  rootPath: string;
  sourceDirCount: number;
  clipGroupCount: number;
  newClipCount: number;
  updatedClipCount: number;
  missingClipCount: number;
  coverMissingCount: number;
  metadataMatchCount?: number;
  metadataEnrichedClipCount?: number;
  metadataEventCount?: number;
  metadataWarningCount?: number;
  errors: string[];
  message: string | null;
};

export type FullDriveScanResult = {
  fixedDriveCount: number;
  visitedDirectoryCount: number;
  validatedSourceDirCount: number;
  scanRootCount: number;
  skippedDirectoryCount: number;
  discoveryWarnings: string[];
  scannedClipCount: number;
  scanSummary: ScanSummary;
};

export type ScanJobStatus =
  | "idle"
  | "running"
  | "cancelling"
  | "completed"
  | "partial"
  | "failed"
  | "cancelled";

export type ScanJobResult<T> = {
  jobId: string;
  status: ScanJobStatus;
  result: T | null;
  message: string;
};

export type ScanStatus = {
  jobId: string | null;
  phase: string | null;
  currentRoot: string | null;
  source: string | null;
  processed: number;
  total: number | null;
  message: string;
  terminal: boolean;
  status: ScanJobStatus;
};

export type CancelScanResult = {
  accepted: boolean;
  reason: "accepted" | "not-running" | "job-mismatch" | string;
  jobId: string;
  activeJobId: string | null;
  status: ScanJobStatus;
  message: string;
};

export type ScanProgress = {
  phase:
    | "discovering"
    | "scanning"
    | "metadata"
    | "finalizing"
    | "completed"
    | string;
  jobId: string;
  currentRoot: string | null;
  source: string | null;
  processed: number;
  total: number | null;
  terminal: boolean;
  status: ScanJobStatus;
  sourceDirCount: number;
  clipGroupCount: number;
  clipFileCount: number;
  message: string;
};

export type ClipMedia = {
  clipId: string;
  playable: boolean;
  mediaUrl: string | null;
  message: string | null;
};

export type BackendClipMedia = {
  clipId: number;
  playable: boolean;
  mediaPath: string | null;
  message: string | null;
};

export type ThumbnailEnqueueResult = {
  requested: number;
  queued: number;
  alreadyQueued: number;
  skipped: number;
};

export type ThumbnailGeneratorStatus = "unknown" | "available" | "unavailable";

export type ThumbnailQueueStatus = {
  generatorStatus: ThumbnailGeneratorStatus;
  pendingCount: number;
  runningCount: number;
  readyCount: number;
  failedCount: number;
  unavailableCount: number;
  evictedCount: number;
  cacheBytes: number;
  processingClipId: number | null;
  lastErrorCode: string | null;
};

export type BackendThumbnailProgress = {
  clipId: number;
  status: string;
  revision: string | null;
  errorCode: string | null;
};

export type ThumbnailProgress = Omit<BackendThumbnailProgress, "clipId"> & {
  clipId: string;
};
