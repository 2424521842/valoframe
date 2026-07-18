import { useCallback, useEffect, useRef, useState } from "react";
import {
  commandErrorMessage,
  getClipDetail,
  mergeClipsWithSources,
} from "../api/backend";
import type { ClipDetail, SourceDir, ThumbnailStatus } from "../types";

export type ClipThumbnailPatch = {
  thumbnailStatus: ThumbnailStatus;
  thumbnailRevision: string | null;
  thumbnailUrl: string | null;
};

export type ClipDetailState =
  | { status: "idle"; clip: null; error: null }
  | { status: "loading"; clip: null; error: null }
  | { status: "ready"; clip: ClipDetail; error: null }
  | { status: "not-found"; clip: null; error: string }
  | { status: "error"; clip: null; error: string };

export type ClipDetailController = {
  state: ClipDetailState;
  retry: () => Promise<boolean>;
  cancelPending: () => void;
  invalidate: () => void;
  getClip: (clipId: string) => ClipDetail | undefined;
  syncClip: (clip: ClipDetail) => void;
  patchThumbnail: (clipId: string, patch: ClipThumbnailPatch) => void;
  removeTag: (tagId: string) => void;
  removeClip: (clipId: string) => void;
};

export type UseClipDetailControllerOptions = {
  active: boolean;
  clipId: string;
  sourceDirs: readonly SourceDir[];
  cacheLimit?: number;
};

const EMPTY_DETAIL_STATE: ClipDetailState = {
  status: "idle",
  clip: null,
  error: null,
};
const DEFAULT_CACHE_LIMIT = 6;

type InFlightRequest = {
  clipId: string;
  token: number;
  promise: Promise<boolean>;
};

export function useClipDetailController({
  active,
  clipId,
  sourceDirs,
  cacheLimit = DEFAULT_CACHE_LIMIT,
}: UseClipDetailControllerOptions): ClipDetailController {
  const mountedRef = useRef(false);
  const activeRef = useRef(active);
  const clipIdRef = useRef(clipId);
  const sourceDirsRef = useRef(sourceDirs);
  const cacheLimitRef = useRef(normalizeCacheLimit(cacheLimit));
  const requestTokenRef = useRef(0);
  const inFlightRef = useRef<InFlightRequest | null>(null);
  const rawCacheRef = useRef(new Map<string, ClipDetail>());
  const [state, setState] = useState<ClipDetailState>(EMPTY_DETAIL_STATE);

  activeRef.current = active;
  clipIdRef.current = clipId;
  sourceDirsRef.current = sourceDirs;
  cacheLimitRef.current = normalizeCacheLimit(cacheLimit);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const hydrate = useCallback((rawClip: ClipDetail): ClipDetail => {
    const [hydrated] = mergeClipsWithSources([rawClip], sourceDirsRef.current);
    return hydrated;
  }, []);

  const cacheRawClip = useCallback((rawClip: ClipDetail) => {
    const cache = rawCacheRef.current;
    cache.delete(rawClip.id);
    cache.set(rawClip.id, rawClip);
    while (cache.size > cacheLimitRef.current) {
      const oldestId = cache.keys().next().value;
      if (typeof oldestId !== "string") break;
      cache.delete(oldestId);
    }
  }, []);

  const requestDetail = useCallback((
    requestedClipId: string,
    force = false,
  ): Promise<boolean> => {
    const pending = inFlightRef.current;
    if (!force && pending?.clipId === requestedClipId) {
      return pending.promise;
    }

    const requestToken = requestTokenRef.current + 1;
    requestTokenRef.current = requestToken;
    inFlightRef.current = null;

    const cached = force ? undefined : rawCacheRef.current.get(requestedClipId);
    if (cached) {
      cacheRawClip(cached);
      if (mountedRef.current) {
        setState({ status: "ready", clip: hydrate(cached), error: null });
      }
      return Promise.resolve(true);
    }

    if (mountedRef.current) {
      setState({ status: "loading", clip: null, error: null });
    }

    const requestPromise = Promise.resolve()
      .then(() => getClipDetail(requestedClipId))
      .then((rawClip) => {
        if (!isCurrentRequest(
          mountedRef.current,
          requestTokenRef.current,
          requestToken,
          activeRef.current,
          clipIdRef.current,
          requestedClipId,
        )) {
          return false;
        }
        cacheRawClip(rawClip);
        setState({ status: "ready", clip: hydrate(rawClip), error: null });
        return true;
      })
      .catch((requestError: unknown) => {
        if (!isCurrentRequest(
          mountedRef.current,
          requestTokenRef.current,
          requestToken,
          activeRef.current,
          clipIdRef.current,
          requestedClipId,
        )) {
          return false;
        }
        const message = commandErrorMessage(requestError);
        setState(
          commandErrorCode(requestError) === "clip-not-found"
            ? { status: "not-found", clip: null, error: message }
            : { status: "error", clip: null, error: message },
        );
        return false;
      })
      .finally(() => {
        if (inFlightRef.current?.token === requestToken) {
          inFlightRef.current = null;
        }
      });

    inFlightRef.current = {
      clipId: requestedClipId,
      token: requestToken,
      promise: requestPromise,
    };
    return requestPromise;
  }, [cacheRawClip, hydrate]);

  useEffect(() => {
    if (!active || !clipId) {
      requestTokenRef.current += 1;
      inFlightRef.current = null;
      setState(EMPTY_DETAIL_STATE);
      return;
    }
    void requestDetail(clipId);
  }, [active, clipId, requestDetail]);

  useEffect(() => {
    setState((current) => {
      if (current.status !== "ready") return current;
      const rawClip = rawCacheRef.current.get(current.clip.id);
      return rawClip
        ? { status: "ready", clip: hydrate(rawClip), error: null }
        : current;
    });
  }, [hydrate, sourceDirs]);

  const retry = useCallback((): Promise<boolean> => {
    const requestedClipId = clipIdRef.current;
    if (!activeRef.current || !requestedClipId) {
      return Promise.resolve(false);
    }
    rawCacheRef.current.delete(requestedClipId);
    return requestDetail(requestedClipId, true);
  }, [requestDetail]);

  const cancelPending = useCallback(() => {
    requestTokenRef.current += 1;
    inFlightRef.current = null;
    if (mountedRef.current) {
      setState(EMPTY_DETAIL_STATE);
    }
  }, []);

  const invalidate = useCallback(() => {
    rawCacheRef.current.clear();
    cancelPending();
  }, [cancelPending]);

  const getClip = useCallback((cachedClipId: string): ClipDetail | undefined => {
    const rawClip = rawCacheRef.current.get(cachedClipId);
    return rawClip ? hydrate(rawClip) : undefined;
  }, [hydrate]);

  const syncClip = useCallback((rawClip: ClipDetail) => {
    const shouldCache = rawCacheRef.current.has(rawClip.id) || (
      activeRef.current && clipIdRef.current === rawClip.id
    );
    if (shouldCache) {
      cacheRawClip(rawClip);
    }
    if (!mountedRef.current) return;
    setState((current) =>
      current.status === "ready" && current.clip.id === rawClip.id
        ? { status: "ready", clip: hydrate(rawClip), error: null }
        : current,
    );
  }, [cacheRawClip, hydrate]);

  const patchThumbnail = useCallback((patchedClipId: string, patch: ClipThumbnailPatch) => {
    const cached = rawCacheRef.current.get(patchedClipId);
    if (cached) {
      rawCacheRef.current.set(patchedClipId, { ...cached, ...patch });
    }
    if (!mountedRef.current) return;
    setState((current) => {
      if (current.status !== "ready" || current.clip.id !== patchedClipId) {
        return current;
      }
      const rawClip = rawCacheRef.current.get(patchedClipId) ?? {
        ...current.clip,
        ...patch,
      };
      return { status: "ready", clip: hydrate(rawClip), error: null };
    });
  }, [hydrate]);

  const removeTag = useCallback((tagId: string) => {
    for (const [cachedClipId, rawClip] of rawCacheRef.current) {
      if (!rawClip.tags.includes(tagId)) continue;
      rawCacheRef.current.set(cachedClipId, {
        ...rawClip,
        tags: rawClip.tags.filter((candidate) => candidate !== tagId),
      });
    }
    if (!mountedRef.current) return;
    setState((current) => {
      if (current.status !== "ready" || !current.clip.tags.includes(tagId)) {
        return current;
      }
      const rawClip = rawCacheRef.current.get(current.clip.id);
      return rawClip
        ? { status: "ready", clip: hydrate(rawClip), error: null }
        : current;
    });
  }, [hydrate]);

  const removeClip = useCallback((removedClipId: string) => {
    rawCacheRef.current.delete(removedClipId);
    if (clipIdRef.current !== removedClipId) return;
    requestTokenRef.current += 1;
    inFlightRef.current = null;
    if (mountedRef.current) {
      setState(EMPTY_DETAIL_STATE);
    }
  }, []);

  return {
    state,
    retry,
    cancelPending,
    invalidate,
    getClip,
    syncClip,
    patchThumbnail,
    removeTag,
    removeClip,
  };
}

function isCurrentRequest(
  mounted: boolean,
  currentToken: number,
  requestToken: number,
  active: boolean,
  currentClipId: string,
  requestedClipId: string,
): boolean {
  return mounted &&
    currentToken === requestToken &&
    active &&
    currentClipId === requestedClipId;
}

function normalizeCacheLimit(cacheLimit: number): number {
  return Number.isFinite(cacheLimit) ? Math.max(1, Math.floor(cacheLimit)) : DEFAULT_CACHE_LIMIT;
}

function commandErrorCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return null;
  }
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" ? code : null;
}
