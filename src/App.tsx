import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useTransition,
} from "react";
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
import { normalizeCustomScanPath } from "./lib/customScan";
import { dateRangeForPreset } from "./lib/libraryFlow";
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
  scanTargetFromPath,
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
  SourceDir,
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

function App() {
  const didLoadInitialData = useRef(false);
  const [activeScreen, setActiveScreen] = useState<AppScreen>("library");
  const [sourceDirs, setSourceDirs] = useState<SourceDir[]>([]);
  const sourceDirsRef = useRef<SourceDir[]>([]);
  sourceDirsRef.current = sourceDirs;
  const [libraryMode, setLibraryMode] = useState<LibraryMode>("all");
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
  const [sortBy, setSortBy] = useState<ClipSort>("modified-desc");
  const [datePreset, setDatePreset] = useState<LibraryDatePreset>("all");
  const [highlightFilter, setHighlightFilter] = useState<HighlightFilter>("all");
  const [viewMode, setViewMode] = useState<LibraryViewMode>("grid");
  const [manualScanTargets, setManualScanTargets] = useState<ScanTarget[]>([]);
  const [excludedScanPaths, setExcludedScanPaths] = useState<Set<string>>(
    () => new Set(),
  );
  const [activityMessage, setActivityMessage] = useState("正在加载本地素材索引");
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);
  const [isFilterPending, startFilterTransition] = useTransition();
  const isSidebarOverlay = useMediaQuery("(max-width: 919px)");
  const menuTriggerRef = useRef<HTMLButtonElement | null>(null);
  const hasSidebarOverlay = isSidebarOverlay && isSidebarOpen;

  const effectiveDatePreset = libraryMode === "today" ? "today" : datePreset;
  const localDay = useLocalDay();
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
  const clearSelectedClip = useCallback((clipIds: ReadonlySet<string>) => {
    setSelectedClipId((current) => clipIds.has(current) ? "" : current);
  }, []);
  const {
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

  const loadSourceList = useCallback(async () => {
    try {
      const nextSourceDirs = await listSources();
      sourceDirsRef.current = nextSourceDirs;
      setSourceDirs(nextSourceDirs);
    } catch (error) {
      setActivityMessage(`来源加载失败：${commandErrorMessage(error)}`);
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

  const visibleClips = clips;

  const activeFilterLabels = useMemo(() => {
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

  const matchGroups = useMemo(
    () => groupClipsByMatch(visibleClips),
    [visibleClips],
  );

  const handleModeChange = useCallback((mode: LibraryMode) => {
    const next = transitionLibraryMode(fileStatus, mode);
    startFilterTransition(() => {
      setIsSidebarOpen(false);
      setActiveScreen("library");
      setLibraryMode(next.libraryMode);
      setFileStatus(next.fileStatus);
    });
  }, [fileStatus]);

  const handleAccountChange = useCallback((accountId: string) => {
    startFilterTransition(() => {
      setSelectedAccountId(accountId);
    });
  }, []);

  const handleAgentChange = useCallback((agentName: string) => {
    startFilterTransition(() => setSelectedAgentName(agentName));
  }, []);

  const handleMapChange = useCallback((mapName: string) => {
    startFilterTransition(() => setSelectedMapName(mapName));
  }, []);

  const handleGameModeChange = useCallback((gameMode: string) => {
    startFilterTransition(() => setSelectedGameMode(gameMode));
  }, []);

  const handleTagChange = useCallback((tagId: string) => {
    startFilterTransition(() => setSelectedTagId(tagId));
  }, []);

  const handleSortChange = useCallback((nextSortBy: ClipSort) => {
    startFilterTransition(() => setSortBy(nextSortBy));
  }, []);

  const handleClearAllFilters = () => {
    startFilterTransition(() => {
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
    setActiveScreen(screen);
  }, [cancelPendingDetail]);

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

  const refreshAfterScan = async () => {
    invalidateDetailCache();
    await loadSourceList();
    const [refreshed] = await Promise.all([
      refreshCurrentClipQuery({ preserveActivity: true }),
      loadTagList(),
      loadLibraryFacets(),
    ]);
    return refreshed;
  };

  const {
    activeJobId: activeScanJobId,
    status: scanStatus,
    progress: scanProgress,
    summary: scanSummary,
    errorMessage: scanError,
    isScanning,
    startScan: handleStartScan,
    discoverAll: handleDiscoverAll,
    cancelScan: handleCancelScan,
    clearOutcome: clearScanOutcome,
    clearSummary: clearScanSummary,
    reportError: setScanError,
  } = useScanController({
    sourcePaths: scanTargets.map((target) => target.path),
    refresh: refreshAfterScan,
    notify: ({ message }) => setActivityMessage(message),
  });

  const handleAddDirectory = async () => {
    if (isScanning) {
      return;
    }

    if (!isTauri()) {
      setScanError("目录选择仅在桌面应用中可用，请使用 npm run tauri dev 启动应用。");
      setActivityMessage("无法在浏览器预览中选择本机目录");
      return;
    }

    try {
      const path = normalizeCustomScanPath(await open({
        directory: true,
        multiple: false,
        title: "选择 wonderfulVideos 上级目录",
      }));
      if (!path) {
        setActivityMessage("已取消添加目录");
        return;
      }

      const target = scanTargetFromPath(path);
      setManualScanTargets((current) => [
        ...current.filter((candidate) => scanPathKey(candidate.path) !== scanPathKey(path)),
        target,
      ]);
      setExcludedScanPaths((current) => {
        const next = new Set(current);
        next.delete(scanPathKey(path));
        return next;
      });
      clearScanOutcome();
      setActivityMessage(`已加入扫描队列：${target.name}`);
    } catch (error) {
      setScanError(`无法打开目录选择窗口：${commandErrorMessage(error)}`);
      setActivityMessage("添加目录失败");
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

  const handleExportClips = useCallback(async (clipIds: string[]): Promise<boolean> => {
    if (clipIds.length === 0) return false;
    if (!isTauri()) {
      setActivityMessage("导出视频仅在桌面应用中可用");
      return false;
    }

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
    }
  }, []);

  return (
    <main className={`app-root app-root--${activeScreen}`}>
      <AmbientBackdrop />
      {activeScreen !== "preview" ? (
        <AppTopBar
          activeScreen={activeScreen}
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
            onClose={handleCloseSidebar}
            onModeChange={handleModeChange}
            onOpenScan={() => handleScreenChange("scan")}
            onOpenTagManager={() => handleScreenChange("tags")}
          />
        ) : null}

        <Suspense fallback={<WorkspaceLoading />}>
          <LibraryWorkspace
            accountId={selectedAccountId}
            accounts={accounts}
            activeFilterLabels={activeFilterLabels}
            activityMessage={activityMessage}
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
            mapName={selectedMapName}
            mapNames={mapNames}
            matchGroups={matchGroups}
            hasMore={hasMoreClips}
            listGeneration={clipListGeneration}
            loadMoreError={loadMoreError}
            libraryMode={libraryMode}
            query={query}
            scrollResetKey={productionListQueryGenerationKey}
            selectedClipId={selectedClipId}
            sortBy={sortBy}
            tagId={selectedTagId}
            tags={libraryTags}
            totalClipCount={totalClipCount}
            viewMode={viewMode}
            visibleClipCount={visibleClips.length}
            onAccountChange={handleAccountChange}
            onAgentChange={handleAgentChange}
            onClearFilters={handleClearAllFilters}
            onCopyPath={handleCopyPath}
            onCreateTag={handleCreateTag}
            onExportClips={handleExportClips}
            onDatePresetChange={(value) => startFilterTransition(() => {
              setLibraryMode("all");
              setDatePreset(value);
            })}
            onGameModeChange={handleGameModeChange}
            onHighlightFilterChange={(value) => startFilterTransition(() => setHighlightFilter(value))}
            onMapChange={handleMapChange}
            onOpenOriginal={handleOpenOriginal}
            onOpenScan={() => handleScreenChange("scan")}
            onLoadAll={loadAllClips}
            onLoadMore={() => void loadMoreClips()}
            onQueryChange={setQuery}
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
            onViewModeChange={setViewMode}
          />
          {activeScreen === "library" ? null : activeScreen === "scan" ? (
            <ScanWorkspace
            activeJobId={activeScanJobId}
            accounts={accounts}
            activityMessage={activityMessage}
            errorMessage={scanError ?? listError}
            facets={libraryFacets}
            isLoading={isLoadingClips}
            isScanning={isScanning}
            progress={scanProgress}
            scanStatus={scanStatus}
            scanTargets={scanTargets}
            sourceDirs={sourceDirsWithFacetCounts}
            summary={scanSummary}
            onAddDirectory={handleAddDirectory}
            onCancelScan={handleCancelScan}
            onDiscoverAll={handleDiscoverAll}
            onOpenLibrary={() => handleScreenChange("library")}
            onRemoveDirectory={handleRemoveDirectory}
            onStartScan={handleStartScan}
            />
          ) : activeScreen === "tags" ? (
            <TagManagementWorkspace
            activityMessage={activityMessage}
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
          ) : (
            <PreviewWorkspace
            activityMessage={activityMessage}
            clip={previewClip}
            clips={clips}
            detailError={detailState.error}
            detailStatus={previewDetailStatus}
            tags={libraryTags}
            onBack={() => handleScreenChange("library")}
            onCopyPath={handleCopyPath}
            onCreateTag={handleCreateTag}
            onManageTags={() => handleScreenChange("tags")}
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
  );
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
  totalCount: number;
  isScanning: boolean;
  onOpenSidebar: (trigger: HTMLButtonElement) => void;
};

function AppTopBar({
  activeScreen,
  totalCount,
  isScanning,
  onOpenSidebar,
}: AppTopBarProps) {
  return (
    <header className="app-topbar" aria-label="应用导航">
      <button
        className="topbar-menu-button"
        aria-label="打开素材导航"
        type="button"
        onClick={(event) => onOpenSidebar(event.currentTarget)}
      >
        <UiIcon name="menu" />
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
          : activeScreen === "tags"
            ? "自定义标签"
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
