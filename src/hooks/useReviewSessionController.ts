import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commandErrorMessage, listClipPage } from "../api/backend";
import {
  cloneReviewSession,
  createReviewSession,
  findResumableReviewSession,
  reviewSessionCounts,
  saveReviewSession,
  selectedClipIdsAcrossReviewSessions,
  type ReviewSessionCounts,
} from "../lib/reviewSessions";
import type {
  ClipSummary,
  ReviewItemDecision,
  ReviewSession,
  ReviewSessionFilters,
} from "../types";

const REVIEW_CANDIDATE_PAGE_SIZE = 200;
const RECENT_ADDITION_WINDOW_MS = 7 * 24 * 60 * 60 * 1_000;

export type ReviewSessionPhase = "setup" | "loading" | "reviewing" | "completed";
export type ReviewQueueMode = "all" | "pending";

type UndoEntry = {
  videoId: string;
  before: ReviewItemDecision;
  after: ReviewItemDecision;
  previousCurrentIndex: number;
  queueCursor: number;
  queueMode: ReviewQueueMode;
};

type QueueState = {
  mode: ReviewQueueMode;
  videoIds: string[];
  cursor: number;
};

export type ReviewSessionController = {
  phase: ReviewSessionPhase;
  session: ReviewSession | null;
  draftCandidates: ClipSummary[];
  candidateClips: ClipSummary[];
  currentClip: ClipSummary | null;
  counts: ReviewSessionCounts;
  resumableSession: ReviewSession | null;
  isPreparing: boolean;
  isDeciding: boolean;
  isUndoing: boolean;
  error: string | null;
  canUndo: boolean;
  prepare: (filters: ReviewSessionFilters) => Promise<void>;
  startNew: (filters: ReviewSessionFilters) => Promise<boolean>;
  resume: (session: ReviewSession) => Promise<boolean>;
  decide: (
    decision: Exclude<ReviewItemDecision, "unreviewed">,
    beforeAdvance?: () => void | Promise<void>,
  ) => Promise<boolean>;
  undo: () => boolean;
  continuePending: () => boolean;
  removeCurrent: () => boolean;
  saveProgress: () => boolean;
  clearError: () => void;
};

export function useReviewSessionController(): ReviewSessionController {
  const [phase, setPhase] = useState<ReviewSessionPhase>("setup");
  const [session, setSession] = useState<ReviewSession | null>(null);
  const [draftCandidates, setDraftCandidates] = useState<ClipSummary[]>([]);
  const [candidateClips, setCandidateClips] = useState<ClipSummary[]>([]);
  const [currentVideoId, setCurrentVideoId] = useState<string | null>(null);
  const [resumableSession, setResumableSession] = useState<ReviewSession | null>(null);
  const [isPreparing, setIsPreparing] = useState(false);
  const [isDeciding, setIsDeciding] = useState(false);
  const [isUndoing, setIsUndoing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [canUndo, setCanUndo] = useState(false);

  const mountedRef = useRef(false);
  const prepareTokenRef = useRef(0);
  const filtersRef = useRef<ReviewSessionFilters | null>(null);
  const baseCandidatesRef = useRef<ClipSummary[]>([]);
  const candidateByIdRef = useRef(new Map<string, ClipSummary>());
  const sessionRef = useRef<ReviewSession | null>(null);
  const queueRef = useRef<QueueState>({ mode: "all", videoIds: [], cursor: 0 });
  const currentVideoIdRef = useRef<string | null>(null);
  const undoStackRef = useRef<UndoEntry[]>([]);
  const decisionBusyRef = useRef(false);
  const loadingRef = useRef(false);

  sessionRef.current = session;
  currentVideoIdRef.current = currentVideoId;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      prepareTokenRef.current += 1;
      decisionBusyRef.current = false;
      loadingRef.current = false;
    };
  }, []);

  const replaceSession = useCallback((nextSession: ReviewSession | null) => {
    sessionRef.current = nextSession;
    if (nextSession) saveReviewSession(nextSession);
    if (mountedRef.current) setSession(nextSession);
  }, []);

  const replaceCandidates = useCallback((nextCandidates: ClipSummary[]) => {
    candidateByIdRef.current = new Map(nextCandidates.map((clip) => [clip.id, clip]));
    if (mountedRef.current) setCandidateClips(nextCandidates);
  }, []);

  const setActiveQueue = useCallback((
    nextSession: ReviewSession,
    mode: ReviewQueueMode,
  ) => {
    const videoIds = mode === "pending"
      ? nextSession.items
        .filter((item) => item.decision === "pending")
        .map((item) => item.videoId)
      : nextSession.items
        .filter((item) => item.decision === "unreviewed")
        .map((item) => item.videoId);
    const firstVideoId = videoIds[0] ?? null;
    queueRef.current = { mode, videoIds, cursor: 0 };
    currentVideoIdRef.current = firstVideoId;
    if (mountedRef.current) setCurrentVideoId(firstVideoId);
    return firstVideoId;
  }, []);

  const prepare = useCallback(async (filters: ReviewSessionFilters) => {
    const token = prepareTokenRef.current + 1;
    prepareTokenRef.current = token;
    loadingRef.current = true;
    filtersRef.current = filters;
    setIsPreparing(true);
    setError(null);
    setPhase("setup");
    try {
      const baseCandidates = await fetchReviewCandidates(filters.query);
      if (!mountedRef.current || prepareTokenRef.current !== token) return;
      baseCandidatesRef.current = baseCandidates;
      const nextDraftCandidates = applyCandidateScopeAndSort(baseCandidates, filters);
      setDraftCandidates(nextDraftCandidates);
      setResumableSession(findResumableReviewSession(filters));
    } catch (requestError) {
      if (!mountedRef.current || prepareTokenRef.current !== token) return;
      baseCandidatesRef.current = [];
      setDraftCandidates([]);
      setResumableSession(null);
      setError(`无法读取当前素材范围：${commandErrorMessage(requestError)}`);
    } finally {
      if (mountedRef.current && prepareTokenRef.current === token) {
        loadingRef.current = false;
        setIsPreparing(false);
      }
    }
  }, []);

  const hydrateSession = useCallback((storedSession: ReviewSession): ReviewSession | null => {
    const availableById = new Map(baseCandidatesRef.current.map((clip) => [clip.id, clip]));
    const items = storedSession.items.filter((item) => availableById.has(item.videoId));
    const nextSession: ReviewSession = {
      ...cloneReviewSession(storedSession),
      totalCount: items.length,
      currentIndex: normalizeCurrentIndex(items, storedSession.currentIndex),
      status: items.some((item) => item.decision === "unreviewed") ? "active" : "completed",
      updatedAt: new Date().toISOString(),
      items,
    };
    if (items.length === 0) {
      nextSession.currentIndex = 0;
      nextSession.status = "completed";
    }
    replaceCandidates(items
      .map((item) => availableById.get(item.videoId))
      .filter((clip): clip is ClipSummary => Boolean(clip)));
    replaceSession(nextSession);
    return nextSession;
  }, [replaceCandidates, replaceSession]);

  const startNew = useCallback(async (filters: ReviewSessionFilters): Promise<boolean> => {
    if (isPreparing || loadingRef.current || decisionBusyRef.current) return false;
    const filtersChanged = filtersRef.current !== filters;
    if (filtersChanged) {
      await prepare(filters);
      if (filtersRef.current !== filters) return false;
    }
    const candidates = applyCandidateScopeAndSort(baseCandidatesRef.current, filters);
    const nextSession = createReviewSession(filters, candidates.map((clip) => clip.id));
    replaceCandidates(candidates);
    replaceSession(nextSession);
    undoStackRef.current = [];
    setCanUndo(false);
    setResumableSession(null);
    const firstVideoId = setActiveQueue(nextSession, "all");
    if (firstVideoId) {
      setPhase("reviewing");
    } else {
      const completed = { ...nextSession, status: "completed" as const };
      replaceSession(completed);
      setPhase("completed");
    }
    return true;
  }, [isPreparing, prepare, replaceCandidates, replaceSession, setActiveQueue]);

  const resume = useCallback(async (storedSession: ReviewSession): Promise<boolean> => {
    if (isPreparing || loadingRef.current || decisionBusyRef.current) return false;
    const token = prepareTokenRef.current + 1;
    prepareTokenRef.current = token;
    loadingRef.current = true;
    setIsPreparing(true);
    setError(null);
    let restoredSession: ReviewSession | null;
    try {
      const baseCandidates = await fetchReviewCandidates(storedSession.filters.query);
      if (!mountedRef.current || prepareTokenRef.current !== token) return false;
      baseCandidatesRef.current = baseCandidates;
      filtersRef.current = storedSession.filters;
      restoredSession = hydrateSession(storedSession);
    } catch (requestError) {
      if (mountedRef.current && prepareTokenRef.current === token) {
        setError(`无法恢复上次挑片：${commandErrorMessage(requestError)}`);
      }
      return false;
    } finally {
      if (mountedRef.current && prepareTokenRef.current === token) {
        loadingRef.current = false;
        setIsPreparing(false);
      }
    }
    if (!restoredSession) return false;
    undoStackRef.current = [];
    setCanUndo(false);
    setResumableSession(null);
    const firstVideoId = setActiveQueue(restoredSession, "all");
    if (firstVideoId) {
      setPhase("reviewing");
    } else {
      const completed = { ...restoredSession, status: "completed" as const };
      replaceSession(completed);
      setPhase("completed");
    }
    return true;
  }, [hydrateSession, isPreparing, replaceSession, setActiveQueue]);

  const decide = useCallback(async (
    decision: Exclude<ReviewItemDecision, "unreviewed">,
    beforeAdvance?: () => void | Promise<void>,
  ): Promise<boolean> => {
    if (decisionBusyRef.current) return false;
    const currentSession = sessionRef.current;
    const videoId = currentVideoIdRef.current;
    if (!currentSession || !videoId) return false;
    const itemIndex = currentSession.items.findIndex((item) => item.videoId === videoId);
    if (itemIndex < 0) return false;

    const previousDecision = currentSession.items[itemIndex].decision;
    const queue = queueRef.current;
    decisionBusyRef.current = true;
    setIsDeciding(true);
    setError(null);
    const nextSession = updateSessionItem(currentSession, itemIndex, decision);
    replaceSession(nextSession);
    undoStackRef.current.push({
      videoId,
      before: previousDecision,
      after: decision,
      previousCurrentIndex: currentSession.currentIndex,
      queueCursor: queue.cursor,
      queueMode: queue.mode,
    });
    setCanUndo(true);

    try {
      await beforeAdvance?.();
      if (!mountedRef.current || sessionRef.current?.id !== nextSession.id) return false;
      const nextCursor = queue.cursor + 1;
      const nextVideoId = queue.videoIds[nextCursor] ?? null;
      queueRef.current = { ...queue, cursor: nextCursor };
      const nextIndex = nextVideoId
        ? nextSession.items.findIndex((item) => item.videoId === nextVideoId)
        : nextSession.totalCount;
      const advancedSession: ReviewSession = {
        ...nextSession,
        currentIndex: nextIndex >= 0 ? nextIndex : nextSession.totalCount,
        status: nextVideoId ? "active" : "completed",
        updatedAt: new Date().toISOString(),
      };
      replaceSession(advancedSession);
      currentVideoIdRef.current = nextVideoId;
      setCurrentVideoId(nextVideoId);
      setPhase(nextVideoId ? "reviewing" : "completed");
      return true;
    } catch (requestError) {
      if (mountedRef.current) {
        setError(`无法继续挑片：${commandErrorMessage(requestError)}`);
      }
      return false;
    } finally {
      decisionBusyRef.current = false;
      if (mountedRef.current) setIsDeciding(false);
    }
  }, [replaceSession]);

  const undo = useCallback((): boolean => {
    if (decisionBusyRef.current || isUndoing) return false;
    const entry = undoStackRef.current.pop();
    const currentSession = sessionRef.current;
    if (!entry || !currentSession) return false;
    const itemIndex = currentSession.items.findIndex((item) => item.videoId === entry.videoId);
    if (itemIndex < 0) return false;
    setIsUndoing(true);
    try {
      const restoredSession = updateSessionItem(currentSession, itemIndex, entry.before, {
        currentIndex: entry.previousCurrentIndex,
        status: "active",
      });
      replaceSession(restoredSession);
      queueRef.current = {
        mode: entry.queueMode,
        videoIds: queueIdsForSession(restoredSession, entry.queueMode, entry.videoId),
        cursor: 0,
      };
      const restoredCursor = queueRef.current.videoIds.indexOf(entry.videoId);
      queueRef.current.cursor = restoredCursor >= 0 ? restoredCursor : 0;
      currentVideoIdRef.current = entry.videoId;
      setCurrentVideoId(entry.videoId);
      setPhase("reviewing");
      setCanUndo(undoStackRef.current.length > 0);
      return true;
    } finally {
      if (mountedRef.current) setIsUndoing(false);
    }
  }, [isUndoing, replaceSession]);

  const continuePending = useCallback((): boolean => {
    if (decisionBusyRef.current) return false;
    const currentSession = sessionRef.current;
    if (!currentSession) return false;
    const firstVideoId = setActiveQueue(currentSession, "pending");
    if (!firstVideoId) return false;
    const nextIndex = currentSession.items.findIndex((item) => item.videoId === firstVideoId);
    replaceSession({
      ...currentSession,
      currentIndex: nextIndex >= 0 ? nextIndex : currentSession.totalCount,
      status: "active",
      updatedAt: new Date().toISOString(),
    });
    setPhase("reviewing");
    return true;
  }, [replaceSession, setActiveQueue]);

  const removeCurrent = useCallback((): boolean => {
    if (decisionBusyRef.current) return false;
    const currentSession = sessionRef.current;
    const videoId = currentVideoIdRef.current;
    if (!currentSession || !videoId) return false;
    const removedIndex = currentSession.items.findIndex((item) => item.videoId === videoId);
    if (removedIndex < 0) return false;
    const items = currentSession.items.filter((item) => item.videoId !== videoId);
    const nextCandidates = candidateClips.filter((clip) => clip.id !== videoId);
    const queue = queueRef.current;
    const queueIndex = queue.videoIds.indexOf(videoId);
    const nextQueueIds = queue.videoIds.filter((candidateId) => candidateId !== videoId);
    const nextCursor = queueIndex >= 0 ? queueIndex : queue.cursor;
    const nextVideoId = nextQueueIds[nextCursor] ?? null;
    const nextSession: ReviewSession = {
      ...currentSession,
      items,
      totalCount: items.length,
      currentIndex: nextVideoId
        ? Math.max(0, items.findIndex((item) => item.videoId === nextVideoId))
        : items.length,
      status: nextVideoId ? "active" : "completed",
      updatedAt: new Date().toISOString(),
    };
    replaceCandidates(nextCandidates);
    replaceSession(nextSession);
    queueRef.current = { mode: queue.mode, videoIds: nextQueueIds, cursor: nextCursor };
    currentVideoIdRef.current = nextVideoId;
    setCurrentVideoId(nextVideoId);
    setPhase(nextVideoId ? "reviewing" : "completed");
    return true;
  }, [candidateClips, replaceCandidates, replaceSession]);

  const saveProgress = useCallback((): boolean => {
    const currentSession = sessionRef.current;
    if (!currentSession) return false;
    replaceSession({
      ...currentSession,
      updatedAt: new Date().toISOString(),
    });
    return true;
  }, [replaceSession]);

  const clearError = useCallback(() => setError(null), []);

  const counts = useMemo(
    () => reviewSessionCounts(session ?? { items: [], totalCount: 0 }),
    [session],
  );
  const currentClip = currentVideoId
    ? candidateByIdRef.current.get(currentVideoId) ?? null
    : null;

  return {
    phase,
    session,
    draftCandidates,
    candidateClips,
    currentClip,
    counts,
    resumableSession,
    isPreparing,
    isDeciding,
    isUndoing,
    error,
    canUndo,
    prepare,
    startNew,
    resume,
    decide,
    undo,
    continuePending,
    removeCurrent,
    saveProgress,
    clearError,
  };
}

async function fetchReviewCandidates(query: ReviewSessionFilters["query"]): Promise<ClipSummary[]> {
  const initialPage = await listClipPage({
    ...query,
    offset: 0,
    limit: REVIEW_CANDIDATE_PAGE_SIZE,
  });
  const candidates = [...initialPage.items];
  let nextOffset = initialPage.nextOffset;
  let hasMore = initialPage.hasMore && nextOffset !== null;
  while (hasMore && nextOffset !== null) {
    const page = await listClipPage({
      ...query,
      offset: nextOffset,
      limit: REVIEW_CANDIDATE_PAGE_SIZE,
    });
    candidates.push(...page.items);
    nextOffset = page.nextOffset;
    hasMore = page.hasMore && nextOffset !== null;
  }
  return deduplicateClips(candidates);
}

function applyCandidateScopeAndSort(
  candidates: readonly ClipSummary[],
  filters: ReviewSessionFilters,
): ClipSummary[] {
  const selectedInEarlierSessions = filters.candidateScope === "not-selected"
    ? selectedClipIdsAcrossReviewSessions()
    : null;
  const recentCutoff = Date.now() - RECENT_ADDITION_WINDOW_MS;
  const scoped = candidates.filter((clip) => {
    if (selectedInEarlierSessions?.has(clip.id)) return false;
    if (filters.candidateScope === "recent") {
      return timestampForRecentAddition(clip) >= recentCutoff;
    }
    return true;
  });
  return [...scoped].sort(reviewSortComparator(filters.sort));
}

function reviewSortComparator(sort: ReviewSessionFilters["sort"]): (left: ClipSummary, right: ClipSummary) => number {
  if (sort === "latest") {
    return (left, right) => timestampForSort(right) - timestampForSort(left) || compareClipIds(left, right);
  }
  if (sort === "oldest") {
    return (left, right) => timestampForSort(left) - timestampForSort(right) || compareClipIds(left, right);
  }
  if (sort === "kills") {
    return (left, right) => (right.killCount ?? -1) - (left.killCount ?? -1)
      || timestampForSort(right) - timestampForSort(left)
      || compareClipIds(left, right);
  }
  if (sort === "score") {
    return (left, right) => (right.combatScore ?? -1) - (left.combatScore ?? -1)
      || timestampForSort(right) - timestampForSort(left)
      || compareClipIds(left, right);
  }
  return () => 0;
}

function timestampForRecentAddition(clip: ClipSummary): number {
  const created = Date.parse(clip.createdAt);
  return Number.isFinite(created) ? created : 0;
}

function timestampForSort(clip: ClipSummary): number {
  const timestamp = Date.parse(clip.createdAt || clip.modifiedAt);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function compareClipIds(left: ClipSummary, right: ClipSummary): number {
  return left.id.localeCompare(right.id, "zh-CN", { numeric: true });
}

function deduplicateClips(candidates: readonly ClipSummary[]): ClipSummary[] {
  const seen = new Set<string>();
  return candidates.filter((clip) => {
    if (seen.has(clip.id)) return false;
    seen.add(clip.id);
    return true;
  });
}

function updateSessionItem(
  session: ReviewSession,
  itemIndex: number,
  decision: ReviewItemDecision,
  override: Partial<Pick<ReviewSession, "currentIndex" | "status">> = {},
): ReviewSession {
  const items = session.items.map((item, index) => (
    index === itemIndex ? { ...item, decision } : item
  ));
  return {
    ...session,
    ...override,
    items,
    updatedAt: new Date().toISOString(),
  };
}

function queueIdsForSession(
  session: ReviewSession,
  mode: ReviewQueueMode,
  includeVideoId: string,
): string[] {
  return session.items
    .filter((item) => (
      item.videoId === includeVideoId
      || (mode === "pending" ? item.decision === "pending" : item.decision === "unreviewed")
    ))
    .map((item) => item.videoId);
}

function normalizeCurrentIndex(
  items: readonly { decision: ReviewItemDecision }[],
  currentIndex: number,
): number {
  if (items.length === 0) return 0;
  if (Number.isInteger(currentIndex) && currentIndex >= 0 && currentIndex < items.length) {
    return currentIndex;
  }
  const firstUnreviewed = items.findIndex((item) => item.decision === "unreviewed");
  return firstUnreviewed >= 0 ? firstUnreviewed : items.length;
}
