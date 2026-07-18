import { useCallback, useEffect, useRef } from "react";
import {
  ensureClipThumbnails,
  listenToThumbnailProgress,
  retryClipThumbnails,
  THUMBNAIL_ENQUEUE_LIMIT,
} from "../api/backend";
import type {
  ClipSummary,
  ThumbnailEnqueueResult,
  ThumbnailProgress,
} from "../types";

export type UseThumbnailControllerOptions = {
  generation: number;
  clips: readonly ClipSummary[];
  onProgress: (progress: ThumbnailProgress) => void;
};

export type ThumbnailController = {
  retry: (clipIds: readonly string[]) => Promise<ThumbnailEnqueueResult>;
};

const EMPTY_ENQUEUE_RESULT: ThumbnailEnqueueResult = {
  requested: 0,
  queued: 0,
  alreadyQueued: 0,
  skipped: 0,
};

/**
 * Ensures thumbnails at page granularity. Virtual card mounts never issue
 * commands, so scrolling and virtualization cannot create an invoke storm.
 */
export function useThumbnailController({
  generation,
  clips,
  onProgress,
}: UseThumbnailControllerOptions): ThumbnailController {
  const ensuredRef = useRef({ generation, ids: new Set<string>() });
  const onProgressRef = useRef(onProgress);
  const readyRevisionRef = useRef(new Map<string, string>());
  onProgressRef.current = onProgress;

  useEffect(() => {
    if (ensuredRef.current.generation !== generation) {
      ensuredRef.current = { generation, ids: new Set<string>() };
    }

    const tracker = ensuredRef.current;
    const newIds: string[] = [];
    for (const clip of clips) {
      if (tracker.ids.has(clip.id)) continue;
      tracker.ids.add(clip.id);
      newIds.push(clip.id);
    }

    if (newIds.length === 0) return;
    void runInBatches(newIds, ensureClipThumbnails).catch(() => {
      // The list remains usable when generation is unavailable. Explicit retry
      // is exposed for a future status surface without creating an auto-loop.
    });
  }, [clips, generation]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listenToThumbnailProgress((progress) => {
      if (disposed) {
        return;
      }
      const status = progress.status.trim().toLowerCase();
      const normalizedProgress = status === progress.status
        ? progress
        : { ...progress, status };
      if (status !== "ready") {
        readyRevisionRef.current.delete(progress.clipId);
        onProgressRef.current(normalizedProgress);
        return;
      }
      const revisionKey = progress.revision ?? "";
      if (readyRevisionRef.current.get(progress.clipId) === revisionKey) {
        return;
      }
      readyRevisionRef.current.set(progress.clipId, revisionKey);
      onProgressRef.current(normalizedProgress);
    }).then((nextUnlisten) => {
      if (disposed) {
        nextUnlisten();
      } else {
        unlisten = nextUnlisten;
      }
    }).catch(() => {
      // Browser preview and unavailable event channels degrade to static covers.
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const retry = useCallback((clipIds: readonly string[]) =>
    runInBatches(uniqueIds(clipIds), retryClipThumbnails), []);

  return { retry };
}

async function runInBatches(
  clipIds: readonly string[],
  request: (batch: readonly string[]) => Promise<ThumbnailEnqueueResult>,
): Promise<ThumbnailEnqueueResult> {
  if (clipIds.length === 0) return { ...EMPTY_ENQUEUE_RESULT };

  const total = { ...EMPTY_ENQUEUE_RESULT };
  for (let index = 0; index < clipIds.length; index += THUMBNAIL_ENQUEUE_LIMIT) {
    const result = await request(clipIds.slice(index, index + THUMBNAIL_ENQUEUE_LIMIT));
    total.requested += result.requested;
    total.queued += result.queued;
    total.alreadyQueued += result.alreadyQueued;
    total.skipped += result.skipped;
  }
  return total;
}

function uniqueIds(clipIds: readonly string[]): string[] {
  return [...new Set(clipIds)];
}
