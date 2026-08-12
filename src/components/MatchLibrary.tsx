import { useVirtualizer } from "@tanstack/react-virtual";
import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { m, useReducedMotion } from "motion/react";
import {
  ArrowCounterClockwise,
  CaretUp,
  CheckSquare,
  Copy,
  Database,
  FolderOpen,
  Heart,
  Play,
  Trash,
} from "@phosphor-icons/react";
import { displayHighlightTitle } from "../api/backend";
import { agentInitial } from "../lib/agentChip";
import { formatBytes, formatDateTime } from "../lib/formatters";
import { motionProfile, type MotionProfile } from "../lib/motionProfile";
import type { ClipSelectionGesture } from "../lib/clipSelection";
import { cn } from "../lib/classNames";
import { expectsOfficialRoundScore } from "../lib/videoTypes";
import {
  valorantAgentDisplayIconUrl,
  valorantMapListViewIconUrl,
} from "../lib/valorantAssets";
import type { ClipMatchGroup, ClipSummary, LibraryViewMode, Tag } from "../types";
import { ThumbnailImage } from "./ThumbnailImage";
import { UiCheckbox } from "./ui/checkbox";
import {
  UiContextMenu,
  UiContextMenuContent,
  UiContextMenuItem,
  UiContextMenuLabel,
  UiContextMenuSeparator,
  UiContextMenuTrigger,
} from "./ui/context-menu";

type MatchLibraryProps = {
  matchGroups: ClipMatchGroup[];
  activeFilterLabels: string[];
  selectedClipId: string;
  selectedClipIds: ReadonlySet<string>;
  tags: Tag[];
  totalClipCount: number;
  viewMode: LibraryViewMode;
  isLoading: boolean;
  isLoadingMore?: boolean;
  hasMore?: boolean;
  loadMoreError?: string | null;
  listGeneration?: number;
  scrollElementRef?: RefObject<HTMLDivElement | null>;
  errorMessage: string | null;
  onClearFilters: () => void;
  onRetryLoad: () => void;
  onLoadMore?: () => void;
  onRetryLoadMore?: () => void;
  onOpenScan: () => void;
  onSelectClip: (clipId: string, trigger: HTMLElement) => void;
  onToggleFavorite: (clipId: string) => void;
  onCopyPath: (clipId: string) => void;
  onOpenOriginal: (clipId: string) => void;
  isTrashMode: boolean;
  removableFromIndexIds: ReadonlySet<string>;
  onSelectionGesture: (clipId: string, gesture: ClipSelectionGesture) => void;
  onRequestTrash: (clipId: string) => void;
  onRequestPermanentDelete: (clipId: string) => void;
  onRequestPermanentRemove: (clipId: string) => void;
  onRestoreClip: (clipId: string) => void;
};

const EAGER_CLIP_LIMIT = 48;
const DEFAULT_SCROLL_VIEWPORT = { width: 1200, height: 800, isCompact: false };
const GRID_MIN_CARD_WIDTH = 226;
const GRID_GAP = 12;
const COMPACT_GRID_GAP = 10;
const MATCH_HEADER_HEIGHT = 52;
const MATCH_ROW_BOTTOM_GAP = 10;
const GRID_CARD_FOOTER_HEIGHT = 59;
const GRID_CARD_TAGGED_FOOTER_HEIGHT = 73;

export const MatchLibrary = memo(function MatchLibrary({
  matchGroups,
  activeFilterLabels,
  selectedClipId,
  selectedClipIds,
  tags,
  totalClipCount,
  viewMode,
  isLoading,
  isLoadingMore = false,
  hasMore = false,
  loadMoreError = null,
  listGeneration = 0,
  scrollElementRef,
  errorMessage,
  onClearFilters,
  onRetryLoad,
  onLoadMore = () => undefined,
  onRetryLoadMore = () => undefined,
  onOpenScan,
  onSelectClip,
  onToggleFavorite,
  onCopyPath,
  onOpenOriginal,
  isTrashMode,
  removableFromIndexIds,
  onSelectionGesture,
  onRequestTrash,
  onRequestPermanentDelete,
  onRequestPermanentRemove,
  onRestoreClip,
}: MatchLibraryProps) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [scrollViewport, setScrollViewport] = useState(DEFAULT_SCROLL_VIEWPORT);
  const fallbackScrollElementRef = useRef<HTMLDivElement>(null);
  const effectiveScrollElementRef = scrollElementRef ?? fallbackScrollElementRef;
  const tagById = useMemo(() => new Map(tags.map((tag) => [tag.id, tag])), [tags]);
  const prefersReducedMotion = Boolean(useReducedMotion());
  const sharedMotionProfile = useMemo(
    () => motionProfile(prefersReducedMotion),
    [prefersReducedMotion],
  );
  const groupLayoutSignature = useMemo(
    () => matchGroups
      .map((group) => `${matchGroupKey(group)}:${group.clips.length}`)
      .join("|"),
    [matchGroups],
  );
  const rowVirtualizer = useVirtualizer({
    count: matchGroups.length,
    getScrollElement: () => effectiveScrollElementRef.current,
    getItemKey: (index) => matchGroupKey(matchGroups[index]),
    estimateSize: (index) => estimateMatchGroupHeight(
      matchGroups[index],
      viewMode,
      estimatedClipContentWidth(scrollViewport.width, scrollViewport.isCompact),
      scrollViewport.width,
      scrollViewport.isCompact,
    ),
    measureElement: (element) => {
      const measuredHeight = element.getBoundingClientRect().height;
      const index = Number(element.getAttribute("data-index"));
      return measuredHeight > 0
        ? measuredHeight
        : estimateMatchGroupHeight(
            matchGroups[index],
            viewMode,
            estimatedClipContentWidth(scrollViewport.width, scrollViewport.isCompact),
            scrollViewport.width,
            scrollViewport.isCompact,
          );
    },
    observeElementRect: (instance, callback) => {
      const notify = () => {
        const rect = instance.scrollElement?.getBoundingClientRect();
        const width = rect?.width || DEFAULT_SCROLL_VIEWPORT.width;
        const height = rect?.height || DEFAULT_SCROLL_VIEWPORT.height;
        const isCompact = compactGridMatches();
        setScrollViewport((current) => (
          current.width === width && current.height === height && current.isCompact === isCompact
            ? current
            : { width, height, isCompact }
        ));
        callback({
          width,
          height,
        });
      };
      notify();
      const element = instance.scrollElement;
      if (!element) return undefined;
      const observer = new ResizeObserver(notify);
      observer.observe(element);
      window.addEventListener("resize", notify);
      return () => {
        observer.disconnect();
        window.removeEventListener("resize", notify);
      };
    },
    overscan: 3,
    initialRect: {
      width: DEFAULT_SCROLL_VIEWPORT.width,
      height: DEFAULT_SCROLL_VIEWPORT.height,
    },
  });
  const virtualRows = rowVirtualizer.getVirtualItems();
  const loadedClipCount = useMemo(
    () => matchGroups.reduce((total, group) => total + group.clips.length, 0),
    [matchGroups],
  );

  useEffect(() => {
    rowVirtualizer.scrollToOffset(0);
  }, [listGeneration]);

  useEffect(() => {
    rowVirtualizer.measure();
  }, [
    collapsed,
    groupLayoutSignature,
    rowVirtualizer,
    scrollViewport.isCompact,
    scrollViewport.width,
    viewMode,
  ]);

  useEffect(() => {
    const scrollElement = effectiveScrollElementRef.current;
    if (!scrollElement) return;
    const handleScroll = () => {
      if (!hasMore || isLoadingMore || loadMoreError || scrollElement.scrollTop <= 0) return;
      const remaining = scrollElement.scrollHeight - scrollElement.scrollTop - scrollElement.clientHeight;
      if (remaining < 720) onLoadMore();
    };
    scrollElement.addEventListener("scroll", handleScroll, { passive: true });
    return () => scrollElement.removeEventListener("scroll", handleScroll);
  }, [effectiveScrollElementRef, hasMore, isLoadingMore, loadMoreError, onLoadMore]);

  if (isLoading) {
    return (
      <section aria-label="正在加载对局素材" className="match-library-loading" role="status">
        {Array.from({ length: 3 }, (_, index) => <span key={index} />)}
      </section>
    );
  }

  if (errorMessage) {
    return <LibraryState title="素材加载失败" detail={errorMessage} action="重试加载" onAction={onRetryLoad} />;
  }

  if (matchGroups.length === 0 && totalClipCount === 0) {
    if (isTrashMode) {
      return (
        <LibraryState
          icon={<Trash aria-hidden="true" weight="duotone" />}
          title="回收站里没有视频"
          detail="移入回收站的视频会显示在这里。"
        />
      );
    }
    return <LibraryState title="还没有本地高光" detail="先添加录像目录并完成扫描，系统会按对局自动整理素材。" action="前往扫描" onAction={onOpenScan} />;
  }

  if (matchGroups.length === 0) {
    return <LibraryState title="没有符合条件的对局" detail={activeFilterLabels.join(" · ") || "请调整筛选条件"} action="清除筛选" onAction={onClearFilters} />;
  }

  return (
    <section className={`match-library match-library--${viewMode}`} aria-label="按对局分组的素材">
      <div
        className="match-library-virtualizer"
        data-testid="match-library-virtualizer"
        style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
      >
        {virtualRows.map((virtualRow) => {
          const matchGroup = matchGroups[virtualRow.index];
          const isCollapsed = collapsed[matchGroupKey(matchGroup)] ?? false;
          const tone = resultTone(matchGroup.resultLabel);
          return (
            <div
              className="match-library-virtual-row"
              data-index={virtualRow.index}
              data-virtual-row="true"
              key={virtualRow.key}
              ref={rowVirtualizer.measureElement}
              style={{ transform: `translateY(${virtualRow.start}px)` }}
            >
              <article className="match-board">
                <button
                  aria-expanded={!isCollapsed}
                  className={`match-board-header match-board-header--${tone}`}
                  type="button"
                  onClick={() => setCollapsed((current) => ({
                    ...current,
                    [matchGroupKey(matchGroup)]: !isCollapsed,
                  }))}
                >
                  <AgentPortrait name={matchGroup.agentName} />
                  <strong className="match-board-account">{matchGroup.accountDisplayName || "未知账号"}</strong>
                  <span className="match-board-date">{formatDateTime(matchGroup.latestModifiedAt)}</span>
                  <span className="match-board-agent-name">{matchGroup.agentName || "未知英雄"}</span>
                  <span className="match-board-mode">{matchGroup.gameMode || "未知模式"}</span>
                  <em className={`match-board-result match-board-result--${tone}`}>
                    {matchGroup.resultLabel}
                  </em>
                  <b className="match-board-score">{matchGroup.scoreline || "--/--"}</b>
                  <small className="match-board-kda-label">KDA</small>
                  <b className="match-board-kda">{matchGroup.kda || "--"}</b>
                  <span className="match-board-map">
                    <small>地图</small>
                    <strong>{matchGroup.mapName || "未知地图"}</strong>
                  </span>
                  <MapSlice name={matchGroup.mapName} />
                  <span className="match-board-count">{matchGroup.clips.length} 条片段</span>
                  <CaretUp className={isCollapsed ? "match-board-caret match-board-caret--collapsed" : "match-board-caret"} weight="bold" />
                </button>

                {!isCollapsed ? matchGroup.clips.length > EAGER_CLIP_LIMIT ? (
                  <VirtualizedMatchClips
                    clips={matchGroup.clips}
                    groupKey={matchGroupKey(matchGroup)}
                    isTrashMode={isTrashMode}
                    removableFromIndexIds={removableFromIndexIds}
                    motionProfile={sharedMotionProfile}
                    positionSignal={virtualRow.start}
                    scrollElementRef={effectiveScrollElementRef}
                    selectedClipId={selectedClipId}
                    selectedClipIds={selectedClipIds}
                    tagById={tagById}
                    viewMode={viewMode}
                    onSelectClip={onSelectClip}
                    onToggleFavorite={onToggleFavorite}
                    onCopyPath={onCopyPath}
                    onOpenOriginal={onOpenOriginal}
                    onSelectionGesture={onSelectionGesture}
                    onRequestTrash={onRequestTrash}
                    onRequestPermanentDelete={onRequestPermanentDelete}
                    onRequestPermanentRemove={onRequestPermanentRemove}
                    onRestoreClip={onRestoreClip}
                  />
                ) : (
                  <div className="match-board-clips">
                    {matchGroup.clips.map((clip, index) => (
                      <MatchClipCard
                        clip={clip}
                        index={index}
                        isActive={clip.id === selectedClipId}
                        isSelected={selectedClipIds.has(clip.id)}
                        isTrashMode={isTrashMode}
                        canRemoveFromIndex={removableFromIndexIds.has(clip.id)}
                        key={clip.id}
                        motionProfile={sharedMotionProfile}
                        tagById={tagById}
                        viewMode={viewMode}
                        onSelectClip={onSelectClip}
                        onToggleFavorite={onToggleFavorite}
                        onCopyPath={onCopyPath}
                        onOpenOriginal={onOpenOriginal}
                        onSelectionGesture={onSelectionGesture}
                        onRequestTrash={onRequestTrash}
                        onRequestPermanentDelete={onRequestPermanentDelete}
                        onRequestPermanentRemove={onRequestPermanentRemove}
                        onRestoreClip={onRestoreClip}
                      />
                    ))}
                  </div>
                ) : null}
              </article>
            </div>
          );
        })}
      </div>
      <footer className="match-library-pagination" aria-live="polite">
        {loadMoreError ? (
          <>
            <span>更多素材加载失败：{loadMoreError}</span>
            <button type="button" onClick={onRetryLoadMore}>重试加载更多</button>
          </>
        ) : hasMore ? (
          <button disabled={isLoadingMore} type="button" onClick={onLoadMore}>
            {isLoadingMore ? "正在加载更多…" : `加载更多（${loadedClipCount} / ${totalClipCount}）`}
          </button>
        ) : (
          <span>已加载全部 {totalClipCount} 条素材</span>
        )}
      </footer>
    </section>
  );
});

type MatchClipCardProps = {
  clip: ClipSummary;
  index: number;
  isActive: boolean;
  isSelected: boolean;
  isTrashMode: boolean;
  canRemoveFromIndex: boolean;
  motionProfile: MotionProfile;
  tagById: ReadonlyMap<string, Tag>;
  viewMode: LibraryViewMode;
  onSelectClip: (clipId: string, trigger: HTMLElement) => void;
  onToggleFavorite: (clipId: string) => void;
  onCopyPath: (clipId: string) => void;
  onOpenOriginal: (clipId: string) => void;
  onSelectionGesture: (clipId: string, gesture: ClipSelectionGesture) => void;
  onRequestTrash: (clipId: string) => void;
  onRequestPermanentDelete: (clipId: string) => void;
  onRequestPermanentRemove: (clipId: string) => void;
  onRestoreClip: (clipId: string) => void;
};

const MatchClipCard = memo(function MatchClipCard({
  clip,
  index,
  isActive,
  isSelected,
  isTrashMode,
  canRemoveFromIndex,
  motionProfile: profile,
  tagById,
  viewMode,
  onSelectClip,
  onToggleFavorite,
  onCopyPath,
  onOpenOriginal,
  onSelectionGesture,
  onRequestTrash,
  onRequestPermanentDelete,
  onRequestPermanentRemove,
  onRestoreClip,
}: MatchClipCardProps) {
  const title = highlightTitle(clip);
  const visibleTags = clip.tags.slice(0, 2);
  const scoreText = formatOfficialVideoScore(clip);
  const cardRef = useRef<HTMLElement | null>(null);

  const openPreview = () => {
    if (cardRef.current) onSelectClip(clip.id, cardRef.current);
  };

  return (
    <UiContextMenu>
      <UiContextMenuTrigger asChild>
        <m.article
          animate={{ opacity: 1, y: 0 }}
          className={cn(
            "match-clip-card",
            isActive && "match-clip-card--active",
            isSelected && "match-clip-card--selected",
          )}
          data-clip-id={clip.id}
          data-clip-index={index}
          data-view-mode={viewMode}
          initial={{ opacity: 0, y: profile.enterY }}
          ref={cardRef}
          transition={{ duration: profile.duration, delay: Math.min(index, 8) * profile.stagger }}
          whileHover={{ y: profile.hoverY }}
        >
          <button
            aria-label={`预览${title}`}
            className="match-clip-preview"
            type="button"
            onClick={(event) => {
              if (event.ctrlKey || event.metaKey || event.shiftKey) {
                onSelectionGesture(clip.id, {
                  additive: event.ctrlKey || event.metaKey,
                  range: event.shiftKey,
                });
                return;
              }
              onSelectClip(clip.id, event.currentTarget);
            }}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key === " ") {
                event.preventDefault();
                onSelectionGesture(clip.id, { additive: true, range: false });
              }
            }}
          />
          <span className="match-clip-select">
            <UiCheckbox
              aria-label={isSelected ? `取消选择${title}` : `选择${title}`}
              checked={isSelected}
              onCheckedChange={() => onSelectionGesture(clip.id, { additive: true, range: false })}
            />
          </span>
          <div className={`match-clip-thumb clip-thumb--${clip.thumbnailTone}`}>
            <ThumbnailImage
              alt=""
              decoding="async"
              fallback={<div className="clip-thumb-fallback" aria-hidden="true"><span /><span /></div>}
              loading="lazy"
              src={clip.thumbnailUrl}
            />
            <span className="match-clip-play"><Play weight="fill" /></span>
            <strong>{title}</strong>
            <em>{formatDuration(clip.durationMs)}</em>
          </div>
          <div className="match-clip-copy">
            <strong>{title}</strong>
            {scoreText ? (
              <span
                className={cn(
                  "match-clip-score",
                  clip.roundScore == null && "match-clip-score--unavailable",
                )}
              >
                {scoreText}
              </span>
            ) : null}
            <small>{formatDuration(clip.durationMs)} · {formatBytes(clip.sizeBytes)}</small>
            {visibleTags.length > 0 ? (
              <div>
                {visibleTags.map((tagId) => (
                  <span className={`tag tag--${tagById.get(tagId)?.color ?? "blue"}`} key={tagId}>
                    {tagById.get(tagId)?.label ?? tagId}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
          <button
            aria-label={clip.isFavorite ? "取消收藏" : "收藏"}
            aria-pressed={clip.isFavorite}
            className={clip.isFavorite ? "match-clip-favorite match-clip-favorite--active" : "match-clip-favorite"}
            type="button"
            onClick={() => onToggleFavorite(clip.id)}
          >
            <Heart weight={clip.isFavorite ? "fill" : "regular"} />
          </button>
        </m.article>
      </UiContextMenuTrigger>

      <UiContextMenuContent aria-label={`${title} 素材操作`}>
        <UiContextMenuLabel>{title}</UiContextMenuLabel>
        <UiContextMenuItem onSelect={openPreview}>
          <Play weight="fill" />
          <span>预览素材</span>
        </UiContextMenuItem>
        <UiContextMenuItem onSelect={() => onToggleFavorite(clip.id)}>
          <Heart weight={clip.isFavorite ? "fill" : "regular"} />
          <span>{clip.isFavorite ? "取消收藏" : "加入收藏"}</span>
        </UiContextMenuItem>
        <UiContextMenuItem onSelect={() => onSelectionGesture(clip.id, { additive: true, range: false })}>
          <CheckSquare weight={isSelected ? "fill" : "regular"} />
          <span>{isSelected ? "取消选择" : "选择素材"}</span>
        </UiContextMenuItem>
        <UiContextMenuSeparator />
        <UiContextMenuItem onSelect={() => onOpenOriginal(clip.id)}>
          <FolderOpen weight="bold" />
          <span>在文件夹中显示</span>
        </UiContextMenuItem>
        <UiContextMenuItem onSelect={() => onCopyPath(clip.id)}>
          <Copy weight="bold" />
          <span>复制文件路径</span>
        </UiContextMenuItem>
        <UiContextMenuSeparator />
        {isTrashMode ? (
          <>
            <UiContextMenuItem onSelect={() => onRestoreClip(clip.id)}>
              <ArrowCounterClockwise weight="bold" />
              <span>恢复素材</span>
            </UiContextMenuItem>
            <UiContextMenuItem className="ui-context-menu-item--danger" onSelect={() => onRequestPermanentRemove(clip.id)}>
              <Database weight="bold" />
              <span>仅从索引移除</span>
            </UiContextMenuItem>
            <UiContextMenuItem className="ui-context-menu-item--danger ui-context-menu-item--danger-strong" onSelect={() => onRequestPermanentDelete(clip.id)}>
              <Trash weight="fill" />
              <span>永久删除本地视频</span>
            </UiContextMenuItem>
          </>
        ) : (
          <>
            {canRemoveFromIndex ? (
              <UiContextMenuItem className="ui-context-menu-item--danger" onSelect={() => onRequestPermanentRemove(clip.id)}>
                <Database weight="bold" />
                <span>仅移除失联索引</span>
              </UiContextMenuItem>
            ) : null}
            <UiContextMenuItem className="ui-context-menu-item--danger" onSelect={() => onRequestTrash(clip.id)}>
              <Trash weight="bold" />
              <span>移入回收站</span>
            </UiContextMenuItem>
          </>
        )}
      </UiContextMenuContent>
    </UiContextMenu>
  );
});

type VirtualizedMatchClipsProps = {
  clips: ClipSummary[];
  groupKey: string;
  isTrashMode: boolean;
  removableFromIndexIds: ReadonlySet<string>;
  motionProfile: MotionProfile;
  positionSignal: number;
  scrollElementRef: RefObject<HTMLDivElement | null>;
  selectedClipId: string;
  selectedClipIds: ReadonlySet<string>;
  tagById: ReadonlyMap<string, Tag>;
  viewMode: LibraryViewMode;
  onSelectClip: (clipId: string, trigger: HTMLElement) => void;
  onToggleFavorite: (clipId: string) => void;
  onCopyPath: (clipId: string) => void;
  onOpenOriginal: (clipId: string) => void;
  onSelectionGesture: (clipId: string, gesture: ClipSelectionGesture) => void;
  onRequestTrash: (clipId: string) => void;
  onRequestPermanentDelete: (clipId: string) => void;
  onRequestPermanentRemove: (clipId: string) => void;
  onRestoreClip: (clipId: string) => void;
};

type ClipGridMetrics = {
  width: number;
  scrollMargin: number;
  viewportWidth: number;
  viewportHeight: number;
  isCompact: boolean;
};

const VirtualizedMatchClips = memo(function VirtualizedMatchClips({
  clips,
  groupKey,
  isTrashMode,
  removableFromIndexIds,
  motionProfile: profile,
  positionSignal,
  scrollElementRef,
  selectedClipId,
  selectedClipIds,
  tagById,
  viewMode,
  onSelectClip,
  onToggleFavorite,
  onCopyPath,
  onOpenOriginal,
  onSelectionGesture,
  onRequestTrash,
  onRequestPermanentDelete,
  onRequestPermanentRemove,
  onRestoreClip,
}: VirtualizedMatchClipsProps) {
  const contentRef = useRef<HTMLDivElement>(null);
  const [metrics, setMetrics] = useState<ClipGridMetrics>(() => ({
    width: estimatedClipContentWidth(DEFAULT_SCROLL_VIEWPORT.width, false),
    scrollMargin: 0,
    viewportWidth: DEFAULT_SCROLL_VIEWPORT.width,
    viewportHeight: DEFAULT_SCROLL_VIEWPORT.height,
    isCompact: false,
  }));
  const columnCount = clipColumnCount(metrics.width, viewMode, metrics.isCompact);
  const gap = metrics.isCompact ? COMPACT_GRID_GAP : GRID_GAP;
  const rowCount = Math.ceil(clips.length / columnCount);
  const estimateRowHeight = useCallback(
    (rowIndex: number) => {
      const startIndex = rowIndex * columnCount;
      const rowHasTags = clips
        .slice(startIndex, startIndex + columnCount)
        .some((clip) => clip.tags.length > 0);
      return estimateClipRowHeight(
        metrics.width,
        columnCount,
        viewMode,
        metrics.viewportWidth,
        metrics.isCompact,
        rowHasTags,
      );
    },
    [clips, columnCount, metrics.isCompact, metrics.viewportWidth, metrics.width, viewMode],
  );
  const getRowKey = useCallback((rowIndex: number) => {
    const startIndex = rowIndex * columnCount;
    const endIndex = Math.min(clips.length, startIndex + columnCount) - 1;
    return [
      groupKey,
      viewMode,
      columnCount,
      clips[startIndex]?.id ?? startIndex,
      clips[endIndex]?.id ?? endIndex,
    ].join(":");
  }, [clips, columnCount, groupKey, viewMode]);
  const measureRow = useCallback((element: Element) => {
    const measuredHeight = element.getBoundingClientRect().height;
    const rowIndex = Number(element.getAttribute("data-index"));
    return measuredHeight > 0
      ? measuredHeight
      : estimateRowHeight(Number.isFinite(rowIndex) ? rowIndex : 0);
  }, [estimateRowHeight]);
  const clipRowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollElementRef.current,
    getItemKey: getRowKey,
    estimateSize: estimateRowHeight,
    measureElement: measureRow,
    scrollMargin: metrics.scrollMargin,
    gap,
    overscan: 1,
    initialRect: {
      width: metrics.viewportWidth,
      height: metrics.viewportHeight,
    },
  });
  const virtualClipRows = clipRowVirtualizer.getVirtualItems();

  useLayoutEffect(() => {
    const contentElement = contentRef.current;
    if (!contentElement) return;

    const updateMetrics = () => {
      const contentRect = contentElement.getBoundingClientRect();
      const scrollElement = scrollElementRef.current;
      const scrollRect = scrollElement?.getBoundingClientRect();
      const isCompact = compactGridMatches();
      const viewportWidth = scrollRect?.width || DEFAULT_SCROLL_VIEWPORT.width;
      const viewportHeight = scrollRect?.height || DEFAULT_SCROLL_VIEWPORT.height;
      const width = contentRect.width || estimatedClipContentWidth(viewportWidth, isCompact);
      const scrollMargin = scrollElement && scrollRect
        ? contentRect.top - scrollRect.top + scrollElement.scrollTop
        : 0;

      setMetrics((current) => (
        nearlyEqual(current.width, width)
        && nearlyEqual(current.scrollMargin, scrollMargin)
        && current.viewportWidth === viewportWidth
        && current.viewportHeight === viewportHeight
        && current.isCompact === isCompact
          ? current
          : { width, scrollMargin, viewportWidth, viewportHeight, isCompact }
      ));
    };

    updateMetrics();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateMetrics);
    observer?.observe(contentElement);
    if (scrollElementRef.current) observer?.observe(scrollElementRef.current);
    window.addEventListener("resize", updateMetrics);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", updateMetrics);
    };
  }, [positionSignal, scrollElementRef, viewMode]);

  useEffect(() => {
    clipRowVirtualizer.measure();
  }, [clipRowVirtualizer, columnCount, metrics.width, viewMode]);

  return (
    <div
      className="match-board-clips"
      data-column-count={columnCount}
      data-testid="match-clip-virtualizer"
      data-total-count={clips.length}
      style={{ display: "block" }}
    >
      <div
        data-testid="match-clip-virtual-content"
        ref={contentRef}
        style={{
          height: `${clipRowVirtualizer.getTotalSize()}px`,
          position: "relative",
          width: "100%",
        }}
      >
        {virtualClipRows.map((virtualRow) => {
          const startIndex = virtualRow.index * columnCount;
          const rowClips = clips.slice(startIndex, startIndex + columnCount);
          return (
            <div
              data-clip-virtual-row="true"
              data-index={virtualRow.index}
              data-row-start-index={startIndex}
              key={virtualRow.key}
              ref={clipRowVirtualizer.measureElement}
              style={{
                display: "grid",
                gap: `${gap}px`,
                gridTemplateColumns: `repeat(${columnCount}, minmax(0, 1fr))`,
                left: 0,
                position: "absolute",
                top: 0,
                transform: `translateY(${virtualRow.start - metrics.scrollMargin}px)`,
                width: "100%",
              }}
            >
              {rowClips.map((clip, rowOffset) => {
                const index = startIndex + rowOffset;
                return (
                  <MatchClipCard
                    clip={clip}
                    index={index}
                    isActive={clip.id === selectedClipId}
                    isSelected={selectedClipIds.has(clip.id)}
                    isTrashMode={isTrashMode}
                    canRemoveFromIndex={removableFromIndexIds.has(clip.id)}
                    key={clip.id}
                    motionProfile={profile}
                    tagById={tagById}
                    viewMode={viewMode}
                    onSelectClip={onSelectClip}
                    onToggleFavorite={onToggleFavorite}
                    onCopyPath={onCopyPath}
                    onOpenOriginal={onOpenOriginal}
                    onSelectionGesture={onSelectionGesture}
                    onRequestTrash={onRequestTrash}
                    onRequestPermanentDelete={onRequestPermanentDelete}
                    onRequestPermanentRemove={onRequestPermanentRemove}
                    onRestoreClip={onRestoreClip}
                  />
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
});

function AgentPortrait({ name }: { name: string }) {
  const url = valorantAgentDisplayIconUrl(name);
  const [failed, setFailed] = useState(false);

  useEffect(() => setFailed(false), [url]);

  return (
    <span
      className={url && !failed ? "match-board-agent" : "match-board-agent match-board-agent--fallback"}
      title={name || "未知英雄"}
    >
      {url && !failed ? (
        <img
          alt=""
          decoding="async"
          src={url}
          onError={() => setFailed(true)}
        />
      ) : agentInitial(name)}
    </span>
  );
}

function MapSlice({ name }: { name: string }) {
  const url = valorantMapListViewIconUrl(name);
  const [failed, setFailed] = useState(false);

  useEffect(() => setFailed(false), [url]);

  return (
    <span className="match-board-map-art" aria-hidden="true">
      {url && !failed ? (
        <img
          alt=""
          decoding="async"
          loading="lazy"
          src={url}
          onError={() => setFailed(true)}
        />
      ) : null}
    </span>
  );
}

function LibraryState({
  title,
  detail,
  action,
  icon,
  onAction,
}: {
  title: string;
  detail: string;
  action?: string;
  icon?: ReactNode;
  onAction?: () => void;
}) {
  return (
    <section className="match-library-state">
      {icon ? <span className="match-library-state-icon">{icon}</span> : null}
      <h2>{title}</h2>
      <p>{detail}</p>
      {action && onAction ? <button type="button" onClick={onAction}>{action}</button> : null}
    </section>
  );
}

function matchGroupKey(group: ClipMatchGroup): string {
  return `${group.accountId}:${group.id}`;
}

function estimateMatchGroupHeight(
  group: ClipMatchGroup,
  viewMode: LibraryViewMode,
  clipContentWidth: number,
  viewportWidth: number,
  isCompact: boolean,
): number {
  const columnCount = clipColumnCount(clipContentWidth, viewMode, isCompact);
  const rowCount = Math.ceil(group.clips.length / columnCount);
  const gap = isCompact ? COMPACT_GRID_GAP : GRID_GAP;
  const clipRowsHeight = Array.from({ length: rowCount }, (_, rowIndex) => {
    const startIndex = rowIndex * columnCount;
    const rowHasTags = group.clips
      .slice(startIndex, startIndex + columnCount)
      .some((clip) => clip.tags.length > 0);
    return estimateClipRowHeight(
      clipContentWidth,
      columnCount,
      viewMode,
      viewportWidth,
      isCompact,
      rowHasTags,
    );
  }).reduce((total, rowHeight) => total + rowHeight, 0)
    + Math.max(0, rowCount - 1) * gap;
  const clipPadding = isCompact ? 20 : 24;
  return MATCH_HEADER_HEIGHT + clipPadding + clipRowsHeight + MATCH_ROW_BOTTOM_GAP + 2;
}

function estimatedClipContentWidth(viewportWidth: number, isCompact: boolean): number {
  const workspacePadding = isCompact ? 20 : 28;
  const clipGridPadding = isCompact ? 20 : 24;
  return Math.max(1, viewportWidth - workspacePadding - clipGridPadding - 2);
}

function clipColumnCount(
  contentWidth: number,
  viewMode: LibraryViewMode,
  isCompact: boolean,
): number {
  if (viewMode === "list" || isCompact) return 1;
  return Math.max(1, Math.floor((contentWidth + GRID_GAP) / (GRID_MIN_CARD_WIDTH + GRID_GAP)));
}

function estimateClipRowHeight(
  contentWidth: number,
  columnCount: number,
  viewMode: LibraryViewMode,
  viewportWidth: number,
  isCompact: boolean,
  hasTags: boolean,
): number {
  if (viewMode === "list") return 88;
  const footerHeight = hasTags
    ? GRID_CARD_TAGGED_FOOTER_HEIGHT
    : GRID_CARD_FOOTER_HEIGHT;
  if (isCompact) {
    const thumbnailHeight = Math.min(220, Math.max(168, viewportWidth * 0.38));
    return thumbnailHeight + footerHeight;
  }
  const cardWidth = Math.max(
    GRID_MIN_CARD_WIDTH,
    (contentWidth - (columnCount - 1) * GRID_GAP) / columnCount,
  );
  return cardWidth * 9 / 16 + footerHeight;
}

function compactGridMatches(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia?.("(max-width: 680px)").matches ?? window.innerWidth <= 680;
}

function nearlyEqual(left: number, right: number): boolean {
  return Math.abs(left - right) < 0.5;
}

function resultTone(result: string): string {
  if (result === "胜利") return "win";
  if (result === "失败") return "loss";
  return "unknown";
}

function highlightTitle(clip: ClipSummary): string {
  return displayHighlightTitle(clip);
}

function formatDuration(durationMs: number | null): string {
  const seconds = Math.max(0, Math.round((durationMs ?? 0) / 1000));
  return `${Math.floor(seconds / 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
}

function formatOfficialVideoScore(clip: ClipSummary): string | null {
  if (clip.roundScore != null) return `${clip.roundScore} 评分`;
  return expectsOfficialRoundScore(clip) ? "官方未同步" : null;
}
