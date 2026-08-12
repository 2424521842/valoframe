import {
  ArrowClockwise,
  ArrowCounterClockwise,
  Database,
  Export,
  Heart,
  Tag as TagIcon,
  Trash,
  X,
} from "@phosphor-icons/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BatchTagDialog } from "../components/BatchTagDialog";
import { LibraryToolbar } from "../components/LibraryToolbar";
import { MatchLibrary } from "../components/MatchLibrary";
import {
  UiAlertDialog,
  UiAlertDialogAction,
  UiAlertDialogCancel,
  UiAlertDialogContent,
  UiAlertDialogDescription,
  UiAlertDialogTitle,
} from "../components/ui/alert-dialog";
import { UiCheckbox } from "../components/ui/checkbox";
import {
  pruneClipSelection,
  toggleAllVisibleClipSelection,
  updateClipSelection,
  type ClipSelectionGesture,
} from "../lib/clipSelection";
import type {
  AccountSummary,
  ClipSummary,
  ClipSort,
  ClipMatchGroup,
  HighlightFilter,
  LibraryDatePreset,
  LibraryMode,
  LibraryViewMode,
  RemoveClipsFromIndexResult,
  SourceDir,
  Tag,
  TagColor,
} from "../types";
import type { VideoTypeFilter } from "../lib/videoTypes";

type LibraryWorkspaceProps = {
  accounts: AccountSummary[];
  sourceDirs: SourceDir[];
  matchGroups: ClipMatchGroup[];
  tags: Tag[];
  totalClipCount: number;
  visibleClipCount: number;
  activityMessage: string;
  isLoading: boolean;
  isLoadingMore: boolean;
  isPending: boolean;
  isScanning: boolean;
  isActive: boolean;
  errorMessage: string | null;
  facetError: string | null;
  isFacetLoading: boolean;
  loadMoreError: string | null;
  hasMore: boolean;
  listGeneration: number;
  query: string;
  accountId: string;
  agentNames: string[];
  agentName: string;
  mapNames: string[];
  mapName: string;
  gameModes: string[];
  gameMode: string;
  tagId: string;
  datePreset: LibraryDatePreset;
  highlightFilter: HighlightFilter;
  videoTypes: readonly VideoTypeFilter[];
  sortBy: ClipSort;
  viewMode: LibraryViewMode;
  libraryMode: LibraryMode;
  selectedClipId: string;
  activeFilterLabels: string[];
  initialSelectedClipIds?: readonly string[];
  selectionRequestId?: string | null;
  openTagDialogForInitialSelection?: boolean;
  onQueryChange: (value: string) => void;
  onAccountChange: (value: string) => void;
  onAgentChange: (value: string) => void;
  onMapChange: (value: string) => void;
  onGameModeChange: (value: string) => void;
  onTagChange: (value: string) => void;
  onDatePresetChange: (value: LibraryDatePreset) => void;
  onHighlightFilterChange: (value: HighlightFilter) => void;
  onSortChange: (value: ClipSort) => void;
  onViewModeChange: (value: LibraryViewMode) => void;
  onClearFilters: () => void;
  onOpenScan: () => void;
  onRefresh: () => void;
  onRetryLoad: () => void;
  onLoadMore: () => void;
  onLoadAll: () => Promise<ClipSummary[] | null>;
  onRetryLoadMore: () => void;
  onSelectClip: (clipId: string, trigger: HTMLElement) => void;
  onToggleFavorite: (clipId: string) => void;
  onCopyPath: (clipId: string) => void;
  onOpenOriginal: (clipId: string) => void;
  onCreateTag: (name: string, color?: TagColor) => Promise<Tag | null>;
  onExportClips: (clipIds: string[]) => Promise<boolean>;
  onSetFavoriteForClips: (clipIds: string[], isFavorite: boolean) => Promise<boolean>;
  onSetTagForClips: (clipIds: string[], tagId: string, shouldAttach: boolean) => Promise<boolean>;
  onSetTrashedForClips: (clipIds: string[], isTrashed: boolean) => Promise<boolean>;
  onDeleteClipsPermanently: (clipIds: string[]) => Promise<boolean>;
  onRemoveClipsFromIndex: (clipIds: string[]) => Promise<RemoveClipsFromIndexResult | null>;
};

type PendingDestructiveAction = {
  kind: "trash" | "remove-index" | "delete-file";
  clipIds: string[];
};

export function LibraryWorkspace(props: LibraryWorkspaceProps) {
  const hasActiveFilters = props.activeFilterLabels.length > 0;
  const isTrashMode = props.libraryMode === "trash";
  const [selectedClipIds, setSelectedClipIds] = useState<Set<string>>(() => new Set());
  const [tagDialogOpen, setTagDialogOpen] = useState(false);
  const [pendingAction, setPendingAction] = useState<PendingDestructiveAction | null>(null);
  const [isBatchBusy, setIsBatchBusy] = useState(false);
  const [isSelectingAll, setIsSelectingAll] = useState(false);
  const [indexRemovalFeedback, setIndexRemovalFeedback] = useState("");
  const scrollElementRef = useRef<HTMLDivElement>(null);
  const visibleClipIds = useMemo(
    () => props.matchGroups.flatMap((group) => group.clips.map((clip) => clip.id)),
    [props.matchGroups],
  );
  const visibleClips = useMemo(
    () => props.matchGroups.flatMap((group) => group.clips),
    [props.matchGroups],
  );
  const clipById = useMemo(
    () => new Map(visibleClips.map((clip) => [clip.id, clip])),
    [visibleClips],
  );
  const selectedClipSnapshotsRef = useRef<Map<string, ClipSummary>>(new Map());
  for (const clipId of selectedClipIds) {
    const visibleClip = clipById.get(clipId);
    if (visibleClip) selectedClipSnapshotsRef.current.set(clipId, visibleClip);
  }
  const selectedClips = useMemo(
    () => [...selectedClipIds]
      .map((id) => clipById.get(id) ?? selectedClipSnapshotsRef.current.get(id))
      .filter((clip): clip is ClipSummary => Boolean(clip)),
    [clipById, selectedClipIds],
  );
  const unavailableSourceIds = useMemo(
    () => new Set(
      props.sourceDirs
        .filter((source) => source.status === "unavailable")
        .map((source) => source.id),
    ),
    [props.sourceDirs],
  );
  const canRemoveClipFromIndex = useCallback(
    (clip: ClipSummary) => clip.fileStatus === "trashed"
      || clip.fileStatus === "missing"
      || unavailableSourceIds.has(clip.sourceDirId),
    [unavailableSourceIds],
  );
  const removableSelectedClipIds = useMemo(
    () => selectedClips.filter(canRemoveClipFromIndex).map((clip) => clip.id),
    [canRemoveClipFromIndex, selectedClips],
  );
  const removableVisibleClipIds = useMemo(
    () => new Set(visibleClips.filter(canRemoveClipFromIndex).map((clip) => clip.id)),
    [canRemoveClipFromIndex, visibleClips],
  );
  const selectionAnchorRef = useRef("");
  const selectedClipIdsRef = useRef(selectedClipIds);
  const selectionVersionRef = useRef(0);
  const visibleClipIdsRef = useRef(visibleClipIds);
  const isBatchBusyRef = useRef(isBatchBusy);
  const isSelectingAllRef = useRef(isSelectingAll);
  const selectAllRequestRef = useRef(0);
  const pendingSelectAllCountRef = useRef(0);
  const appliedSelectionRequestRef = useRef<string | null>(null);
  selectedClipIdsRef.current = selectedClipIds;
  visibleClipIdsRef.current = visibleClipIds;
  isBatchBusyRef.current = isBatchBusy;
  isSelectingAllRef.current = isSelectingAll;
  const allResultsSelected = props.totalClipCount > 0
    && selectedClipIds.size === props.totalClipCount
    && visibleClipIds.every((id) => selectedClipIds.has(id));
  const selectionCheckboxState = allResultsSelected
    ? true
    : selectedClipIds.size > 0
      ? "indeterminate"
      : false;

  const handleSelectionGesture = useCallback((clipId: string, gesture: ClipSelectionGesture) => {
    setSelectedClipIds((current) => {
      const next = updateClipSelection(
        current,
        visibleClipIdsRef.current,
        clipId,
        selectionAnchorRef.current,
        gesture,
      );
      selectionAnchorRef.current = next.anchorId;
      selectedClipIdsRef.current = next.selectedIds;
      selectionVersionRef.current += 1;
      return next.selectedIds;
    });
  }, []);

  const clearSelection = useCallback(() => {
    const emptySelection = new Set<string>();
    selectionAnchorRef.current = "";
    selectedClipSnapshotsRef.current.clear();
    selectedClipIdsRef.current = emptySelection;
    selectionVersionRef.current += 1;
    setSelectedClipIds(emptySelection);
  }, []);

  const toggleAllResults = useCallback(async () => {
    if (isBatchBusyRef.current || isSelectingAllRef.current) return;
    if (
      props.totalClipCount > 0
      && selectedClipIdsRef.current.size === props.totalClipCount
      && visibleClipIdsRef.current.every((id) => selectedClipIdsRef.current.has(id))
    ) {
      clearSelection();
      return;
    }

    const requestId = selectAllRequestRef.current + 1;
    selectAllRequestRef.current = requestId;
    isSelectingAllRef.current = true;
    setIsSelectingAll(true);
    try {
      const allClips = props.hasMore
        ? await props.onLoadAll()
        : visibleClips;
      if (!allClips || selectAllRequestRef.current !== requestId) return;

      const next = toggleAllVisibleClipSelection(
        new Set<string>(),
        allClips.map((clip) => clip.id),
      );
      pendingSelectAllCountRef.current = allClips.length;
      selectedClipSnapshotsRef.current = new Map(allClips.map((clip) => [clip.id, clip]));
      selectionAnchorRef.current = allClips[0]?.id ?? "";
      selectedClipIdsRef.current = next;
      selectionVersionRef.current += 1;
      setSelectedClipIds(next);
    } finally {
      if (selectAllRequestRef.current === requestId) {
        isSelectingAllRef.current = false;
        setIsSelectingAll(false);
      }
    }
  }, [
    clearSelection,
    props.hasMore,
    props.onLoadAll,
    props.totalClipCount,
    visibleClips,
  ]);

  const targetIdsForCard = useCallback((clipId: string): string[] => {
    const current = selectedClipIdsRef.current;
    if (current.has(clipId)) return [...current];
    const next = new Set([clipId]);
    selectionAnchorRef.current = clipId;
    selectedClipIdsRef.current = next;
    selectionVersionRef.current += 1;
    setSelectedClipIds(next);
    return [clipId];
  }, []);

  const runBatchAction = useCallback(async (
    action: () => Promise<boolean>,
    clearOnSuccess = false,
  ) => {
    if (isBatchBusyRef.current) return false;
    const selectionVersion = selectionVersionRef.current;
    isBatchBusyRef.current = true;
    setIsBatchBusy(true);
    try {
      const succeeded = await action();
      if (
        succeeded &&
        clearOnSuccess &&
        selectionVersionRef.current === selectionVersion
      ) {
        clearSelection();
      }
      return succeeded;
    } finally {
      isBatchBusyRef.current = false;
      setIsBatchBusy(false);
    }
  }, [clearSelection]);

  useEffect(() => {
    setSelectedClipIds((current) => {
      if (
        pendingSelectAllCountRef.current > visibleClipIds.length
        && current.size === pendingSelectAllCountRef.current
      ) {
        return current;
      }
      pendingSelectAllCountRef.current = 0;
      const pruned = pruneClipSelection(current, visibleClipIds);
      if (pruned.size === current.size && [...pruned].every((id) => current.has(id))) return current;
      selectedClipIdsRef.current = pruned;
      selectionVersionRef.current += 1;
      return pruned;
    });
    if (selectionAnchorRef.current && !visibleClipIds.includes(selectionAnchorRef.current)) {
      selectionAnchorRef.current = "";
    }
  }, [visibleClipIds]);

  useEffect(() => {
    const requestId = props.selectionRequestId ?? null;
    if (!requestId || appliedSelectionRequestRef.current === requestId) return;
    appliedSelectionRequestRef.current = requestId;
    const visibleIds = new Set(visibleClipIds);
    const initialIds = props.initialSelectedClipIds?.filter((clipId) => visibleIds.has(clipId)) ?? [];
    const nextSelection = new Set(initialIds);
    selectedClipSnapshotsRef.current = new Map(
      visibleClips
        .filter((clip) => nextSelection.has(clip.id))
        .map((clip) => [clip.id, clip]),
    );
    selectedClipIdsRef.current = nextSelection;
    selectionAnchorRef.current = initialIds[0] ?? "";
    selectionVersionRef.current += 1;
    setSelectedClipIds(nextSelection);
    if (props.openTagDialogForInitialSelection && initialIds.length > 0) {
      setTagDialogOpen(true);
    }
  }, [props.initialSelectedClipIds, props.openTagDialogForInitialSelection, props.selectionRequestId, visibleClipIds, visibleClips]);

  useEffect(() => {
    selectAllRequestRef.current += 1;
    pendingSelectAllCountRef.current = 0;
    isSelectingAllRef.current = false;
    setIsSelectingAll(false);
  }, [props.listGeneration]);

  useEffect(() => {
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || tagDialogOpen || pendingAction) return;
      clearSelection();
    };
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [clearSelection, pendingAction, tagDialogOpen]);

  const handleFavoriteSelected = () => {
    const shouldFavorite = !selectedClips.every((clip) => clip.isFavorite);
    void runBatchAction(() => props.onSetFavoriteForClips([...selectedClipIds], shouldFavorite));
  };

  const handleExportSelected = () => {
    void runBatchAction(() => props.onExportClips([...selectedClipIdsRef.current]));
  };

  const handleRestore = useCallback((clipId?: string) => {
    const clipIds = clipId ? targetIdsForCard(clipId) : [...selectedClipIdsRef.current];
    void runBatchAction(() => props.onSetTrashedForClips(clipIds, false), true);
  }, [props.onSetTrashedForClips, runBatchAction, targetIdsForCard]);

  const requestDestructiveAction = useCallback((kind: PendingDestructiveAction["kind"], clipId?: string) => {
    const clipIds = clipId ? targetIdsForCard(clipId) : [...selectedClipIdsRef.current];
    if (clipIds.length > 0) {
      if (kind === "remove-index") setIndexRemovalFeedback("");
      setPendingAction({ kind, clipIds });
    }
  }, [targetIdsForCard]);

  const requestTrashForCard = useCallback(
    (clipId: string) => requestDestructiveAction("trash", clipId),
    [requestDestructiveAction],
  );

  const requestPermanentRemoveForCard = useCallback(
    (clipId: string) => requestDestructiveAction("remove-index", clipId),
    [requestDestructiveAction],
  );

  const requestSelectedIndexRemoval = useCallback(() => {
    if (removableSelectedClipIds.length > 0) {
      setIndexRemovalFeedback("");
      setPendingAction({ kind: "remove-index", clipIds: removableSelectedClipIds });
    }
  }, [removableSelectedClipIds]);

  const requestPermanentDeleteForCard = useCallback(
    (clipId: string) => requestDestructiveAction("delete-file", clipId),
    [requestDestructiveAction],
  );

  const confirmDestructiveAction = async () => {
    if (!pendingAction) return;
    const action = pendingAction;
    if (action.kind === "remove-index") {
      if (isBatchBusyRef.current) return;
      isBatchBusyRef.current = true;
      setIsBatchBusy(true);
      setIndexRemovalFeedback("");
      try {
        const result = await props.onRemoveClipsFromIndex(action.clipIds);
        if (!result) {
          setIndexRemovalFeedback("仅移除索引失败：本次未移除任何记录，请重试。");
          return;
        }
        const completedIds = new Set([...result.removedIds, ...result.missingIds]);
        setSelectedClipIds((current) => {
          const next = new Set([...current].filter((clipId) => !completedIds.has(clipId)));
          for (const clipId of completedIds) selectedClipSnapshotsRef.current.delete(clipId);
          pendingSelectAllCountRef.current = next.size;
          selectedClipIdsRef.current = next;
          selectionVersionRef.current += 1;
          return next;
        });
        const retryIds = [...result.blocked, ...result.failures].map((problem) => problem.clipId);
        setIndexRemovalFeedback(indexRemovalResultMessage(result));
        setPendingAction(retryIds.length > 0
          ? { kind: "remove-index", clipIds: retryIds }
          : null);
      } catch (error) {
        setIndexRemovalFeedback(`仅移除索引失败：${errorMessage(error)}；本次未移除任何记录。`);
      } finally {
        isBatchBusyRef.current = false;
        setIsBatchBusy(false);
      }
      return;
    }
    const succeeded = await runBatchAction(
      () => action.kind === "trash"
        ? props.onSetTrashedForClips(action.clipIds, true)
        : props.onDeleteClipsPermanently(action.clipIds),
      true,
    );
    if (succeeded) setPendingAction(null);
  };

  return (
    <section
      className={`library-workspace${selectedClipIds.size > 0 ? " library-workspace--selecting" : ""}`}
      aria-label="素材库对局浏览"
      hidden={!props.isActive}
      style={props.isActive ? undefined : { display: "none" }}
    >
      <header className="library-workspace-heading">
        <div>
          <span className="cinematic-eyebrow">TACTICAL ARCHIVE / MATCH INDEX</span>
          <h1>{isTrashMode ? "回收站 / 素材恢复" : "素材库 / 对局分组浏览"}</h1>
          <p>{isTrashMode ? "回收素材默认保留本地视频，可恢复、仅移除索引，或经二次确认后永久删除视频。" : "扫描后的素材按账号与对局自动归组，筛选条件会立即作用于分组结果。"}</p>
        </div>
        <div className="library-heading-status" aria-live="polite">
          <strong>{props.matchGroups.length} 个对局</strong>
          <span>{props.visibleClipCount} / {props.totalClipCount} 条片段</span>
          <small>{props.isPending ? "正在更新筛选…" : props.activityMessage}</small>
        </div>
      </header>

      {props.facetError ? (
        <p className="library-load-more-error" role="status">{props.facetError}</p>
      ) : props.isFacetLoading ? (
        <p className="library-load-more-status" role="status">正在加载全库筛选统计…</p>
      ) : null}

      <LibraryToolbar
        accountId={props.accountId}
        accounts={props.accounts}
        agentName={props.agentName}
        agentNames={props.agentNames}
        datePreset={props.datePreset}
        gameMode={props.gameMode}
        gameModes={props.gameModes}
        hasActiveFilters={hasActiveFilters}
        highlightFilter={props.highlightFilter}
        videoTypes={props.videoTypes}
        mapName={props.mapName}
        mapNames={props.mapNames}
        query={props.query}
        sortBy={props.sortBy}
        tagId={props.tagId}
        tags={props.tags}
        viewMode={props.viewMode}
        onAccountChange={props.onAccountChange}
        onAgentChange={props.onAgentChange}
        onClearAll={props.onClearFilters}
        onDatePresetChange={props.onDatePresetChange}
        onGameModeChange={props.onGameModeChange}
        onHighlightFilterChange={props.onHighlightFilterChange}
        onMapChange={props.onMapChange}
        onQueryChange={props.onQueryChange}
        onSortChange={props.onSortChange}
        onTagChange={props.onTagChange}
        onViewModeChange={props.onViewModeChange}
      />

      <div className="library-workspace-scroll" ref={scrollElementRef}>
        <MatchLibrary
          activeFilterLabels={props.activeFilterLabels}
          errorMessage={props.errorMessage}
          isLoading={props.isLoading}
          isLoadingMore={props.isLoadingMore}
          isTrashMode={isTrashMode}
          removableFromIndexIds={removableVisibleClipIds}
          hasMore={props.hasMore}
          listGeneration={props.listGeneration}
          loadMoreError={props.loadMoreError}
          matchGroups={props.matchGroups}
          selectedClipId={props.selectedClipId}
          selectedClipIds={selectedClipIds}
          tags={props.tags}
          totalClipCount={props.totalClipCount}
          viewMode={props.viewMode}
          scrollElementRef={scrollElementRef}
          onClearFilters={props.onClearFilters}
          onCopyPath={props.onCopyPath}
          onOpenOriginal={props.onOpenOriginal}
          onOpenScan={props.onOpenScan}
          onLoadMore={props.onLoadMore}
          onRequestPermanentDelete={requestPermanentDeleteForCard}
          onRequestPermanentRemove={requestPermanentRemoveForCard}
          onRequestTrash={requestTrashForCard}
          onRestoreClip={handleRestore}
          onRetryLoad={props.onRetryLoad}
          onRetryLoadMore={props.onRetryLoadMore}
          onSelectClip={props.onSelectClip}
          onSelectionGesture={handleSelectionGesture}
          onToggleFavorite={props.onToggleFavorite}
        />
      </div>

      {selectedClipIds.size > 0 ? (
        <aside aria-label="批量操作" className="library-batch-toolbar">
          <div>
            <strong>已选择 {selectedClipIds.size} 条素材</strong>
            <span>Ctrl 点击多选 · Shift 点击连续选择 · Esc 取消</span>
          </div>
          <button disabled={isBatchBusy} type="button" onClick={handleFavoriteSelected}>
            <Heart weight={selectedClips.every((clip) => clip.isFavorite) ? "fill" : "bold"} />
            {selectedClips.every((clip) => clip.isFavorite) ? "取消收藏" : "收藏"}
          </button>
          <button disabled={isBatchBusy} type="button" onClick={() => setTagDialogOpen(true)}>
            <TagIcon weight="bold" />
            自定义标签
          </button>
          <button disabled={isBatchBusy} type="button" onClick={handleExportSelected}>
            <Export weight="bold" />
            导出所选
          </button>
          {isTrashMode ? (
            <>
              <button disabled={isBatchBusy} type="button" onClick={() => handleRestore()}>
                <ArrowCounterClockwise weight="bold" />
                恢复
              </button>
              <button className="library-batch-danger" disabled={isBatchBusy} type="button" onClick={() => requestDestructiveAction("remove-index")}>
                <Database weight="bold" />
                仅移除索引
              </button>
              <button className="library-batch-danger library-batch-danger--strong" disabled={isBatchBusy} type="button" onClick={() => requestDestructiveAction("delete-file")}>
                <Trash weight="fill" />
                永久删除视频
              </button>
            </>
          ) : (
            <>
              {removableSelectedClipIds.length > 0 ? (
                <button className="library-batch-danger" disabled={isBatchBusy} type="button" onClick={requestSelectedIndexRemoval}>
                  <Database weight="bold" />
                  仅移除失联索引 ({removableSelectedClipIds.length})
                </button>
              ) : null}
              <button className="library-batch-danger" disabled={isBatchBusy} type="button" onClick={() => requestDestructiveAction("trash")}>
                <Trash weight="bold" />
                移入回收站
              </button>
            </>
          )}
          <button aria-label="取消全部选择" className="library-batch-close" disabled={isBatchBusy} type="button" onClick={clearSelection}>
            <X weight="bold" />
          </button>
        </aside>
      ) : null}

      <footer className="library-workspace-footer">
        <label className="library-select-all">
          <UiCheckbox
            aria-label={allResultsSelected
              ? "取消选择全部结果"
              : `选择全部 ${props.totalClipCount} 条结果`}
            checked={selectionCheckboxState}
            disabled={
              props.totalClipCount === 0
              || isBatchBusy
              || isSelectingAll
              || props.isLoading
              || props.isLoadingMore
            }
            onCheckedChange={() => void toggleAllResults()}
          />
          <span>
            {isSelectingAll
              ? "正在加载并选择全部结果…"
              : allResultsSelected
                ? "取消全选"
                : `全选全部 ${props.totalClipCount} 条结果`}
          </span>
          <small>已加载 {props.visibleClipCount} 条 / 共 {props.totalClipCount} 条</small>
        </label>
        <button disabled={props.isScanning || props.isLoading || isBatchBusy || isSelectingAll} type="button" onClick={props.onRefresh}>
          <ArrowClockwise weight="bold" />
          {props.isScanning ? "扫描中" : "刷新索引"}
        </button>
      </footer>

      <BatchTagDialog
        isBusy={isBatchBusy}
        open={tagDialogOpen}
        selectedClips={selectedClips}
        tags={props.tags}
        onCreateTag={props.onCreateTag}
        onOpenChange={setTagDialogOpen}
        onSetTag={(tagId, shouldAttach) => runBatchAction(
          () => props.onSetTagForClips([...selectedClipIds], tagId, shouldAttach),
        )}
      />

      <UiAlertDialog open={Boolean(pendingAction)} onOpenChange={(open) => !open && !isBatchBusy && setPendingAction(null)}>
        <UiAlertDialogContent>
          <UiAlertDialogTitle>
            {pendingAction?.kind === "trash"
              ? "将所选素材移入回收站？"
              : pendingAction?.kind === "remove-index"
                ? "从管理器索引中永久移除？"
                : "永久删除所选视频？"}
          </UiAlertDialogTitle>
          <UiAlertDialogDescription>
            {pendingAction?.kind === "trash"
              ? `将隐藏 ${pendingAction.clipIds.length} 条素材，但不会删除本地视频文件，可以随时从回收站恢复。`
              : pendingAction?.kind === "remove-index"
                ? `将从瓦刻数据库永久移除 ${pendingAction?.clipIds.length ?? 0} 条索引记录，并删除其中的收藏、标签、备注、评审决定、缩略图状态和结构化元数据。绝不会删除、移动或修改磁盘上的视频文件；后续重新扫描可能再次导入。`
                : `将永久删除 ${pendingAction?.clipIds.length ?? 0} 个本地视频文件，并同时移除管理器记录。此操作无法撤销，文件不会进入系统回收站。`}
          </UiAlertDialogDescription>
          {pendingAction?.kind === "remove-index" && indexRemovalFeedback ? (
            <p
              aria-live="assertive"
              className="library-index-removal-feedback"
              role="alert"
            >
              {indexRemovalFeedback}
            </p>
          ) : null}
          <div className="ui-alert-dialog-actions">
            <UiAlertDialogCancel disabled={isBatchBusy}>取消</UiAlertDialogCancel>
            <UiAlertDialogAction
              disabled={isBatchBusy}
              onClick={(event) => {
                event.preventDefault();
                void confirmDestructiveAction();
              }}
            >
              {isBatchBusy
                ? "正在处理…"
                : pendingAction?.kind === "trash"
                  ? "移入回收站"
                  : pendingAction?.kind === "remove-index"
                    ? "仅移除索引"
                    : "永久删除视频"}
            </UiAlertDialogAction>
          </div>
        </UiAlertDialogContent>
      </UiAlertDialog>
    </section>
  );
}

function indexRemovalResultMessage(result: RemoveClipsFromIndexResult): string {
  const firstProblem = result.blocked[0] ?? result.failures[0];
  const message = [
    `本次成功移除 ${result.removedIds.length} 条`,
    `索引已不存在 ${result.missingIds.length} 条`,
    `阻断 ${result.blocked.length} 条`,
    `失败 ${result.failures.length} 条`,
  ].join("；");
  return firstProblem ? `${message}。${firstProblem.message}` : `${message}。`;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  return "未知错误";
}
