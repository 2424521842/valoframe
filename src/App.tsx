import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useTransition,
} from "react";
import { MotionConfig } from "motion/react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./cinematic.css";
import {
  commandErrorMessage,
  coverUrlForClipId,
  copyClipPath,
  exportClips,
  listSources,
  openClipLocation,
  openClipExternally,
  previewScanSourceRelocation,
  requestStartupSourceSync,
  relocateScanSource,
  registerScanSource,
  setScanSourceEnabled,
} from "./api/backend";
import { AmbientBackdrop } from "./components/AmbientBackdrop";
import { CinematicSidebar } from "./components/CinematicSidebar";
import { UiIcon } from "./components/UiIcon";
import { groupClipsByMatch } from "./lib/accountGrouping";
import { deriveActiveFilters, transitionLibraryMode } from "./lib/activeFilters";
import {
  buildClipListQuery,
  CLIP_SEARCH_DEBOUNCE_MS,
  clipListQueryKey,
} from "./lib/clipListQuery";
import {
  useClipPageController,
  type LoadClipPageOptions,
} from "./hooks/useClipPageController";
import { useClipDetailController } from "./hooks/useClipDetailController";
import { useClipMutationController } from "./hooks/useClipMutationController";
import { useLibraryFacetsController } from "./hooks/useLibraryFacetsController";
import { useLocalDay } from "./hooks/useLocalDay";
import { useScanController } from "./hooks/useScanController";
import { useTagController } from "./hooks/useTagController";
import { useThumbnailController } from "./hooks/useThumbnailController";
import { useAppUpdaterController } from "./hooks/useAppUpdaterController";
import { useAppPreferences } from "./hooks/useAppPreferences";
import type { StartupDestination } from "./lib/appPreferences";
import { normalizeCustomScanPath } from "./lib/customScan";
import { dateRangeForPreset } from "./lib/libraryFlow";
import {
  sourceScanFreshness,
  summarizeScanFreshness,
} from "./lib/scanFreshness";
import {
  mergeTagsWithFacets,
  removeTagFromClipCollection,
} from "./lib/tags";
import {
  videoTypeLabel,
  VIDEO_TYPE_FILTERS,
} from "./lib/videoTypes";
import {
  mergeScanTargets,
  scanPathKey,
} from "./lib/scanTargets";
import { useMediaQuery } from "./lib/useMediaQuery";
import type {
  AppScreen,
  AccountSummary,
  ClipDetail,
  ClipSummary,
  ClipSort,
  HighlightFilter,
  LibraryDatePreset,
  LibraryFacets,
  LibraryFacetValue,
  LibraryMode,
  LibraryViewMode,
  ScanTarget,
  SourceKind,
  SourceDir,
  RegisterScanSourceInput,
  RegisterScanSourceResult,
  ReviewSession,
  ThumbnailProgress,
} from "./types";

const LibraryWorkspace = lazy(() =>
  import("./screens/LibraryWorkspace").then((module) => ({
    default: module.LibraryWorkspace,
  })),
);
const PreviewWorkspace = lazy(() =>
  import("./screens/PreviewWorkspace").then((module) => ({
    default: module.PreviewWorkspace,
  })),
);
const ScanWorkspace = lazy(() =>
  import("./screens/ScanWorkspace").then((module) => ({
    default: module.ScanWorkspace,
  })),
);
const TagManagementWorkspace = lazy(() =>
  import("./screens/TagManagementWorkspace").then((module) => ({
    default: module.TagManagementWorkspace,
  })),
);
const SettingsWorkspace = lazy(() =>
  import("./screens/SettingsWorkspace").then((module) => ({
    default: module.SettingsWorkspace,
  })),
);
const ReviewWorkspace = lazy(() =>
  import("./screens/ReviewWorkspace").then((module) => ({
    default: module.ReviewWorkspace,
  })),
);

type ReviewResultSelection = {
  sessionId: string;
  clipIds: string[];
  selectionRequestId: string;
  openTagDialog: boolean;
};

function App() {
  const didLoadInitialData = useRef(false);
  const didRequestStartupSourceSync = useRef(false);
  const scanFreshnessDiagnosticsRef = useRef(new Set<string>());
  const preferencesController = useAppPreferences();
  const { preferences } = preferencesController;
  const [activeScreen, setActiveScreen] = useState<AppScreen>(
    () => startupNavigation(preferences.startupDestination).screen,
  );
  const [sourceDirs, setSourceDirs] = useState<SourceDir[]>([]);
  const sourceDirsRef = useRef<SourceDir[]>([]);
  sourceDirsRef.current = sourceDirs;
  const [libraryMode, setLibraryMode] = useState<LibraryMode>(
    () => startupNavigation(preferences.startupDestination).libraryMode,
  );
  const [selectedAccountId, setSelectedAccountId] = useState("all");
  const [selectedSourceDirId, setSelectedSourceDirId] = useState("all");
  const [selectedAgentName, setSelectedAgentName] = useState("all");
  const [selectedMapName, setSelectedMapName] = useState("all");
  const [selectedGameMode, setSelectedGameMode] = useState("all");
  const [selectedTagId, setSelectedTagId] = useState("all");
  const [selectedClipId, setSelectedClipId] = useState("");
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [fileStatus, setFileStatus] = useState("all");
  const [datePreset, setDatePreset] = useState<LibraryDatePreset>("all");
  const [highlightFilter, setHighlightFilter] = useState<HighlightFilter>("all");
  const [reviewResultSelection, setReviewResultSelection] = useState<ReviewResultSelection | null>(null);
  const [manualScanTargets, setManualScanTargets] = useState<ScanTarget[]>([]);
  const [excludedScanPaths, setExcludedScanPaths] = useState<Set<string>>(
    () => new Set(),
  );
  const [activityMessage, setActivityMessage] = useState("正在加载本地素材索引");
  const [activeExportCount, setActiveExportCount] = useState(0);
  const [activeRelocationCount, setActiveRelocationCount] = useState(0);
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);
  const [isFilterPending, startFilterTransition] = useTransition();
  const isSidebarOverlay = useMediaQuery("(max-width: 919px)");
  const menuTriggerRef = useRef<HTMLButtonElement | null>(null);
  const hasSidebarOverlay = isSidebarOverlay && isSidebarOpen;
  const sortBy = preferences.librarySort;
  const viewMode = preferences.libraryViewMode;
  const appUpdater = useAppUpdaterController({
    automaticCheck: preferences.automaticUpdateCheck,
  });
  const appUpdateBadge = appUpdater.phase === "downloaded"
    ? "待安装" as const
    : ["available", "downloading", "cancelling"].includes(appUpdater.phase)
      || (appUpdater.phase === "error" && appUpdater.update !== null)
      ? "更新" as const
      : undefined;
  const hasAvailableAppUpdate = appUpdateBadge !== undefined;
  const isExportActive = activeExportCount > 0;
  const isSourceRelocationActive = activeRelocationCount > 0;

  useLayoutEffect(() => {
    if (preferences.motionMode === "reduced") {
      document.documentElement.dataset.motion = "reduced";
    } else {
      delete document.documentElement.dataset.motion;
    }
    return () => {
      delete document.documentElement.dataset.motion;
    };
  }, [preferences.motionMode]);

  const effectiveDatePreset = libraryMode === "today" ? "today" : datePreset;
  const localDay = useLocalDay();
  const scanFreshnessSummary = useMemo(
    () => summarizeScanFreshness(sourceDirs, localDay),
    [localDay, sourceDirs],
  );
  const globalActivityMessage = scanFreshnessSummary.message
    ? `${activityMessage} · ${scanFreshnessSummary.message}`
    : activityMessage;
  const filterDateRange = useMemo(
    () => dateRangeForPreset(effectiveDatePreset, localDay),
    [effectiveDatePreset, localDay],
  );
  const productionListQuery = useMemo(
    () => buildClipListQuery({
      query: debouncedQuery,
      accountId: selectedAccountId,
      sourceDirId: selectedSourceDirId,
      agentName: selectedAgentName,
      mapName: selectedMapName,
      gameMode: selectedGameMode,
      tagId: selectedTagId,
      highlightFilter,
      fileStatus,
      libraryMode,
      modifiedFrom: filterDateRange.modifiedFrom,
      modifiedTo: filterDateRange.modifiedTo,
      sortBy,
    }),
    [
      debouncedQuery,
      fileStatus,
      filterDateRange,
      highlightFilter,
      libraryMode,
      selectedAccountId,
      selectedAgentName,
      selectedGameMode,
      selectedMapName,
      selectedSourceDirId,
      selectedTagId,
      sortBy,
    ],
  );
  const productionListQueryGenerationKey = clipListQueryKey(productionListQuery);
  const {
    items: clips,
    totalCount: totalClipCount,
    hasMore: hasMoreClips,
    generation: clipListGeneration,
    isLoading: isLoadingClips,
    isLoadingMore: isLoadingMoreClips,
    error: listError,
    loadMoreError,
    reload: reloadClipPages,
    loadMore: loadMoreClips,
    loadAll: loadAllClips,
    retryLoadMore,
    getQuery: getCurrentClipQuery,
    getItem: getClipSummary,
    mergeSummaries,
    removeSummaries,
    updateItems: updateClipSummaries,
  } = useClipPageController({
    query: productionListQuery,
    queryKey: productionListQueryGenerationKey,
    onActivityMessage: setActivityMessage,
  });
  const refreshCurrentClipQuery = useCallback(
    (options: LoadClipPageOptions = {
      preserveActivity: true,
      preserveItems: true,
    }) =>
      reloadClipPages(options),
    [reloadClipPages],
  );
  const {
    facets: libraryFacets,
    error: facetError,
    isLoading: isLoadingFacets,
    refresh: loadLibraryFacets,
  } = useLibraryFacetsController();
  const {
    state: detailState,
    retry: retryClipDetail,
    cancelPending: cancelPendingDetail,
    invalidate: invalidateDetailCache,
    getClip: getCachedClipDetail,
    syncClip: syncUpdatedDetail,
    patchThumbnail: patchDetailThumbnail,
    removeTag: removeTagFromDetails,
    removeClip: removeClipFromDetails,
  } = useClipDetailController({
    active: activeScreen === "preview",
    clipId: selectedClipId,
    sourceDirs,
  });
  const selectedClipSummary = selectedClipId
    ? getClipSummary(selectedClipId)
    : undefined;
  const readySelectedClip = (
    detailState.status === "ready" &&
    detailState.clip.id === selectedClipId
  )
    ? detailState.clip
    : undefined;
  const selectedClipDetail = readySelectedClip ?? (
    selectedClipId ? getCachedClipDetail(selectedClipId) : undefined
  );
  const previewDetailStatus = (
    selectedClipDetail
      ? "ready"
      : detailState.status === "ready"
        ? "loading"
        : detailState.status
  );
  const previewClip = selectedClipDetail
    ? selectedClipDetail
    : selectedClipSummary && (
      previewDetailStatus === "idle" ||
      previewDetailStatus === "loading"
    )
      ? clipDetailPlaceholder(selectedClipSummary)
      : null;
  const handleThumbnailProgress = useCallback((progress: ThumbnailProgress) => {
    const thumbnailStatus = progress.status.trim().toLowerCase();
    if (thumbnailStatus === "suppressed") {
      return;
    }
    const isReady = thumbnailStatus === "ready";
    const patch = {
      thumbnailStatus,
      thumbnailRevision: isReady ? progress.revision : null,
      thumbnailUrl: isReady
        ? coverUrlForClipId(progress.clipId, progress.revision)
        : null,
    };
    updateClipSummaries((currentClips) => {
      let changed = false;
      const nextClips = currentClips.map((clip) => {
        if (clip.id !== progress.clipId) return clip;
        if (
          clip.thumbnailStatus === patch.thumbnailStatus &&
          clip.thumbnailRevision === patch.thumbnailRevision &&
          clip.thumbnailUrl === patch.thumbnailUrl
        ) {
          return clip;
        }
        changed = true;
        return { ...clip, ...patch };
      });
      return changed ? nextClips : currentClips;
    });
    patchDetailThumbnail(progress.clipId, patch);
  }, [patchDetailThumbnail, updateClipSummaries]);
  useThumbnailController({
    generation: clipListGeneration,
    clips,
    onProgress: handleThumbnailProgress,
  });
  const handleTagDeleted = useCallback((tagId: string) => {
    updateClipSummaries((currentClips) =>
      removeTagFromClipCollection(currentClips, tagId),
    );
    removeTagFromDetails(tagId);
    setSelectedTagId((currentTagId) =>
      currentTagId === tagId ? "all" : currentTagId,
    );
  }, [removeTagFromDetails, updateClipSummaries]);
  const {
    tags,
    refresh: loadTagList,
    create: handleCreateTag,
    update: handleUpdateTag,
    remove: handleDeleteTag,
  } = useTagController({
    onActivityMessage: setActivityMessage,
    refreshFacets: loadLibraryFacets,
    onTagDeleted: handleTagDeleted,
  });

  const accounts = useMemo(
    () => accountSummariesFromFacets(libraryFacets, selectedAccountId),
    [libraryFacets, selectedAccountId],
  );
  const agentNames = useMemo(
    () => selectedFacetValues(libraryFacets?.agents, selectedAgentName),
    [libraryFacets, selectedAgentName],
  );
  const mapNames = useMemo(
    () => selectedFacetValues(libraryFacets?.maps, selectedMapName),
    [libraryFacets, selectedMapName],
  );
  const gameModes = useMemo(
    () => selectedFacetValues(libraryFacets?.gameModes, selectedGameMode),
    [libraryFacets, selectedGameMode],
  );
  const videoTypes = VIDEO_TYPE_FILTERS;
  const libraryTags = useMemo(
    () => mergeTagsWithFacets(tags, libraryFacets?.tags, selectedTagId),
    [libraryFacets, selectedTagId, tags],
  );
  const reviewScopeOptions = useMemo(() => ({
    accounts: accountSummariesFromFacets(libraryFacets, "all"),
    agentNames: selectedFacetValues(libraryFacets?.agents, "all"),
    mapNames: selectedFacetValues(libraryFacets?.maps, "all"),
    gameModes: selectedFacetValues(libraryFacets?.gameModes, "all"),
    tags: mergeTagsWithFacets(tags, libraryFacets?.tags, "all"),
    videoTypes,
  }), [libraryFacets, tags, videoTypes]);
  const clearSelectedClip = useCallback((clipIds: ReadonlySet<string>) => {
    setSelectedClipId((current) => clipIds.has(current) ? "" : current);
  }, []);
  const {
    isPermanentDeleteActive,
    toggleFavorite: handleToggleFavorite,
    setFavoriteForClips: handleSetFavoriteForClips,
    setTagForClips: handleSetTagForClips,
    setTrashedForClips: handleSetTrashedForClips,
    deleteClipsPermanently: handleDeleteClipsPermanently,
    removeClipsFromIndex: handleRemoveClipsFromIndex,
    updateNote: handleUpdateNote,
    toggleTag: handleToggleClipTag,
  } = useClipMutationController({
    sourceDirs,
    tags: libraryTags,
    getSummary: getClipSummary,
    getDetail: getCachedClipDetail,
    getQuery: getCurrentClipQuery,
    updateSummaries: updateClipSummaries,
    removeSummaries,
    syncDetail: syncUpdatedDetail,
    removeDetail: removeClipFromDetails,
    refreshClips: refreshCurrentClipQuery,
    refreshFacets: loadLibraryFacets,
    clearSelectedClip,
    onActivityMessage: setActivityMessage,
  });
  const tagUsageCounts = useMemo(
    () => new Map(
      (libraryFacets?.tags ?? []).map((tag) => [String(tag.id), tag.activeCount]),
    ),
    [libraryFacets],
  );
  const sourceDirsWithFacetCounts = useMemo(() => {
    if (!libraryFacets) return sourceDirs;
    const countById = new Map(
      libraryFacets.sourceDirs.map((source) => [String(source.sourceDirId), source.activeCount]),
    );
    return sourceDirs.map((source) => ({
      ...source,
      clipCount: countById.get(source.id) ?? 0,
    }));
  }, [libraryFacets, sourceDirs]);
  const fullLibraryCount = libraryFacets?.activeCount ?? totalClipCount;
  const scanTargets = useMemo(
    () => mergeScanTargets(sourceDirs, manualScanTargets, excludedScanPaths),
    [excludedScanPaths, manualScanTargets, sourceDirs],
  );

  const loadSourceList = useCallback(async (
    { preserveActivity = false }: { preserveActivity?: boolean } = {},
  ): Promise<boolean> => {
    try {
      const nextSourceDirs = await listSources();
      sourceDirsRef.current = nextSourceDirs;
      setSourceDirs(nextSourceDirs);
      return true;
    } catch (error) {
      if (!preserveActivity) {
        setActivityMessage(`来源加载失败：${commandErrorMessage(error)}`);
      }
      return false;
    }
  }, []);

  useEffect(() => {
    if (didLoadInitialData.current) {
      return;
    }

    didLoadInitialData.current = true;
    void loadSourceList();
  }, [loadSourceList]);

  useEffect(() => {
    for (const source of sourceDirs) {
      const freshness = sourceScanFreshness(source.lastScanAt, localDay);
      const diagnosticKey = `${source.id}:${source.lastScanAt ?? "null"}:${freshness.issue}`;
      if (!freshness.issue || scanFreshnessDiagnosticsRef.current.has(diagnosticKey)) {
        continue;
      }
      scanFreshnessDiagnosticsRef.current.add(diagnosticKey);
      if (freshness.issue === "invalid") {
        console.warn(`来源 ${source.id} 的 lastScanAt 无效，已按首次扫描处理`);
      } else if (freshness.issue === "future") {
        console.warn(`来源 ${source.id} 的 lastScanAt 位于未来，已按今天扫描处理`);
      }
    }
  }, [localDay, sourceDirs]);

  useEffect(() => {
    const timer = window.setTimeout(
      () => setDebouncedQuery(query),
      CLIP_SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    if (
      selectedSourceDirId !== "all" &&
      !sourceDirs.some((sourceDir) => sourceDir.id === selectedSourceDirId)
    ) {
      setSelectedSourceDirId("all");
    }
  }, [selectedSourceDirId, sourceDirs]);

  const visibleClips = useMemo(() => {
    if (!reviewResultSelection) return clips;
    const selectedIds = new Set(reviewResultSelection.clipIds);
    return clips.filter((clip) => selectedIds.has(clip.id));
  }, [clips, reviewResultSelection]);
  const visibleTotalClipCount = reviewResultSelection
    ? reviewResultSelection.clipIds.length
    : totalClipCount;
  const libraryScrollResetKey = `${productionListQueryGenerationKey}|review:${reviewResultSelection?.selectionRequestId ?? "all"}`;

  const libraryFilterLabels = useMemo(() => {
    const labels = deriveActiveFilters({
      libraryMode,
      query,
      accountId: selectedAccountId,
      accountLabel: accounts.find((account) => account.id === selectedAccountId)?.displayName ?? "",
      sourceDirId: selectedSourceDirId,
      sourceDirLabel: sourceDirs.find((source) => source.id === selectedSourceDirId)?.name ?? "",
      agentName: selectedAgentName,
      mapName: selectedMapName,
      gameMode: selectedGameMode,
      tagId: selectedTagId,
      tagLabel: libraryTags.find((tag) => tag.id === selectedTagId)?.label ?? "",
      fileStatus,
    }).map((filter) => filter.label);

    if (effectiveDatePreset !== "all" && libraryMode !== "today") {
      labels.push(`日期：${datePresetLabel(effectiveDatePreset)}`);
    }
    if (highlightFilter !== "all") {
      labels.push(`视频类型：${videoTypeLabel(highlightFilter)}`);
    }
    return labels;
  }, [accounts, effectiveDatePreset, fileStatus, highlightFilter, libraryMode, libraryTags, query, selectedAccountId, selectedAgentName, selectedGameMode, selectedMapName, selectedSourceDirId, selectedTagId, sourceDirs]);

  const activeFilterLabels = useMemo(() => (
    reviewResultSelection
      ? [...libraryFilterLabels, `本轮入选：${reviewResultSelection.clipIds.length} 条`]
      : libraryFilterLabels
  ), [libraryFilterLabels, reviewResultSelection]);

  const matchGroups = useMemo(
    () => groupClipsByMatch(visibleClips),
    [visibleClips],
  );

  const clearReviewResultSelection = useCallback(() => {
    setReviewResultSelection(null);
  }, []);

  const handleModeChange = useCallback((mode: LibraryMode) => {
    const next = transitionLibraryMode(fileStatus, mode);
    startFilterTransition(() => {
      clearReviewResultSelection();
      setIsSidebarOpen(false);
      setActiveScreen("library");
      setLibraryMode(next.libraryMode);
      setFileStatus(next.fileStatus);
    });
  }, [clearReviewResultSelection, fileStatus]);

  const handleAccountChange = useCallback((accountId: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setSelectedAccountId(accountId);
    });
  }, [clearReviewResultSelection]);

  const handleAgentChange = useCallback((agentName: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setSelectedAgentName(agentName);
    });
  }, [clearReviewResultSelection]);

  const handleMapChange = useCallback((mapName: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setSelectedMapName(mapName);
    });
  }, [clearReviewResultSelection]);

  const handleGameModeChange = useCallback((gameMode: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setSelectedGameMode(gameMode);
    });
  }, [clearReviewResultSelection]);

  const handleTagChange = useCallback((tagId: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setSelectedTagId(tagId);
    });
  }, [clearReviewResultSelection]);

  const handleSortChange = useCallback((nextSortBy: ClipSort) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      preferencesController.updatePreferences({ librarySort: nextSortBy });
    });
  }, [clearReviewResultSelection, preferencesController]);

  const handleViewModeChange = useCallback((nextViewMode: LibraryViewMode) => {
    preferencesController.updatePreferences({ libraryViewMode: nextViewMode });
  }, [preferencesController]);

  const handleAudioPreferenceChange = useCallback((audio: {
    volumePercent: number;
    muted: boolean;
  }) => {
    preferencesController.updatePreferences({
      previewVolumePercent: audio.volumePercent,
      previewMuted: audio.muted,
    });
  }, [preferencesController]);

  const handleClearAllFilters = () => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setLibraryMode("all");
      setQuery("");
      setSelectedAccountId("all");
      setSelectedSourceDirId("all");
      setSelectedAgentName("all");
      setSelectedMapName("all");
      setSelectedGameMode("all");
      setSelectedTagId("all");
      setFileStatus("all");
      setDatePreset("all");
      setHighlightFilter("all");
    });
  };

  const handleSelectClip = useCallback((clipId: string, _trigger: HTMLElement) => {
    cancelPendingDetail();
    setSelectedClipId(clipId);
    setIsSidebarOpen(false);
    setActiveScreen("preview");
  }, [cancelPendingDetail]);

  const handleScreenChange = useCallback((screen: Exclude<AppScreen, "preview">) => {
    cancelPendingDetail();
    setIsSidebarOpen(false);
    if (screen === "review") clearReviewResultSelection();
    setActiveScreen(screen);
  }, [cancelPendingDetail, clearReviewResultSelection]);

  const handleSelectPreviewClip = useCallback((clipId: string) => {
    if (clipId === selectedClipId) return;
    setSelectedClipId(clipId);
  }, [selectedClipId]);

  const handleRetryDetail = useCallback(() => {
    void retryClipDetail();
  }, [retryClipDetail]);

  const handleCloseSidebar = useCallback(() => {
    setIsSidebarOpen(false);
    window.requestAnimationFrame(() => {
      menuTriggerRef.current?.focus();
    });
  }, []);

  const handleOpenSidebar = useCallback((trigger: HTMLButtonElement) => {
    menuTriggerRef.current = trigger;
    setIsSidebarOpen(true);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!hasSidebarOverlay) return;

      if (event.key === "Escape") {
        handleCloseSidebar();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleCloseSidebar, hasSidebarOverlay]);

  const handleViewTag = (tagId: string) => {
    startFilterTransition(() => {
      clearReviewResultSelection();
      setActiveScreen("library");
      setLibraryMode("all");
      setQuery("");
      setSelectedAccountId("all");
      setSelectedSourceDirId("all");
      setSelectedAgentName("all");
      setSelectedMapName("all");
      setSelectedGameMode("all");
      setSelectedTagId(tagId);
      setFileStatus("all");
      setDatePreset("all");
      setHighlightFilter("all");
    });
  };

  const handleViewReviewSelected = useCallback((
    session: ReviewSession,
    candidates: readonly ClipSummary[],
    autoOpenTagDialog: boolean,
  ) => {
    const selectedIds = session.items
      .filter((item) => item.decision === "selected")
      .map((item) => item.videoId);
    if (selectedIds.length === 0) return;
    mergeSummaries(candidates);
    setReviewResultSelection({
      sessionId: session.id,
      clipIds: selectedIds,
      selectionRequestId: `${session.id}:${Date.now()}`,
      openTagDialog: autoOpenTagDialog,
    });
    setActiveScreen("library");
  }, [mergeSummaries]);

  const handleRemoveReviewClipFromIndex = useCallback(async (clipId: string): Promise<boolean> => {
    const result = await handleRemoveClipsFromIndex([clipId]);
    return Boolean(result && (result.removedIds.includes(clipId) || result.missingIds.includes(clipId)));
  }, [handleRemoveClipsFromIndex]);

  const refreshAfterScan = async () => {
    invalidateDetailCache();
    const outcomes = await Promise.allSettled([
      loadSourceList({ preserveActivity: true }),
      refreshCurrentClipQuery({ preserveActivity: true }),
      loadTagList({ preserveActivity: true }),
      loadLibraryFacets(),
    ]);
    return outcomes.every((outcome) => (
      outcome.status === "fulfilled" && outcome.value
    ));
  };

  const {
    activeJobId: activeScanJobId,
    status: scanStatus,
    progress: scanProgress,
    summary: scanSummary,
    errorMessage: scanError,
    isScanning,
    startScan: handleStartScan,
    syncSource: handleSyncSource,
    syncEnabledSources: handleSyncEnabledSources,
    discoverAll: handleDiscoverAll,
    cancelScan: handleCancelScan,
    settleExternalTerminal: settleExternalScanTerminal,
    clearOutcome: clearScanOutcome,
    clearSummary: clearScanSummary,
    reportError: setScanError,
  } = useScanController({
    sourcePaths: scanTargets.map((target) => target.path),
    refresh: refreshAfterScan,
    notify: ({ message }) => setActivityMessage(message),
  });

  useEffect(() => {
    if (didRequestStartupSourceSync.current) return;
    didRequestStartupSourceSync.current = true;
    if (!preferences.scanOnStartup || !isTauri()) return;

    void requestStartupSourceSync().catch((error) => {
      setScanError(`启动自动扫描失败：${commandErrorMessage(error)}`);
      setActivityMessage("启动自动扫描未能开始，请在扫描目录中手动重试");
    });
  }, [preferences.scanOnStartup, setScanError]);

  const handleChooseSourceDirectory = async (sourceKind: SourceKind): Promise<string | null> => {
    if (isScanning) {
      return null;
    }

    if (!isTauri()) {
      setScanError("目录选择仅在桌面应用中可用，请使用 npm run tauri dev 启动应用。");
      setActivityMessage("无法在浏览器预览中选择本机目录");
      return null;
    }

    try {
      const path = normalizeCustomScanPath(await open({
        directory: true,
        multiple: false,
        title: sourceKind === "aclos"
          ? "选择 ACLOS 根目录或 wonderfulVideos 目录"
          : "选择本地 MP4 录制输出目录",
      }));
      if (!path) {
        setActivityMessage("已取消选择来源目录");
        return null;
      }
      clearScanOutcome();
      setActivityMessage(`已选择来源目录：${path}`);
      return path;
    } catch (error) {
      setScanError(`无法打开目录选择窗口：${commandErrorMessage(error)}`);
      setActivityMessage("添加目录失败");
      return null;
    }
  };

  const handleChooseRelocationDirectory = async (
    source: SourceDir,
  ): Promise<string | null> => {
    if (isScanning) return null;
    if (!isTauri()) {
      setActivityMessage("重新定位仅在桌面应用中可用");
      throw new Error("目录选择仅在桌面应用中可用，请使用 npm run tauri dev 启动应用。");
    }

    try {
      const path = normalizeCustomScanPath(await open({
        directory: true,
        multiple: false,
        title: `为 ${source.displayName} 选择新的来源根目录`,
      }));
      if (!path) {
        setActivityMessage("已取消重新定位目录选择");
        return null;
      }
      setActivityMessage(`正在预览来源新位置：${path}`);
      return path;
    } catch (error) {
      const message = commandErrorMessage(error);
      setActivityMessage(`重新定位目录选择失败：${message}`);
      throw new Error(message);
    }
  };

  const handleRegisterSource = async (
    input: RegisterScanSourceInput,
  ): Promise<RegisterScanSourceResult> => {
    try {
      const result = await registerScanSource(input);
      if (result.requiresOverlapConfirmation) {
        setActivityMessage("来源目录与已有来源重叠，等待确认");
        return result;
      }
      await loadSourceList();
      setActivityMessage(
        result.duplicateCount > 0 ? "已复用现有视频来源" : "视频来源已注册，开始首次同步",
      );
      // Registration should not keep the source wizard open for the entire first scan. Run each
      // adapter sequentially in the background so multi-source ACLOS registrations still avoid
      // scan-coordinator conflicts while the newly registered source becomes visible immediately.
      void (async () => {
        for (const source of result.sources) {
          await handleSyncSource(source.id);
        }
      })();
      return result;
    } catch (error) {
      const message = commandErrorMessage(error);
      setScanError(`添加视频来源失败：${message}`);
      setActivityMessage("添加视频来源失败");
      throw new Error(message);
    }
  };

  const handleRelocateSource = useCallback(async (
    sourceId: string,
    newRootPath: string,
  ) => {
    setActiveRelocationCount((count) => count + 1);
    let result: Awaited<ReturnType<typeof relocateScanSource>>;
    try {
      result = await relocateScanSource(sourceId, newRootPath);
    } finally {
      setActiveRelocationCount((count) => Math.max(0, count - 1));
    }
    if (result.syncJobId) {
      if (result.syncStatus) {
        await settleExternalScanTerminal({
          jobId: result.syncJobId,
          status: result.syncStatus,
          message: result.syncMessage ?? "重新定位后的来源同步已结束",
        });
      }
      return result;
    }

    invalidateDetailCache();
    const refreshOutcomes = await Promise.allSettled([
      loadSourceList({ preserveActivity: true }),
      refreshCurrentClipQuery({ preserveActivity: true }),
      loadLibraryFacets(),
    ]);
    const refreshFailed = refreshOutcomes.some((outcome) => (
      outcome.status === "rejected" || !outcome.value
    ));
    setActivityMessage(refreshFailed
      ? "重新定位成功、同步尚未启动；索引视图刷新失败，请手动刷新"
      : "重新定位成功、同步尚未启动");
    return result;
  }, [
    invalidateDetailCache,
    loadLibraryFacets,
    loadSourceList,
    refreshCurrentClipQuery,
    settleExternalScanTerminal,
  ]);

  const handleSetSourceEnabled = async (source: SourceDir, enabled: boolean) => {
    try {
      await setScanSourceEnabled(source.id, enabled);
      await loadSourceList();
      setActivityMessage(`${source.displayName} 已${enabled ? "加入" : "移出"}自动同步`);
    } catch (error) {
      setScanError(`更新来源设置失败：${commandErrorMessage(error)}`);
    }
  };

  const handleRemoveDirectory = (target: ScanTarget) => {
    const key = scanPathKey(target.path);
    setManualScanTargets((current) =>
      current.filter((candidate) => scanPathKey(candidate.path) !== key),
    );
    setExcludedScanPaths((current) => new Set(current).add(key));
    clearScanSummary();
    setActivityMessage(`已从扫描队列移除：${target.name}`);
  };

  const handleRefreshLibrary = () => {
    clearReviewResultSelection();
    invalidateDetailCache();
    void Promise.all([
      refreshCurrentClipQuery({ preserveActivity: false }),
      loadSourceList(),
      loadTagList(),
      loadLibraryFacets(),
    ]);
  };

  const handleCopyPath = useCallback(async (clipId: string) => {
    try {
      const filePath = await copyClipPath(clipId);
      await navigator.clipboard?.writeText(filePath);
      setActivityMessage("已复制素材路径");
    } catch (error) {
      setActivityMessage(`复制路径失败：${commandErrorMessage(error)}`);
    }
  }, []);

  const handleOpenOriginal = useCallback(async (clipId: string) => {
    try {
      await openClipLocation(clipId);
      setActivityMessage("已在文件资源管理器中定位素材");
    } catch (error) {
      setActivityMessage(`打开原位置失败：${commandErrorMessage(error)}`);
    }
  }, []);

  const handleOpenExternal = useCallback(async (clipId: string) => {
    try {
      await openClipExternally(clipId);
      setActivityMessage("已交给系统默认播放器");
    } catch (error) {
      setActivityMessage(`系统播放器打开失败：${commandErrorMessage(error)}`);
    }
  }, []);

  const handleExportClips = useCallback(async (clipIds: string[]): Promise<boolean> => {
    if (clipIds.length === 0) return false;
    if (!isTauri()) {
      setActivityMessage("导出视频仅在桌面应用中可用");
      return false;
    }

    let exportStarted = false;
    try {
      const destinationDir = normalizeCustomScanPath(await open({
        directory: true,
        multiple: false,
        title: "选择导出文件夹",
      }));
      if (!destinationDir) {
        setActivityMessage("已取消导出");
        return false;
      }

      exportStarted = true;
      setActiveExportCount((count) => count + 1);
      setActivityMessage(`正在导出 ${clipIds.length} 条素材…`);
      const result = await exportClips(clipIds, destinationDir);
      if (result.failed > 0) {
        const firstFailure = result.failures[0]?.message;
        setActivityMessage(
          `导出部分完成：成功 ${result.exported}/${result.requested} 条，失败 ${result.failed} 条${firstFailure ? `：${firstFailure}` : ""}`,
        );
        return false;
      }

      setActivityMessage(`已导出 ${result.exported} 条素材到 ${result.destinationDir}`);
      return true;
    } catch (error) {
      setActivityMessage(`导出失败：${commandErrorMessage(error)}`);
      return false;
    } finally {
      if (exportStarted) {
        setActiveExportCount((count) => Math.max(0, count - 1));
      }
    }
  }, []);

  return (
    <MotionConfig reducedMotion={preferences.motionMode === "reduced" ? "always" : "user"}>
      <main className={`app-root app-root--${activeScreen}`}>
        <AmbientBackdrop />
        {activeScreen !== "preview" ? (
          <AppTopBar
            activeScreen={activeScreen}
            appVersion={appUpdater.runtimeInfo?.currentVersion ?? null}
            hasAvailableUpdate={hasAvailableAppUpdate}
            totalCount={fullLibraryCount}
            isScanning={isScanning}
            onOpenSidebar={handleOpenSidebar}
          />
        ) : null}

        <div className={`app-shell app-shell--${activeScreen}`}>
        {activeScreen !== "preview" ? (
          <CinematicSidebar
            activeMode={libraryMode}
            activeScreen={activeScreen}
            favoriteCount={libraryFacets?.activeFavoriteCount ?? 0}
            isOpen={!isSidebarOverlay || isSidebarOpen}
            isOverlay={isSidebarOverlay}
            recentCount={libraryFacets?.recentCount ?? 0}
            tagCount={libraryTags.length}
            totalCount={fullLibraryCount}
            trashCount={libraryFacets?.trashedCount ?? 0}
            updateBadge={appUpdateBadge}
            updateVersion={appUpdater.update?.version}
            onClose={handleCloseSidebar}
            onModeChange={handleModeChange}
            onOpenScan={() => handleScreenChange("scan")}
            onOpenReview={() => handleScreenChange("review")}
            onOpenTagManager={() => handleScreenChange("tags")}
            onOpenSettings={() => handleScreenChange("settings")}
          />
        ) : null}

        <Suspense fallback={<WorkspaceLoading />}>
          <LibraryWorkspace
            accountId={selectedAccountId}
            accounts={accounts}
            sourceDirs={sourceDirsWithFacetCounts}
            activeFilterLabels={activeFilterLabels}
            activityMessage={globalActivityMessage}
            agentName={selectedAgentName}
            agentNames={agentNames}
            datePreset={effectiveDatePreset}
            errorMessage={listError}
            facetError={facetError}
            gameMode={selectedGameMode}
            gameModes={gameModes}
            highlightFilter={highlightFilter}
            videoTypes={videoTypes}
            isFacetLoading={isLoadingFacets}
            isLoading={isLoadingClips}
            isLoadingMore={isLoadingMoreClips}
            isPending={isFilterPending || query !== debouncedQuery}
            isScanning={isScanning}
            isActive={activeScreen === "library"}
            initialSelectedClipIds={reviewResultSelection?.clipIds}
            openTagDialogForInitialSelection={reviewResultSelection?.openTagDialog ?? false}
            mapName={selectedMapName}
            mapNames={mapNames}
            matchGroups={matchGroups}
            hasMore={reviewResultSelection ? false : hasMoreClips}
            listGeneration={clipListGeneration}
            loadMoreError={loadMoreError}
            libraryMode={libraryMode}
            query={query}
            scrollResetKey={libraryScrollResetKey}
            selectedClipId={selectedClipId}
            sortBy={sortBy}
            tagId={selectedTagId}
            tags={libraryTags}
            totalClipCount={visibleTotalClipCount}
            viewMode={viewMode}
            selectionRequestId={reviewResultSelection?.selectionRequestId ?? null}
            visibleClipCount={visibleClips.length}
            onAccountChange={handleAccountChange}
            onAgentChange={handleAgentChange}
            onClearFilters={handleClearAllFilters}
            onCopyPath={handleCopyPath}
            onCreateTag={handleCreateTag}
            onExportClips={handleExportClips}
            onDatePresetChange={(value) => startFilterTransition(() => {
              clearReviewResultSelection();
              setLibraryMode("all");
              setDatePreset(value);
            })}
            onGameModeChange={handleGameModeChange}
            onHighlightFilterChange={(value) => startFilterTransition(() => {
              clearReviewResultSelection();
              setHighlightFilter(value);
            })}
            onMapChange={handleMapChange}
            onOpenOriginal={handleOpenOriginal}
            onOpenScan={() => handleScreenChange("scan")}
            onLoadAll={loadAllClips}
            onLoadMore={() => void loadMoreClips()}
            onQueryChange={(value) => {
              clearReviewResultSelection();
              setQuery(value);
            }}
            onRefresh={handleRefreshLibrary}
            onRetryLoad={() => void refreshCurrentClipQuery({ preserveActivity: false })}
            onRetryLoadMore={() => void retryLoadMore()}
            onSelectClip={handleSelectClip}
            onSetFavoriteForClips={handleSetFavoriteForClips}
            onSetTagForClips={handleSetTagForClips}
            onSetTrashedForClips={handleSetTrashedForClips}
            onDeleteClipsPermanently={handleDeleteClipsPermanently}
            onRemoveClipsFromIndex={handleRemoveClipsFromIndex}
            onSortChange={handleSortChange}
            onTagChange={handleTagChange}
            onToggleFavorite={handleToggleFavorite}
            onViewModeChange={handleViewModeChange}
          />
          {activeScreen === "library" ? null : activeScreen === "review" ? (
            <ReviewWorkspace
              autoplay={preferences.reviewAutoplay}
              initialMuted={preferences.previewMuted}
              initialVolumePercent={preferences.previewVolumePercent}
              librarySort={sortBy}
              scopeOptions={reviewScopeOptions}
              onAudioPreferenceChange={handleAudioPreferenceChange}
              onBack={() => handleScreenChange("library")}
              onFavoriteSelected={(clipIds) => handleSetFavoriteForClips(clipIds, true)}
              onOpenOriginal={handleOpenOriginal}
              onRemoveFromIndex={handleRemoveReviewClipFromIndex}
              onViewSelected={handleViewReviewSelected}
            />
          ) : activeScreen === "scan" ? (
            <ScanWorkspace
              activeJobId={activeScanJobId}
              accounts={accounts}
              activityMessage={globalActivityMessage}
              localDay={localDay}
              errorMessage={scanError ?? listError}
              facets={libraryFacets}
              isLoading={isLoadingClips}
              isScanning={isScanning}
              progress={scanProgress}
              scanStatus={scanStatus}
              scanTargets={scanTargets}
              sourceDirs={sourceDirsWithFacetCounts}
              summary={scanSummary}
              onChooseRelocationDirectory={handleChooseRelocationDirectory}
              onChooseSourceDirectory={handleChooseSourceDirectory}
              onCancelScan={handleCancelScan}
              onDiscoverAll={handleDiscoverAll}
              onOpenLibrary={() => handleScreenChange("library")}
              onPreviewSourceRelocation={previewScanSourceRelocation}
              onRegisterSource={handleRegisterSource}
              onRelocateSource={handleRelocateSource}
              onRemoveDirectory={handleRemoveDirectory}
              onSetSourceEnabled={(source, enabled) => void handleSetSourceEnabled(source, enabled)}
              onStartScan={handleStartScan}
              onSyncEnabledSources={() => void handleSyncEnabledSources()}
              onSyncSource={(source) => void handleSyncSource(source.id)}
            />
          ) : activeScreen === "tags" ? (
            <TagManagementWorkspace
              activityMessage={globalActivityMessage}
              taggedClipCount={libraryFacets?.activeTaggedCount ?? 0}
              tagUsageCounts={tagUsageCounts}
              tags={libraryTags}
              totalClipCount={fullLibraryCount}
              onBack={() => handleScreenChange("library")}
              onCreateTag={handleCreateTag}
              onDeleteTag={handleDeleteTag}
              onUpdateTag={handleUpdateTag}
              onViewTag={handleViewTag}
            />
          ) : activeScreen === "settings" ? (
            <SettingsWorkspace
              criticalTaskMessage={
                isScanning
                  ? "扫描任务正在运行，请等待扫描结束后再安装"
                  : isPermanentDeleteActive
                    ? "永久删除任务正在处理，请等待任务结束后再安装"
                    : isExportActive
                      ? "视频导出任务正在运行，请等待导出结束后再安装"
                      : isSourceRelocationActive
                        ? "来源重新定位任务正在运行，请等待任务结束后再安装"
                        : null
              }
              onOpenScan={() => handleScreenChange("scan")}
              preferences={preferencesController}
              sourceDirs={sourceDirsWithFacetCounts}
              updater={appUpdater}
            />
          ) : (
            <PreviewWorkspace
              activityMessage={globalActivityMessage}
              clip={previewClip}
              clips={clips}
              detailError={detailState.error}
              detailStatus={previewDetailStatus}
              initialMuted={preferences.previewMuted}
              initialVolumePercent={preferences.previewVolumePercent}
              tags={libraryTags}
              onBack={() => handleScreenChange("library")}
              onCopyPath={handleCopyPath}
              onCreateTag={handleCreateTag}
              onManageTags={() => handleScreenChange("tags")}
              onAudioPreferenceChange={handleAudioPreferenceChange}
              onOpenExternal={handleOpenExternal}
              onOpenOriginal={handleOpenOriginal}
              onRetryDetail={handleRetryDetail}
              onSelectClip={handleSelectPreviewClip}
              onToggleFavorite={handleToggleFavorite}
              onToggleTag={handleToggleClipTag}
              onUpdateNote={handleUpdateNote}
            />
          )}
        </Suspense>

        {hasSidebarOverlay ? (
          <button
            aria-label="关闭素材导航"
            className="app-backdrop app-backdrop--sidebar"
            type="button"
            onClick={handleCloseSidebar}
          />
        ) : null}
        </div>
      </main>
    </MotionConfig>
  );
}

function startupNavigation(destination: StartupDestination): {
  screen: AppScreen;
  libraryMode: LibraryMode;
} {
  if (destination === "review") return { screen: "review", libraryMode: "all" };
  if (destination === "scan") return { screen: "scan", libraryMode: "all" };
  if (destination === "library-today") return { screen: "library", libraryMode: "today" };
  if (destination === "library-favorites") {
    return { screen: "library", libraryMode: "favorites" };
  }
  return { screen: "library", libraryMode: "all" };
}

function WorkspaceLoading() {
  return (
    <section aria-live="polite" className="workspace-loading" role="status">
      <span>正在加载工作区…</span>
    </section>
  );
}

function clipDetailPlaceholder(summary: ClipSummary): ClipDetail {
  return {
    ...summary,
    note: "",
    extractedText: "",
    eventCount: 0,
    clipEvents: [],
  };
}

type AppTopBarProps = {
  activeScreen: Exclude<AppScreen, "preview">;
  appVersion: string | null;
  hasAvailableUpdate: boolean;
  totalCount: number;
  isScanning: boolean;
  onOpenSidebar: (trigger: HTMLButtonElement) => void;
};

function AppTopBar({
  activeScreen,
  appVersion,
  hasAvailableUpdate,
  totalCount,
  isScanning,
  onOpenSidebar,
}: AppTopBarProps) {
  return (
    <header className="app-topbar" aria-label="应用导航">
      <button
        className={hasAvailableUpdate
          ? "topbar-menu-button topbar-menu-button--update"
          : "topbar-menu-button"}
        aria-label={hasAvailableUpdate ? "打开素材导航，有可用更新" : "打开素材导航"}
        type="button"
        onClick={(event) => onOpenSidebar(event.currentTarget)}
      >
        <UiIcon name="menu" />
        {hasAvailableUpdate ? <span aria-hidden="true" /> : null}
      </button>
      <div className="topbar-brand">
        <span className="topbar-mark" aria-hidden="true">
          <img alt="" src="/valoframe-mark.png" />
        </span>
        <div>
          <strong>瓦刻</strong>
          <span>VALOFRAME</span>
        </div>
      </div>
      <span className="topbar-context">
        {activeScreen === "scan"
          ? "扫描目录"
          : activeScreen === "review"
            ? "快速挑片"
            : activeScreen === "tags"
              ? "自定义标签"
              : activeScreen === "settings"
                ? "设置"
                : "素材库"}
      </span>
      <div className="topbar-status">
        <span
          className={
            isScanning ? "status-pulse status-pulse--busy" : "status-pulse"
          }
        />
        {isScanning
          ? "正在扫描"
          : activeScreen === "settings"
            ? `${appVersion ? `v${appVersion}` : "当前版本"} · Stable`
            : `${totalCount.toLocaleString("zh-CN")} 个素材`}
      </div>
    </header>
  );
}

function accountSummariesFromFacets(
  facets: LibraryFacets | null,
  selectedAccountId: string,
): AccountSummary[] {
  const latestModifiedAt = facets?.modifiedAtMax
    ? new Date(facets.modifiedAtMax * 1_000).toISOString()
    : new Date(0).toISOString();
  const accounts = (facets?.accounts ?? []).map((account): AccountSummary => ({
    id: account.accountIdentityKey,
    displayName: account.accountDisplayName,
    sourceName: "",
    clipCount: account.activeCount,
    missingCount: 0,
    favoriteCount: 0,
    sizeBytes: 0,
    lastModifiedAt: latestModifiedAt,
    detectedBy: account.accountIdentityKey.startsWith("source-") ? "source-dir" : "metadata",
  }));
  if (
    selectedAccountId !== "all" &&
    !accounts.some((account) => account.id === selectedAccountId)
  ) {
    accounts.push({
      id: selectedAccountId,
      displayName: selectedAccountId,
      sourceName: "",
      clipCount: 0,
      missingCount: 0,
      favoriteCount: 0,
      sizeBytes: 0,
      lastModifiedAt: new Date(0).toISOString(),
      detectedBy: selectedAccountId.startsWith("source-") ? "source-dir" : "metadata",
    });
  }
  return accounts;
}

function selectedFacetValues(
  facets: readonly LibraryFacetValue[] | undefined,
  selectedValue: string,
): string[] {
  const values = (facets ?? []).map((facet) => facet.value);
  if (selectedValue !== "all" && !values.includes(selectedValue)) {
    values.push(selectedValue);
  }
  return values;
}

function datePresetLabel(preset: LibraryDatePreset): string {
  if (preset === "today") return "今天";
  if (preset === "week") return "近 7 天";
  if (preset === "month") return "近 30 天";
  return "全部";
}

export default App;
