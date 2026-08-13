import { useCallback, useEffect, useMemo, useState } from "react";
import { ReviewComplete } from "../components/review/ReviewComplete";
import {
  ReviewSetup,
  type ReviewScopeEditor,
  type ReviewScopeOptions,
} from "../components/review/ReviewSetup";
import { ReviewSession as ReviewSessionPlayer } from "../components/review/ReviewSession";
import { useReviewSessionController } from "../hooks/useReviewSessionController";
import { useLocalDay } from "../hooks/useLocalDay";
import { deriveActiveFilters } from "../lib/activeFilters";
import { buildClipListQuery, CLIP_SEARCH_DEBOUNCE_MS } from "../lib/clipListQuery";
import { dateRangeForPreset } from "../lib/libraryFlow";
import { videoTypeLabel } from "../lib/videoTypes";
import type {
  ClipSort,
  ClipSummary,
  HighlightFilter,
  LibraryDatePreset,
  ReviewCandidateScope,
  ReviewSession,
  ReviewSessionFilters,
  ReviewSessionSort,
} from "../types";

type ReviewWorkspaceProps = {
  scopeOptions: ReviewScopeOptions;
  librarySort?: ClipSort;
  autoplay?: boolean;
  initialVolumePercent?: number;
  initialMuted?: boolean;
  onBack: () => void;
  onAudioPreferenceChange?: (preference: { volumePercent: number; muted: boolean }) => void;
  onOpenOriginal: (clipId: string) => void;
  onRemoveFromIndex: (clipId: string) => Promise<boolean>;
  onViewSelected: (
    session: ReviewSession,
    candidates: readonly ClipSummary[],
    autoOpenTagDialog: boolean,
  ) => void;
  onFavoriteSelected: (clipIds: string[]) => Promise<boolean>;
};

/**
 * Quick-pick owns its scope. The material library only provides choices, never
 * the currently selected values, so entering this workspace always begins from
 * the default all-material range.
 */
export function ReviewWorkspace({
  scopeOptions,
  librarySort = "modified-desc",
  autoplay = true,
  initialVolumePercent = 100,
  initialMuted = false,
  onBack,
  onAudioPreferenceChange,
  onOpenOriginal,
  onRemoveFromIndex,
  onViewSelected,
  onFavoriteSelected,
}: ReviewWorkspaceProps) {
  const controller = useReviewSessionController();
  const localDay = useLocalDay();
  const [scopeQuery, setScopeQuery] = useState("");
  const [debouncedScopeQuery, setDebouncedScopeQuery] = useState("");
  const [accountId, setAccountId] = useState("all");
  const [agentName, setAgentName] = useState("all");
  const [mapName, setMapName] = useState("all");
  const [gameMode, setGameMode] = useState("all");
  const [tagId, setTagId] = useState("all");
  const [datePreset, setDatePreset] = useState<LibraryDatePreset>("all");
  const [highlightFilter, setHighlightFilter] = useState<HighlightFilter>("all");
  const [sort, setSort] = useState<ReviewSessionSort>("latest");
  const [candidateScope, setCandidateScope] = useState<ReviewCandidateScope>("all");

  useEffect(() => {
    const timeoutId = window.setTimeout(
      () => setDebouncedScopeQuery(scopeQuery),
      CLIP_SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timeoutId);
  }, [scopeQuery]);

  const filterDateRange = useMemo(
    () => dateRangeForPreset(datePreset, localDay),
    [datePreset, localDay],
  );
  const query = useMemo<ReviewSessionFilters["query"]>(() => {
    const { offset: _offset, limit: _limit, reviewDecision: _reviewDecision, ...reviewQuery } = buildClipListQuery({
      query: debouncedScopeQuery,
      accountId,
      sourceDirId: "all",
      agentName,
      mapName,
      gameMode,
      tagId,
      highlightFilter,
      fileStatus: "all",
      libraryMode: "all",
      modifiedFrom: filterDateRange.modifiedFrom,
      modifiedTo: filterDateRange.modifiedTo,
      sortBy: sort === "library" ? librarySort : "modified-desc",
    });
    void _offset;
    void _limit;
    void _reviewDecision;
    return reviewQuery;
  }, [accountId, agentName, debouncedScopeQuery, filterDateRange, gameMode, highlightFilter, librarySort, mapName, sort, tagId]);

  const filterLabels = useMemo(() => {
    const labels = deriveActiveFilters({
      libraryMode: "all",
      query: debouncedScopeQuery,
      accountId,
      accountLabel: scopeOptions.accounts.find((account) => account.id === accountId)?.displayName ?? accountId,
      sourceDirId: "all",
      sourceDirLabel: "",
      agentName,
      mapName,
      gameMode,
      tagId,
      tagLabel: scopeOptions.tags.find((tag) => tag.id === tagId)?.label ?? tagId,
      fileStatus: "all",
    }).map((filter) => filter.label);

    if (datePreset !== "all") labels.push(`日期：${datePresetLabel(datePreset)}`);
    if (highlightFilter !== "all") labels.push(`视频类型：${videoTypeLabel(highlightFilter)}`);
    return labels;
  }, [accountId, agentName, datePreset, debouncedScopeQuery, gameMode, highlightFilter, mapName, scopeOptions.accounts, scopeOptions.tags, tagId]);

  const sessionFilters = useMemo<ReviewSessionFilters>(() => ({
    query,
    labels: filterLabels,
    sort,
    candidateScope,
  }), [candidateScope, filterLabels, query, sort]);

  useEffect(() => {
    void controller.prepare(sessionFilters);
  }, [controller.prepare, sessionFilters]);

  const resetScope = useCallback(() => {
    setScopeQuery("");
    setDebouncedScopeQuery("");
    setAccountId("all");
    setAgentName("all");
    setMapName("all");
    setGameMode("all");
    setTagId("all");
    setDatePreset("all");
    setHighlightFilter("all");
  }, []);

  const scopeEditor = useMemo<ReviewScopeEditor>(() => ({
    query: scopeQuery,
    accounts: scopeOptions.accounts,
    accountId,
    agentNames: scopeOptions.agentNames,
    agentName,
    mapNames: scopeOptions.mapNames,
    mapName,
    gameModes: scopeOptions.gameModes,
    gameMode,
    tags: scopeOptions.tags,
    tagId,
    datePreset,
    highlightFilter,
    videoTypes: scopeOptions.videoTypes,
    onQueryChange: setScopeQuery,
    onAccountChange: setAccountId,
    onAgentChange: setAgentName,
    onMapChange: setMapName,
    onGameModeChange: setGameMode,
    onTagChange: setTagId,
    onDatePresetChange: setDatePreset,
    onHighlightFilterChange: setHighlightFilter,
    onClearFilters: resetScope,
  }), [accountId, agentName, datePreset, gameMode, highlightFilter, mapName, resetScope, scopeOptions, scopeQuery, tagId]);

  const isScopeUpdating = scopeQuery !== debouncedScopeQuery;

  if (controller.phase === "setup" || controller.phase === "loading") {
    return (
      <ReviewSetup
        candidateCount={controller.draftCandidates.length}
        candidateScope={candidateScope}
        error={controller.error}
        filterLabels={filterLabels}
        isPreparing={controller.isPreparing || isScopeUpdating}
        resumableSession={controller.resumableSession}
        scopeEditor={scopeEditor}
        sort={sort}
        onCandidateScopeChange={setCandidateScope}
        onResume={() => {
          if (controller.resumableSession) void controller.resume(controller.resumableSession);
        }}
        onSortChange={setSort}
        onStart={() => {
          void controller.startNew(sessionFilters);
        }}
      />
    );
  }

  if (controller.phase === "completed" && controller.session) {
    return (
      <ReviewComplete
        candidates={controller.candidateClips}
        canUndo={controller.canUndo}
        counts={controller.counts}
        isUndoing={controller.isUndoing}
        session={controller.session}
        onContinuePending={() => controller.continuePending()}
        onFavoriteSelected={onFavoriteSelected}
        onOpenLibrary={onBack}
        onUndo={() => controller.undo()}
        onViewSelected={(autoOpenTagDialog) => onViewSelected(
          controller.session!,
          controller.candidateClips,
          autoOpenTagDialog,
        )}
      />
    );
  }

  return (
    <ReviewSessionPlayer
      autoplay={autoplay}
      controller={controller}
      initialMuted={initialMuted}
      initialVolumePercent={initialVolumePercent}
      onAudioPreferenceChange={onAudioPreferenceChange}
      onExit={onBack}
      onOpenOriginal={onOpenOriginal}
      onRemoveFromIndex={onRemoveFromIndex}
    />
  );
}

function datePresetLabel(preset: Exclude<LibraryDatePreset, "all">): string {
  if (preset === "today") return "今天";
  if (preset === "week") return "近 7 天";
  return "近 30 天";
}
