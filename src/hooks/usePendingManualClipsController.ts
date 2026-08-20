import { useCallback, useEffect, useRef, useState } from "react";
import {
  commandErrorMessage,
  importPendingManualClip,
  listPendingManualClips,
  setPendingManualClipIgnored,
} from "../api/backend";
import type { Clip, ManualClipImportInput, PendingManualClip } from "../types";

export type UsePendingManualClipsOptions = {
  notify?: (message: string) => void;
  onImported?: (clip: Clip) => void;
};

export type PendingManualClipsController = {
  /** All rows including ignored ones; the list command is the source of truth. */
  items: PendingManualClip[];
  pendingCount: number;
  ignoredCount: number;
  showIgnored: boolean;
  isLoading: boolean;
  error: string | null;
  importingId: string | null;
  load: (options?: { preserveActivity?: boolean }) => Promise<boolean>;
  importClip: (pendingId: string, input: ManualClipImportInput) => Promise<boolean>;
  setIgnored: (pendingId: string, ignored: boolean) => Promise<boolean>;
  toggleShowIgnored: () => void;
};

export function usePendingManualClipsController({
  notify,
  onImported,
}: UsePendingManualClipsOptions): PendingManualClipsController {
  const notifyRef = useRef(notify);
  const onImportedRef = useRef(onImported);
  notifyRef.current = notify;
  onImportedRef.current = onImported;
  const [items, setItems] = useState<PendingManualClip[]>([]);
  const [showIgnored, setShowIgnored] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importingId, setImportingId] = useState<string | null>(null);

  const load = useCallback(async (
    { preserveActivity = false }: { preserveActivity?: boolean } = {},
  ): Promise<boolean> => {
    setIsLoading(true);
    try {
      const nextItems = await listPendingManualClips(true);
      setItems(nextItems);
      setError(null);
      return true;
    } catch (loadError) {
      setError(commandErrorMessage(loadError));
      if (!preserveActivity) notifyRef.current?.(`待录入列表加载失败：${commandErrorMessage(loadError)}`);
      return false;
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const importClip = useCallback(async (
    pendingId: string,
    input: ManualClipImportInput,
  ): Promise<boolean> => {
    setImportingId(pendingId);
    setError(null);
    try {
      const clip = await importPendingManualClip(pendingId, input);
      setItems((current) => current.filter((pending) => pending.id !== pendingId));
      onImportedRef.current?.(clip);
      notifyRef.current?.(`已录入 ${clip.fileName}`);
      return true;
    } catch (importError) {
      const message = commandErrorMessage(importError);
      setError(message);
      notifyRef.current?.(`录入失败：${message}`);
      return false;
    } finally {
      setImportingId(null);
    }
  }, []);

  const setIgnored = useCallback(async (
    pendingId: string,
    ignored: boolean,
  ): Promise<boolean> => {
    setError(null);
    try {
      await setPendingManualClipIgnored(pendingId, ignored);
      setItems((current) => current.map((pending) => (
        pending.id === pendingId ? { ...pending, ignored } : pending
      )));
      return true;
    } catch (ignoreError) {
      const message = commandErrorMessage(ignoreError);
      setError(message);
      notifyRef.current?.(`更新待录入视频失败：${message}`);
      return false;
    }
  }, []);

  const toggleShowIgnored = useCallback(() => {
    setShowIgnored((current) => !current);
  }, []);

  const pendingCount = items.filter((pending) => !pending.ignored).length;
  const ignoredCount = items.length - pendingCount;

  return {
    items,
    pendingCount,
    ignoredCount,
    showIgnored,
    isLoading,
    error,
    importingId,
    load,
    importClip,
    setIgnored,
    toggleShowIgnored,
  };
}

