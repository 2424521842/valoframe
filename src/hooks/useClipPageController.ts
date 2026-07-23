import { useCallback, useEffect, useRef, useState } from "react";
import { commandErrorMessage, listClipPage } from "../api/backend";
import {
  CLIP_PAGE_SIZE,
  CLIP_SELECT_ALL_PAGE_SIZE,
} from "../lib/clipListQuery";
import type { ClipListQuery, ClipSummary } from "../types";

export type LoadClipPageOptions = {
  preserveActivity?: boolean;
  preserveItems?: boolean;
};

export type UseClipPageControllerOptions = {
  query: ClipListQuery;
  queryKey: string;
  onActivityMessage?: (message: string) => void;
};

export type ClipPageController = {
  items: ClipSummary[];
  totalCount: number;
  hasMore: boolean;
  nextOffset: number | null;
  generation: number;
  isLoading: boolean;
  isLoadingMore: boolean;
  error: string | null;
  loadMoreError: string | null;
  reload: (options?: LoadClipPageOptions) => Promise<boolean>;
  loadMore: () => Promise<boolean>;
  loadAll: () => Promise<ClipSummary[] | null>;
  retry: () => Promise<boolean>;
  retryLoadMore: () => Promise<boolean>;
  getQuery: () => ClipListQuery;
  getItem: (clipId: string) => ClipSummary | undefined;
  mergeSummaries: (summaries: readonly ClipSummary[]) => void;
  removeSummaries: (clipIds: Iterable<string>) => void;
  updateItems: (
    updater: (current: readonly ClipSummary[]) => readonly ClipSummary[],
  ) => void;
};

export function useClipPageController({
  query,
  queryKey,
  onActivityMessage,
}: UseClipPageControllerOptions): ClipPageController {
  const mountedRef = useRef(false);
  const queryRef = useRef(query);
  const queryKeyRef = useRef(queryKey);
  const activityRef = useRef(onActivityMessage);
  const itemsRef = useRef<ClipSummary[]>([]);
  const generationRef = useRef(0);
  const requestedOffsetsRef = useRef(new Set<number>());
  const appliedQueryKeyRef = useRef("");
  const hasMoreRef = useRef(false);
  const nextOffsetRef = useRef<number | null>(null);
  const loadingInitialRef = useRef(false);
  const loadingMoreRef = useRef(false);

  queryRef.current = query;
  queryKeyRef.current = queryKey;
  activityRef.current = onActivityMessage;

  const [items, setItems] = useState<ClipSummary[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [generation, setGeneration] = useState(0);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadMoreError, setLoadMoreError] = useState<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const replaceItems = useCallback((nextItems: ClipSummary[]) => {
    itemsRef.current = nextItems;
    if (mountedRef.current) {
      setItems(nextItems);
    }
  }, []);

  const updateItems = useCallback((
    updater: (current: readonly ClipSummary[]) => readonly ClipSummary[],
  ) => {
    const nextItems = updater(itemsRef.current);
    if (nextItems === itemsRef.current) return;
    replaceItems([...nextItems]);
  }, [replaceItems]);

  const requestPage = useCallback(async (
    pageQuery: ClipListQuery,
    requestGeneration: number,
    mode: "initial" | "more",
    {
      preserveActivity = false,
      preserveItems = false,
    }: LoadClipPageOptions = {},
  ): Promise<boolean> => {
    const offset = pageQuery.offset ?? 0;
    if (requestedOffsetsRef.current.has(offset)) {
      return false;
    }
    requestedOffsetsRef.current.add(offset);
    const keepCurrentItemsVisible = mode === "initial"
      && preserveItems
      && itemsRef.current.length > 0;

    if (mode === "initial") {
      loadingInitialRef.current = true;
      if (mountedRef.current) {
        if (!keepCurrentItemsVisible) setIsLoading(true);
        setError(null);
      }
    } else {
      loadingMoreRef.current = true;
      if (mountedRef.current) {
        setIsLoadingMore(true);
        setLoadMoreError(null);
      }
    }

    try {
      const page = await listClipPage(pageQuery);
      if (!mountedRef.current || generationRef.current !== requestGeneration) {
        return false;
      }

      const nextItems = mode === "initial"
        ? page.items
        : mergeClipSummaryPages(itemsRef.current, page.items);
      replaceItems(nextItems);
      setTotalCount(page.totalCount);
      const pageHasMore = page.hasMore && page.nextOffset !== null;
      hasMoreRef.current = pageHasMore;
      nextOffsetRef.current = pageHasMore ? page.nextOffset : null;
      setHasMore(pageHasMore);
      setNextOffset(nextOffsetRef.current);

      if (!preserveActivity) {
        activityRef.current?.(
          pageHasMore
            ? `已加载 ${nextItems.length} / ${page.totalCount} 个素材`
            : `已加载全部 ${page.totalCount} 个素材`,
        );
      }
      return true;
    } catch (requestError) {
      if (!mountedRef.current || generationRef.current !== requestGeneration) {
        return false;
      }
      requestedOffsetsRef.current.delete(offset);
      const message = commandErrorMessage(requestError);
      if (mode === "initial") {
        setError(message);
        if (!preserveActivity) {
          activityRef.current?.("素材加载失败");
        }
      } else {
        setLoadMoreError(message);
        if (!preserveActivity) {
          activityRef.current?.("更多素材加载失败，可重试");
        }
      }
      return false;
    } finally {
      if (generationRef.current === requestGeneration) {
        if (mode === "initial") {
          loadingInitialRef.current = false;
          if (mountedRef.current) setIsLoading(false);
        } else {
          loadingMoreRef.current = false;
          if (mountedRef.current) setIsLoadingMore(false);
        }
      }
    }
  }, [replaceItems]);

  const reload = useCallback((
    options: LoadClipPageOptions = {},
  ): Promise<boolean> => {
    const keepCurrentItemsVisible = Boolean(
      options.preserveItems && itemsRef.current.length > 0,
    );
    const nextGeneration = generationRef.current + 1;
    generationRef.current = nextGeneration;
    requestedOffsetsRef.current = new Set();
    const firstPageQuery: ClipListQuery = {
      ...queryRef.current,
      offset: 0,
      limit: queryRef.current.limit ?? CLIP_PAGE_SIZE,
    };
    appliedQueryKeyRef.current = queryKeyRef.current;
    queryRef.current = firstPageQuery;
    loadingMoreRef.current = false;
    if (!keepCurrentItemsVisible) {
      hasMoreRef.current = false;
      nextOffsetRef.current = null;
      replaceItems([]);
    }
    if (mountedRef.current) {
      setGeneration(nextGeneration);
      if (!keepCurrentItemsVisible) {
        setTotalCount(0);
        setHasMore(false);
        setNextOffset(null);
      }
      setError(null);
      setLoadMoreError(null);
      setIsLoadingMore(false);
    }
    return requestPage(firstPageQuery, nextGeneration, "initial", options);
  }, [replaceItems, requestPage]);

  const loadMore = useCallback((): Promise<boolean> => {
    const offset = nextOffsetRef.current;
    if (
      !hasMoreRef.current ||
      offset === null ||
      loadingInitialRef.current ||
      loadingMoreRef.current
    ) {
      return Promise.resolve(false);
    }
    return requestPage(
      {
        ...queryRef.current,
        offset,
        limit: queryRef.current.limit ?? CLIP_PAGE_SIZE,
      },
      generationRef.current,
      "more",
    );
  }, [requestPage]);

  const loadAll = useCallback(async (): Promise<ClipSummary[] | null> => {
    const requestGeneration = generationRef.current;
    while (hasMoreRef.current) {
      const offset = nextOffsetRef.current;
      if (offset === null || loadingInitialRef.current || loadingMoreRef.current) {
        return null;
      }
      const loaded = await requestPage(
        {
          ...queryRef.current,
          offset,
          limit: CLIP_SELECT_ALL_PAGE_SIZE,
        },
        requestGeneration,
        "more",
      );
      if (!loaded || generationRef.current !== requestGeneration) {
        return null;
      }
    }
    return [...itemsRef.current];
  }, [requestPage]);

  useEffect(() => {
    if (appliedQueryKeyRef.current === queryKey) {
      return;
    }
    appliedQueryKeyRef.current = queryKey;
    void reload();
  }, [queryKey, reload]);

  const mergeSummaries = useCallback((summaries: readonly ClipSummary[]) => {
    updateItems((current) => mergeClipSummaryPages(current, summaries));
  }, [updateItems]);

  const removeSummaries = useCallback((clipIds: Iterable<string>) => {
    const removedIds = new Set(clipIds);
    if (removedIds.size === 0) return;
    updateItems((current) => current.filter((clip) => !removedIds.has(clip.id)));
  }, [updateItems]);

  const getItem = useCallback(
    (clipId: string) => itemsRef.current.find((clip) => clip.id === clipId),
    [],
  );
  const getQuery = useCallback(() => queryRef.current, []);

  return {
    items,
    totalCount,
    hasMore,
    nextOffset,
    generation,
    isLoading,
    isLoadingMore,
    error,
    loadMoreError,
    reload,
    loadMore,
    loadAll,
    retry: reload,
    retryLoadMore: loadMore,
    getQuery,
    getItem,
    mergeSummaries,
    removeSummaries,
    updateItems,
  };
}

export function mergeClipSummaryPages(
  current: readonly ClipSummary[],
  incoming: readonly ClipSummary[],
): ClipSummary[] {
  const incomingById = new Map(incoming.map((clip) => [clip.id, clip]));
  const merged = current.map((clip) => incomingById.get(clip.id) ?? clip);
  const seen = new Set(current.map((clip) => clip.id));
  for (const clip of incoming) {
    if (seen.has(clip.id)) continue;
    seen.add(clip.id);
    merged.push(clip);
  }
  return merged;
}
