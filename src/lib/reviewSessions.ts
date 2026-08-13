import type {
  ClipListQuery,
  ReviewCandidateScope,
  ReviewItemDecision,
  ReviewSession,
  ReviewSessionFilters,
  ReviewSessionSort,
} from "../types";

export const REVIEW_SESSION_STORAGE_KEY = "valoframe.review-sessions.v1";

const REVIEW_SESSION_STORE_VERSION = 1;
const MAX_STORED_REVIEW_SESSIONS = 24;

type StoredReviewSessions = {
  version: number;
  sessions: ReviewSession[];
};

export type ReviewSessionCounts = {
  total: number;
  reviewed: number;
  selected: number;
  pending: number;
  skipped: number;
  remaining: number;
};

export function createReviewSession(
  filters: ReviewSessionFilters,
  videoIds: readonly string[],
): ReviewSession {
  const now = new Date().toISOString();
  return {
    id: createReviewSessionId(),
    createdAt: now,
    updatedAt: now,
    filters: cloneReviewSessionFilters(filters),
    totalCount: videoIds.length,
    currentIndex: videoIds.length > 0 ? 0 : videoIds.length,
    status: videoIds.length > 0 ? "active" : "completed",
    items: videoIds.map((videoId) => ({ videoId, decision: "unreviewed" })),
  };
}

export function reviewSessionCounts(session: Pick<ReviewSession, "items" | "totalCount">): ReviewSessionCounts {
  let selected = 0;
  let pending = 0;
  let skipped = 0;
  for (const item of session.items) {
    if (item.decision === "selected") selected += 1;
    else if (item.decision === "pending") pending += 1;
    else if (item.decision === "skipped") skipped += 1;
  }
  const reviewed = selected + pending + skipped;
  const total = Math.max(0, session.totalCount);
  return {
    total,
    reviewed,
    selected,
    pending,
    skipped,
    remaining: Math.max(0, total - reviewed),
  };
}

export function loadReviewSessions(): ReviewSession[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(REVIEW_SESSION_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!isStoredReviewSessions(parsed)) return [];
    return parsed.sessions
      .filter(isReviewSession)
      .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
  } catch {
    return [];
  }
}

export function saveReviewSession(nextSession: ReviewSession): void {
  if (typeof window === "undefined") return;
  const sessions = loadReviewSessions()
    .filter((session) => session.id !== nextSession.id)
    .concat(cloneReviewSession(nextSession));
  const pruned = pruneReviewSessions(sessions);
  try {
    window.localStorage.setItem(REVIEW_SESSION_STORAGE_KEY, JSON.stringify({
      version: REVIEW_SESSION_STORE_VERSION,
      sessions: pruned,
    } satisfies StoredReviewSessions));
  } catch {
    // A session remains usable in memory if local storage is temporarily unavailable.
  }
}

export function findResumableReviewSession(
  filters: Pick<ReviewSessionFilters, "query">,
): ReviewSession | null {
  const queryKey = reviewSessionQueryKey(filters.query);
  return preferredResumableSession(loadReviewSessions().filter((session) => (
    session.status === "active"
    && reviewSessionQueryKey(session.filters.query) === queryKey
  )));
}

/**
 * Finds progress independently of the setup page's current filters. This lets
 * quick-pick start each new round from its default scope without hiding a
 * previously saved, deliberately narrowed round.
 */
export function findLatestResumableReviewSession(): ReviewSession | null {
  return loadReviewSessions()
    .filter((session) => session.status === "active")
    .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))[0] ?? null;
}

function preferredResumableSession(sessions: readonly ReviewSession[]): ReviewSession | null {
  return [...sessions].sort((left, right) => (
    reviewSessionCounts(right).reviewed - reviewSessionCounts(left).reviewed
    || right.updatedAt.localeCompare(left.updatedAt)
  ))[0] ?? null;
}

export function selectedClipIdsAcrossReviewSessions(): Set<string> {
  const selectedIds = new Set<string>();
  for (const session of loadReviewSessions()) {
    for (const item of session.items) {
      if (item.decision === "selected") selectedIds.add(item.videoId);
    }
  }
  return selectedIds;
}

export function cloneReviewSession(session: ReviewSession): ReviewSession {
  return {
    ...session,
    filters: cloneReviewSessionFilters(session.filters),
    items: session.items.map((item) => ({ ...item })),
  };
}

export function cloneReviewSessionFilters(filters: ReviewSessionFilters): ReviewSessionFilters {
  return {
    ...filters,
    query: { ...filters.query },
    labels: [...filters.labels],
  };
}

export function reviewSessionQueryKey(query: Omit<ClipListQuery, "offset" | "limit" | "reviewDecision">): string {
  const normalized = Object.entries(query)
    .filter(([, value]) => value !== undefined)
    .sort(([left], [right]) => left.localeCompare(right));
  return JSON.stringify(normalized);
}

export function isReviewItemDecision(value: unknown): value is ReviewItemDecision {
  return value === "unreviewed" || value === "selected" || value === "pending" || value === "skipped";
}

function createReviewSessionId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `review-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function pruneReviewSessions(sessions: ReviewSession[]): ReviewSession[] {
  const sorted = [...sessions].sort((left, right) => (
    Number(right.status === "active") - Number(left.status === "active")
    || right.updatedAt.localeCompare(left.updatedAt)
  ));
  return sorted.slice(0, MAX_STORED_REVIEW_SESSIONS);
}

function isStoredReviewSessions(value: unknown): value is StoredReviewSessions {
  if (!isRecord(value) || value.version !== REVIEW_SESSION_STORE_VERSION || !Array.isArray(value.sessions)) {
    return false;
  }
  return true;
}

function isReviewSession(value: unknown): value is ReviewSession {
  if (!isRecord(value) || !isRecord(value.filters) || !Array.isArray(value.items)) return false;
  return typeof value.id === "string"
    && typeof value.createdAt === "string"
    && typeof value.updatedAt === "string"
    && typeof value.totalCount === "number"
    && typeof value.currentIndex === "number"
    && (value.status === "active" || value.status === "completed")
    && isReviewSessionSort(value.filters.sort)
    && isReviewCandidateScope(value.filters.candidateScope)
    && Array.isArray(value.filters.labels)
    && isRecord(value.filters.query)
    && value.items.every((item) => (
      isRecord(item)
      && typeof item.videoId === "string"
      && isReviewItemDecision(item.decision)
    ));
}

function isReviewSessionSort(value: unknown): value is ReviewSessionSort {
  return value === "library"
    || value === "latest"
    || value === "oldest"
    || value === "kills"
    || value === "score";
}

function isReviewCandidateScope(value: unknown): value is ReviewCandidateScope {
  return value === "all" || value === "not-selected" || value === "recent";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
