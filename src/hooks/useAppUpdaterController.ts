import { useCallback, useEffect, useRef, useState } from "react";
import {
  appUpdaterClient,
  normalizeAppUpdateError,
  type AppUpdateMetadata,
  type AppUpdateRuntimeInfo,
  type AppUpdaterClient,
} from "../services/appUpdater";

export const AUTO_UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1_000;
export const AUTO_UPDATE_CHECK_STORAGE_KEY = "valoframe.updater.lastAutomaticCheckAt.v1";

export type AppUpdaterPhase =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "cancelling"
  | "downloaded"
  | "discarding"
  | "installing"
  | "restarting"
  | "error";

export type AppUpdaterProgress = {
  downloadedBytes: number;
  totalBytes: number | null;
};

export type AppUpdaterRuntimeStatus = "loading" | "ready" | "unconfigured" | "error";
export type AppUpdaterFailedAction =
  | "check"
  | "download"
  | "cancel-download"
  | "discard"
  | "install";

export type AppUpdaterController = {
  runtimeInfo: AppUpdateRuntimeInfo | null;
  runtimeStatus: AppUpdaterRuntimeStatus;
  runtimeError: ReturnType<typeof normalizeAppUpdateError> | null;
  phase: AppUpdaterPhase;
  update: AppUpdateMetadata | null;
  progress: AppUpdaterProgress;
  message: string;
  error: ReturnType<typeof normalizeAppUpdateError> | null;
  failedAction: AppUpdaterFailedAction | null;
  canCheck: boolean;
  canDownload: boolean;
  canCancelDownload: boolean;
  canDiscard: boolean;
  canInstall: boolean;
  refreshRuntimeInfo: () => Promise<void>;
  checkManually: () => Promise<void>;
  download: () => Promise<void>;
  cancelDownload: () => Promise<void>;
  discardUpdate: () => Promise<void>;
  installAndRestart: () => Promise<void>;
};

type UseAppUpdaterControllerOptions = {
  client?: AppUpdaterClient;
  storage?: Pick<Storage, "getItem" | "setItem"> | null;
  now?: () => number;
  automaticCheck?: boolean;
};

type AppUpdaterOperation = {
  id: number;
  kind: "check" | "download" | "discard" | "install";
};

let inMemoryAutomaticCheckAt: number | null = null;

export function useAppUpdaterController(
  options: UseAppUpdaterControllerOptions = {},
): AppUpdaterController {
  const client = options.client ?? appUpdaterClient;
  const storage = options.storage === undefined
    ? getDefaultStorage()
    : options.storage;
  const now = options.now ?? Date.now;
  const automaticCheck = options.automaticCheck ?? true;
  const mountedRef = useRef(false);
  const phaseRef = useRef<AppUpdaterPhase>("idle");
  const runtimeInfoRef = useRef<AppUpdateRuntimeInfo | null>(null);
  const updateRef = useRef<AppUpdateMetadata | null>(null);
  const operationRef = useRef<AppUpdaterOperation | null>(null);
  const operationSequenceRef = useRef(0);
  const runtimeRequestSequenceRef = useRef(0);
  const storageRef = useRef(storage);
  const nowRef = useRef(now);
  const automaticCheckRef = useRef(automaticCheck);
  const [runtimeInfo, setRuntimeInfo] = useState<AppUpdateRuntimeInfo | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<AppUpdaterRuntimeStatus>("loading");
  const [runtimeError, setRuntimeError] = useState<ReturnType<typeof normalizeAppUpdateError> | null>(null);
  const [phase, setPhase] = useState<AppUpdaterPhase>("idle");
  const [update, setUpdate] = useState<AppUpdateMetadata | null>(null);
  const [progress, setProgress] = useState<AppUpdaterProgress>({
    downloadedBytes: 0,
    totalBytes: null,
  });
  const [message, setMessage] = useState("更新检查尚未运行");
  const [error, setError] = useState<ReturnType<typeof normalizeAppUpdateError> | null>(null);
  const [failedAction, setFailedAction] = useState<AppUpdaterFailedAction | null>(null);

  storageRef.current = storage;
  nowRef.current = now;
  automaticCheckRef.current = automaticCheck;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const setCurrentPhase = useCallback((nextPhase: AppUpdaterPhase) => {
    phaseRef.current = nextPhase;
    setPhase(nextPhase);
  }, []);

  const setCurrentUpdate = useCallback((nextUpdate: AppUpdateMetadata | null) => {
    updateRef.current = nextUpdate;
    setUpdate(nextUpdate);
  }, []);

  const beginOperation = useCallback((kind: AppUpdaterOperation["kind"]): number | null => {
    if (operationRef.current) return null;
    const id = ++operationSequenceRef.current;
    operationRef.current = { id, kind };
    return id;
  }, []);

  const ownsOperation = useCallback((kind: AppUpdaterOperation["kind"], id: number): boolean => {
    const current = operationRef.current;
    return mountedRef.current && current?.kind === kind && current.id === id;
  }, []);

  const finishOperation = useCallback((kind: AppUpdaterOperation["kind"], id: number) => {
    const current = operationRef.current;
    if (current?.kind === kind && current.id === id) {
      operationRef.current = null;
    }
  }, []);

  const performCheck = useCallback(async (
    manual: boolean,
    automaticAttemptAt?: number,
  ): Promise<boolean> => {
    if (
      operationRef.current
      || runtimeInfoRef.current?.configured !== true
      || updateRef.current !== null
      || phaseRef.current === "available"
      || phaseRef.current === "downloaded"
      || phaseRef.current === "installing"
      || phaseRef.current === "restarting"
    ) {
      return false;
    }
    const operationId = beginOperation("check");
    if (operationId === null) return false;
    if (!manual && automaticAttemptAt !== undefined) {
      writeAutomaticCheckAt(storageRef.current, automaticAttemptAt);
    }
    if (mountedRef.current) {
      setCurrentPhase("checking");
      setCurrentUpdate(null);
      setError(null);
      setFailedAction(null);
      setMessage(manual ? "正在手动检查稳定版本…" : "正在后台检查稳定版本…");
    }
    try {
      const nextUpdate = await client.check();
      if (!ownsOperation("check", operationId)) return true;
      setCurrentUpdate(nextUpdate);
      if (nextUpdate) {
        setCurrentPhase("available");
        setMessage(`发现稳定版本 v${nextUpdate.version}`);
      } else {
        setCurrentPhase("up-to-date");
        setMessage("当前已是最新稳定版本");
      }
    } catch (requestError) {
      if (!ownsOperation("check", operationId)) return true;
      if (manual) {
        const normalized = normalizeAppUpdateError(requestError);
        setError(normalized);
        setFailedAction("check");
        setCurrentPhase("error");
        setMessage(normalized.message);
      } else {
        // Automatic checks are intentionally silent so offline use is never interrupted.
        setCurrentPhase("idle");
        setError(null);
        setFailedAction(null);
        setMessage("后台更新检查暂不可用，可稍后手动重试");
      }
    } finally {
      finishOperation("check", operationId);
    }
    return true;
  }, [beginOperation, client, finishOperation, ownsOperation, setCurrentPhase, setCurrentUpdate]);

  const maybeStartAutomaticCheck = useCallback((announceExistingAttempt: boolean) => {
    if (!automaticCheckRef.current || runtimeInfoRef.current?.configured !== true) return;
    const timestamp = nowRef.current();
    if (!isAutomaticUpdateCheckDue(readAutomaticCheckAt(storageRef.current), timestamp)) {
      if (
        announceExistingAttempt
        && phaseRef.current === "idle"
        && operationRef.current === null
        && mountedRef.current
      ) {
        setMessage("今日已尝试自动更新检查");
      }
      return;
    }
    void performCheck(false, timestamp);
  }, [performCheck]);

  const refreshRuntimeInfo = useCallback(async () => {
    const requestId = ++runtimeRequestSequenceRef.current;
    if (mountedRef.current) {
      setRuntimeStatus("loading");
      setRuntimeError(null);
    }
    try {
      const info = await client.getRuntimeInfo();
      if (!mountedRef.current || requestId !== runtimeRequestSequenceRef.current) return;
      runtimeInfoRef.current = info;
      setRuntimeInfo(info);
      setRuntimeError(null);
      if (!info.configured) {
        setRuntimeStatus("unconfigured");
        if (phaseRef.current === "idle" && operationRef.current === null) {
          setMessage("当前构建未配置正式更新公钥");
        }
        return;
      }
      setRuntimeStatus("ready");
      if (automaticCheckRef.current) {
        maybeStartAutomaticCheck(true);
      } else if (phaseRef.current === "idle" && operationRef.current === null) {
        setMessage("更新检查尚未运行");
      }
    } catch (requestError) {
      if (!mountedRef.current || requestId !== runtimeRequestSequenceRef.current) return;
      const normalized = normalizeAppUpdateError(requestError);
      setRuntimeStatus("error");
      setRuntimeError(normalized);
      if (phaseRef.current === "idle" && operationRef.current === null) {
        setMessage(`无法读取更新配置：${normalized.message}`);
      }
    }
  }, [client, maybeStartAutomaticCheck]);

  useEffect(() => {
    void refreshRuntimeInfo();
    return () => {
      runtimeRequestSequenceRef.current += 1;
    };
  }, [refreshRuntimeInfo]);

  useEffect(() => {
    if (!automaticCheck || typeof window === "undefined" || typeof document === "undefined") {
      return undefined;
    }
    const checkWhenVisible = () => {
      if (document.visibilityState === "visible") {
        maybeStartAutomaticCheck(false);
      }
    };
    const checkWhenFocused = () => maybeStartAutomaticCheck(false);
    checkWhenVisible();
    window.addEventListener("focus", checkWhenFocused);
    document.addEventListener("visibilitychange", checkWhenVisible);
    return () => {
      window.removeEventListener("focus", checkWhenFocused);
      document.removeEventListener("visibilitychange", checkWhenVisible);
    };
  }, [automaticCheck, maybeStartAutomaticCheck]);

  const checkManually = useCallback(async () => {
    await performCheck(true);
  }, [performCheck]);

  const download = useCallback(async () => {
    if (operationRef.current || phaseRef.current !== "available" || !update) return;
    const operationId = beginOperation("download");
    if (operationId === null) return;
    setCurrentPhase("downloading");
    setError(null);
    setFailedAction(null);
    setMessage(`正在下载 v${update.version}…`);
    setProgress({ downloadedBytes: 0, totalBytes: null });
    try {
      await client.download((event) => {
        if (!ownsOperation("download", operationId)) return;
        if (event.event === "Started") {
          setProgress({
            downloadedBytes: 0,
            totalBytes: event.data.contentLength ?? null,
          });
        } else if (event.event === "Progress") {
          setProgress((current) => ({
            ...current,
            downloadedBytes: current.downloadedBytes + event.data.chunkLength,
          }));
        } else if (event.event === "Verifying" || event.event === "Finished") {
          if (!isUpdaterPhase(phaseRef, "cancelling")) {
            setError(null);
            setFailedAction(null);
            setMessage("更新包下载完成，正在验证签名…");
          }
        }
      });
      if (!ownsOperation("download", operationId)) return;
      setError(null);
      setFailedAction(null);
      setCurrentPhase("downloaded");
      setMessage("更新包已下载并通过签名验证，可以安装");
    } catch (requestError) {
      if (!ownsOperation("download", operationId)) return;
      const normalized = normalizeAppUpdateError(requestError);
      if (normalized.code === "update-download-cancelled") {
        setCurrentPhase("available");
        setMessage(normalized.message);
        setError(null);
        setFailedAction(null);
      } else if (normalized.retryable) {
        setCurrentPhase("available");
        setMessage(`${normalized.message}；可以直接重试下载`);
        setError(normalized);
        setFailedAction("download");
      } else {
        setCurrentPhase("error");
        setMessage(normalized.message);
        setError(normalized);
        setFailedAction("download");
      }
    } finally {
      finishOperation("download", operationId);
    }
  }, [beginOperation, client, finishOperation, ownsOperation, setCurrentPhase, update]);

  const cancelDownload = useCallback(async () => {
    const activeOperation = operationRef.current;
    if (!isUpdaterPhase(phaseRef, "downloading") || activeOperation?.kind !== "download") return;
    const operationId = activeOperation.id;
    setCurrentPhase("cancelling");
    setError(null);
    setFailedAction(null);
    setMessage("正在取消更新下载…");
    try {
      const accepted = await client.cancelDownload();
      if (!ownsOperation("download", operationId) || !isUpdaterPhase(phaseRef, "cancelling")) return;
      if (!accepted) {
        setCurrentPhase("downloading");
        setMessage("下载已进入收尾阶段，暂时无法取消");
      }
    } catch (requestError) {
      if (!ownsOperation("download", operationId) || !isUpdaterPhase(phaseRef, "cancelling")) return;
      const normalized = normalizeAppUpdateError(requestError);
      setError(normalized);
      setFailedAction("cancel-download");
      setCurrentPhase("downloading");
      setMessage(`取消下载失败，下载仍在继续：${normalized.message}`);
    }
  }, [client, ownsOperation, setCurrentPhase]);

  const discardUpdate = useCallback(async () => {
    const previousPhase = phaseRef.current;
    if (
      operationRef.current
      || update === null
      || (previousPhase !== "available"
        && previousPhase !== "downloaded"
        && previousPhase !== "error")
    ) {
      return;
    }
    const operationId = beginOperation("discard");
    if (operationId === null) return;
    const discardedVersion = update.version;
    setCurrentPhase("discarding");
    setError(null);
    setFailedAction(null);
    setMessage(`正在放弃 v${discardedVersion}…`);
    try {
      const discarded = await client.discard();
      if (!ownsOperation("discard", operationId)) return;
      if (!discarded) {
        throw {
          code: "update-discard-rejected",
          message: "未能放弃此更新，请重试",
          retryable: true,
        };
      }
      setCurrentUpdate(null);
      setProgress({ downloadedBytes: 0, totalBytes: null });
      setError(null);
      setFailedAction(null);
      setCurrentPhase("idle");
      setMessage(`已放弃 v${discardedVersion}，可以重新检查更新`);
    } catch (requestError) {
      if (!ownsOperation("discard", operationId)) return;
      const normalized = normalizeAppUpdateError(requestError);
      setError(normalized);
      setFailedAction("discard");
      setCurrentPhase(previousPhase);
      setMessage(`放弃此更新失败：${normalized.message}`);
    } finally {
      finishOperation("discard", operationId);
    }
  }, [beginOperation, client, finishOperation, ownsOperation, setCurrentPhase, setCurrentUpdate, update]);

  const installAndRestart = useCallback(async () => {
    if (operationRef.current || phaseRef.current !== "downloaded") return;
    const operationId = beginOperation("install");
    if (operationId === null) return;
    setCurrentPhase("installing");
    setError(null);
    setFailedAction(null);
    setMessage("正在启动安装程序，应用即将关闭并重启…");
    try {
      await client.install();
      if (!ownsOperation("install", operationId)) return;
      setCurrentPhase("restarting");
      setMessage("更新已安装，正在重启应用…");
    } catch (requestError) {
      if (!ownsOperation("install", operationId)) return;
      const normalized = normalizeAppUpdateError(requestError);
      setError(normalized);
      setFailedAction("install");
      setCurrentPhase("downloaded");
      setMessage(normalized.message);
    } finally {
      finishOperation("install", operationId);
    }
  }, [beginOperation, client, finishOperation, ownsOperation, setCurrentPhase]);

  const canCheck = runtimeInfo?.configured === true
    && update === null
    && phase !== "checking"
    && phase !== "downloading"
    && phase !== "cancelling"
    && phase !== "available"
    && phase !== "downloaded"
    && phase !== "discarding"
    && phase !== "installing"
    && phase !== "restarting";
  const canDownload = phase === "available" && update !== null;
  const canCancelDownload = phase === "downloading";
  const canDiscard = update !== null
    && (phase === "available" || phase === "downloaded" || phase === "error");
  const canInstall = phase === "downloaded";

  return {
    runtimeInfo,
    runtimeStatus,
    runtimeError,
    phase,
    update,
    progress,
    message,
    error,
    failedAction,
    canCheck,
    canDownload,
    canCancelDownload,
    canDiscard,
    canInstall,
    refreshRuntimeInfo,
    checkManually,
    download,
    cancelDownload,
    discardUpdate,
    installAndRestart,
  };
}

export function isAutomaticUpdateCheckDue(
  lastCheckAt: number | null,
  now: number,
): boolean {
  if (!Number.isFinite(lastCheckAt) || lastCheckAt === null || lastCheckAt < 0) {
    return true;
  }
  if (lastCheckAt > now + 5 * 60 * 1_000) {
    return true;
  }
  return now - lastCheckAt >= AUTO_UPDATE_CHECK_INTERVAL_MS;
}

function readAutomaticCheckAt(
  storage: Pick<Storage, "getItem"> | null,
): number | null {
  let stored: number | null = null;
  try {
    const raw = storage?.getItem(AUTO_UPDATE_CHECK_STORAGE_KEY);
    const parsed = raw === null || raw === undefined ? Number.NaN : Number(raw);
    stored = Number.isFinite(parsed) ? parsed : null;
  } catch {
    stored = null;
  }
  if (stored === null) return inMemoryAutomaticCheckAt;
  if (inMemoryAutomaticCheckAt === null) return stored;
  return Math.max(stored, inMemoryAutomaticCheckAt);
}

function writeAutomaticCheckAt(
  storage: Pick<Storage, "setItem"> | null,
  timestamp: number,
) {
  inMemoryAutomaticCheckAt = timestamp;
  try {
    storage?.setItem(AUTO_UPDATE_CHECK_STORAGE_KEY, String(timestamp));
  } catch {
    // The in-memory timestamp still prevents repeated checks during this app session.
  }
}

function getDefaultStorage(): Pick<Storage, "getItem" | "setItem"> | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

function isUpdaterPhase(
  phaseRef: { current: AppUpdaterPhase },
  expected: AppUpdaterPhase,
): boolean {
  return phaseRef.current === expected;
}
