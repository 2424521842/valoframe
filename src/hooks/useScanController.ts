import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import {
  cancelScan as requestScanCancellation,
  commandErrorMessage,
  discoverAndScanFixedDrives,
  getScanStatus,
  listenToScanProgress,
  scanCommandErrorCode,
  scanCommandErrorJobId,
  scanDefaultAclosDir,
  scanRoots,
} from "../api/backend";
import {
  fullDriveDiscoveryActivityMessage,
  scanActivityMessage,
} from "../lib/scanSummary";
import type {
  ScanJobResult,
  ScanJobStatus,
  ScanProgress,
  ScanStatus,
  ScanSummary,
} from "../types";

const SCAN_STATUS_RECOVERY_INTERVAL_MS = 400;
const STALE_SCAN_JOB_LIMIT = 8;

export type ScanControllerNotification = {
  kind: "info" | "progress" | "success" | "warning" | "error";
  message: string;
};

export type UseScanControllerOptions = {
  sourcePaths: readonly string[];
  refresh: () => Promise<boolean>;
  notify: (notification: ScanControllerNotification) => void;
};

export type ScanController = {
  activeJobId: string | null;
  status: ScanJobStatus;
  phase: string | null;
  currentRoot: string | null;
  source: string | null;
  processed: number;
  total: number | null;
  message: string;
  progress: ScanProgress | null;
  summary: ScanSummary | null;
  errorMessage: string | null;
  isScanning: boolean;
  isCancelling: boolean;
  startScan: () => Promise<void>;
  discoverAll: () => Promise<void>;
  cancelScan: () => Promise<void>;
  clearOutcome: () => void;
  clearSummary: () => void;
  reportError: (message: string) => void;
};

export function useScanController({
  sourcePaths,
  refresh,
  notify,
}: UseScanControllerOptions): ScanController {
  const mountedRef = useRef(false);
  const scanRequestActiveRef = useRef(false);
  const cancelRequestActiveRef = useRef(false);
  const activeJobIdRef = useRef<string | null>(null);
  const statusRef = useRef<ScanJobStatus>("idle");
  const staleJobIdsRef = useRef(new Set<string>());
  const sourcePathsRef = useRef<readonly string[]>(sourcePaths);
  const refreshRef = useRef(refresh);
  const notifyRef = useRef(notify);

  sourcePathsRef.current = sourcePaths;
  refreshRef.current = refresh;
  notifyRef.current = notify;

  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [status, setStatus] = useState<ScanJobStatus>("idle");
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [summary, setSummary] = useState<ScanSummary | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [message, setMessage] = useState("当前没有扫描任务");
  const [isScanning, setIsScanning] = useState(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const publish = useCallback((notification: ScanControllerNotification) => {
    if (!mountedRef.current) {
      return;
    }
    setMessage(notification.message);
    notifyRef.current(notification);
  }, []);

  const updateStatus = useCallback((nextStatus: ScanJobStatus) => {
    statusRef.current = nextStatus;
    if (mountedRef.current) {
      setStatus(nextStatus);
    }
  }, []);

  const rememberJob = useCallback((jobId: string) => {
    if (activeJobIdRef.current) {
      return;
    }
    activeJobIdRef.current = jobId;
    if (mountedRef.current) {
      setActiveJobId(jobId);
    }
  }, []);

  const beginRequest = useCallback((activityMessage: string): boolean => {
    if (scanRequestActiveRef.current) {
      return false;
    }

    scanRequestActiveRef.current = true;
    cancelRequestActiveRef.current = false;
    activeJobIdRef.current = null;
    statusRef.current = "running";
    if (mountedRef.current) {
      setActiveJobId(null);
      setIsScanning(true);
      setStatus("running");
      setProgress(null);
      setErrorMessage(null);
      setSummary(null);
    }
    publish({ kind: "info", message: activityMessage });
    return true;
  }, [publish]);

  const finishRequest = useCallback(() => {
    const finishedJobId = activeJobIdRef.current;
    if (finishedJobId) {
      const staleJobIds = staleJobIdsRef.current;
      staleJobIds.add(finishedJobId);
      if (staleJobIds.size > STALE_SCAN_JOB_LIMIT) {
        const oldestJobId = staleJobIds.values().next().value;
        if (typeof oldestJobId === "string") {
          staleJobIds.delete(oldestJobId);
        }
      }
    }

    activeJobIdRef.current = null;
    scanRequestActiveRef.current = false;
    cancelRequestActiveRef.current = false;
    if (mountedRef.current) {
      setActiveJobId(null);
      setIsScanning(false);
    }
  }, []);

  const applyTerminalResult = useCallback((
    response: ScanJobResult<unknown>,
    scanSummary: ScanSummary | null,
  ) => {
    rememberJob(response.jobId);
    updateStatus(response.status);
    if (scanSummary) {
      setSummary(scanSummary);
    }
    setProgress((current) => terminalProgress(
      response.jobId,
      response.status,
      response.message,
      scanSummary,
      current,
    ));
  }, [rememberJob, updateStatus]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const handleProgress = (nextProgress: ScanProgress) => {
      if (disposed || !mountedRef.current) {
        return;
      }
      if (staleJobIdsRef.current.has(nextProgress.jobId)) {
        return;
      }

      const currentJobId = activeJobIdRef.current;
      if (currentJobId && nextProgress.jobId !== currentJobId) {
        return;
      }
      if (!currentJobId) {
        if (!scanRequestActiveRef.current) {
          return;
        }
        activeJobIdRef.current = nextProgress.jobId;
        setActiveJobId(nextProgress.jobId);
      }

      statusRef.current = nextProgress.status;
      setStatus(nextProgress.status);
      setProgress(nextProgress);
      publish({ kind: "progress", message: nextProgress.message });
    };

    void listenToScanProgress(handleProgress)
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((error: unknown) => {
        if (!disposed) {
          publish({
            kind: "error",
            message: `扫描进度监听失败：${commandErrorMessage(error)}`,
          });
        }
      });

    return () => {
      disposed = true;
      unlisten?.();
      unlisten = null;
    };
  }, [publish]);

  useEffect(() => {
    if (!isScanning || !isTauri()) {
      return;
    }

    let disposed = false;
    let requestPending = false;
    const recoverActiveJob = async () => {
      if (
        disposed ||
        requestPending ||
        activeJobIdRef.current ||
        !scanRequestActiveRef.current
      ) {
        return;
      }

      requestPending = true;
      try {
        const recoveredStatus = await getScanStatus();
        if (!canRecoverStatus(recoveredStatus, disposed)) {
          return;
        }

        activeJobIdRef.current = recoveredStatus.jobId;
        setActiveJobId(recoveredStatus.jobId);
        statusRef.current = recoveredStatus.status;
        setStatus(recoveredStatus.status);
        setProgress((current) => progressFromRecoveredStatus(recoveredStatus, current));
        publish({ kind: "progress", message: recoveredStatus.message });
      } catch {
        // Progress events remain the primary path; polling only repairs a missed first event.
      } finally {
        requestPending = false;
      }
    };

    const canRecoverStatus = (
      recoveredStatus: ScanStatus,
      isDisposed: boolean,
    ): recoveredStatus is ScanStatus & { jobId: string } => (
      !isDisposed &&
      mountedRef.current &&
      scanRequestActiveRef.current &&
      Boolean(recoveredStatus.jobId) &&
      !staleJobIdsRef.current.has(recoveredStatus.jobId ?? "") &&
      (recoveredStatus.status === "running" || recoveredStatus.status === "cancelling")
    );

    void recoverActiveJob();
    const interval = window.setInterval(
      () => void recoverActiveJob(),
      SCAN_STATUS_RECOVERY_INTERVAL_MS,
    );
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [isScanning, publish]);

  const startScan = useCallback(async () => {
    const paths = [...sourcePathsRef.current];
    if (!beginRequest(`准备扫描 ${paths.length || 1} 个目录`)) {
      return;
    }

    try {
      const response = paths.length === 0
        ? await scanDefaultAclosDir()
        : await scanRoots(paths);
      if (!mountedRef.current) {
        return;
      }

      const scanSummary = response.result;
      applyTerminalResult(response, scanSummary);
      const refreshed = await refreshRef.current();
      if (!mountedRef.current) {
        return;
      }
      if (!refreshed) {
        publish({
          kind: "warning",
          message: `${scanStatusLabel(response.status)}，但刷新素材列表失败`,
        });
        return;
      }

      if (response.status === "completed" && scanSummary) {
        setErrorMessage(null);
        publish({ kind: "success", message: scanActivityMessage(scanSummary) });
      } else if (response.status === "partial") {
        setErrorMessage(`扫描部分完成：${scanSummary?.errors[0] ?? response.message}`);
        publish({ kind: "warning", message: "扫描部分完成，素材索引已刷新" });
      } else if (response.status === "cancelled") {
        setErrorMessage("扫描已取消；取消前已安全写入的索引已保留。");
        publish({ kind: "warning", message: "扫描已取消，素材索引已刷新" });
      } else {
        setErrorMessage(`扫描失败：${response.message}`);
        publish({ kind: "error", message: "扫描失败，素材索引已刷新" });
      }
    } catch (error) {
      if (!mountedRef.current) {
        return;
      }
      const failedJobId = scanCommandErrorJobId(error);
      if (failedJobId) {
        rememberJob(failedJobId);
      }
      const failureMessage = commandErrorMessage(error);
      if (scanCommandErrorCode(error) === "already-running") {
        setErrorMessage(`扫描互斥冲突：${failureMessage}`);
        publish({ kind: "warning", message: "已有扫描任务正在运行" });
      } else {
        await refreshRef.current();
        if (!mountedRef.current) {
          return;
        }
        updateStatus("failed");
        setErrorMessage(`扫描失败：${failureMessage}`);
        publish({ kind: "error", message: "扫描失败，已刷新当前索引" });
      }
    } finally {
      finishRequest();
    }
  }, [applyTerminalResult, beginRequest, finishRequest, publish, rememberJob, updateStatus]);

  const discoverAll = useCallback(async () => {
    if (!beginRequest("正在全电脑发现无畏时刻素材")) {
      return;
    }

    try {
      const response = await discoverAndScanFixedDrives();
      if (!mountedRef.current) {
        return;
      }

      const result = response.result;
      const scanSummary = result?.scanSummary ?? null;
      applyTerminalResult(response, scanSummary);
      const refreshed = await refreshRef.current();
      if (!mountedRef.current) {
        return;
      }
      if (!refreshed) {
        publish({
          kind: "warning",
          message: `${scanStatusLabel(response.status)}，但刷新素材列表失败`,
        });
      } else if (response.status === "completed" && result) {
        setErrorMessage(null);
        publish({ kind: "success", message: fullDriveDiscoveryActivityMessage(result) });
      } else if (response.status === "partial") {
        setErrorMessage(`全电脑发现部分完成：${scanSummary?.errors[0] ?? response.message}`);
        publish({ kind: "warning", message: "全电脑发现部分完成，素材索引已刷新" });
      } else if (response.status === "cancelled") {
        setErrorMessage("全电脑发现已取消；已完成的安全写入已保留。");
        publish({ kind: "warning", message: "全电脑发现已取消，素材索引已刷新" });
      } else {
        setErrorMessage(`全电脑发现失败：${response.message}`);
        publish({ kind: "error", message: "全电脑发现失败，素材索引已刷新" });
      }
    } catch (error) {
      if (!mountedRef.current) {
        return;
      }
      const failedJobId = scanCommandErrorJobId(error);
      if (failedJobId) {
        rememberJob(failedJobId);
      }
      const failureMessage = commandErrorMessage(error);
      if (scanCommandErrorCode(error) === "already-running") {
        setErrorMessage(`扫描互斥冲突：${failureMessage}`);
        publish({ kind: "warning", message: "已有扫描任务正在运行" });
      } else {
        await refreshRef.current();
        if (!mountedRef.current) {
          return;
        }
        updateStatus("failed");
        setErrorMessage(`全电脑发现失败：${failureMessage}`);
        publish({ kind: "error", message: "全电脑发现失败，已刷新当前索引" });
      }
    } finally {
      finishRequest();
    }
  }, [applyTerminalResult, beginRequest, finishRequest, publish, rememberJob, updateStatus]);

  const cancelScan = useCallback(async () => {
    const jobId = activeJobIdRef.current;
    if (!jobId || statusRef.current === "cancelling" || cancelRequestActiveRef.current) {
      return;
    }

    cancelRequestActiveRef.current = true;
    updateStatus("cancelling");
    setProgress((current) => current?.jobId === jobId
      ? {
        ...current,
        phase: "cancelling",
        status: "cancelling",
        message: "正在取消扫描",
      }
      : current);
    publish({ kind: "info", message: "正在取消扫描" });
    try {
      const result = await requestScanCancellation(jobId);
      if (!mountedRef.current) {
        return;
      }
      if (result.accepted) {
        publish({ kind: "info", message: "已请求取消，正在完成安全清理" });
      } else if (result.reason === "job-mismatch") {
        setErrorMessage("取消请求已过期，新的扫描任务未受影响。");
        publish({ kind: "warning", message: "未取消新的扫描任务" });
      } else {
        setErrorMessage("扫描任务已经结束，无需取消。");
        publish({ kind: "warning", message: "扫描任务已结束" });
      }
    } catch (error) {
      if (!mountedRef.current) {
        return;
      }
      cancelRequestActiveRef.current = false;
      setErrorMessage(`取消扫描失败：${commandErrorMessage(error)}`);
      publish({ kind: "error", message: "取消请求失败，扫描仍在运行" });
      updateStatus("running");
    }
  }, [publish, updateStatus]);

  const clearOutcome = useCallback(() => {
    setSummary(null);
    setErrorMessage(null);
  }, []);

  const clearSummary = useCallback(() => {
    setSummary(null);
  }, []);

  const reportError = useCallback((nextErrorMessage: string) => {
    setErrorMessage(nextErrorMessage);
  }, []);

  return {
    activeJobId,
    status,
    phase: progress?.phase ?? null,
    currentRoot: progress?.currentRoot ?? null,
    source: progress?.source ?? null,
    processed: progress?.processed ?? 0,
    total: progress?.total ?? null,
    message,
    progress,
    summary,
    errorMessage,
    isScanning,
    isCancelling: isScanning && status === "cancelling",
    startScan,
    discoverAll,
    cancelScan,
    clearOutcome,
    clearSummary,
    reportError,
  };
}

function terminalProgress(
  jobId: string,
  status: ScanJobStatus,
  message: string,
  summary: ScanSummary | null,
  current: ScanProgress | null,
): ScanProgress {
  const processed = summary?.sourceDirCount ?? current?.processed ?? 0;
  return {
    jobId,
    phase: status,
    currentRoot: summary?.rootPath || current?.currentRoot || null,
    source: current?.jobId === jobId ? current.source : null,
    processed,
    total: processed > 0 ? processed : current?.total ?? null,
    terminal: true,
    status,
    sourceDirCount: summary?.sourceDirCount ?? current?.sourceDirCount ?? 0,
    clipGroupCount: summary?.clipGroupCount ?? current?.clipGroupCount ?? 0,
    clipFileCount: current?.clipFileCount ?? 0,
    message,
  };
}

function progressFromRecoveredStatus(
  status: ScanStatus & { jobId: string },
  current: ScanProgress | null,
): ScanProgress {
  const currentJobProgress = current?.jobId === status.jobId ? current : null;
  return {
    jobId: status.jobId,
    phase: status.phase ?? status.status,
    currentRoot: status.currentRoot,
    source: status.source,
    processed: status.processed,
    total: status.total,
    terminal: status.terminal,
    status: status.status,
    sourceDirCount: currentJobProgress?.sourceDirCount ?? 0,
    clipGroupCount: currentJobProgress?.clipGroupCount ?? 0,
    clipFileCount: currentJobProgress?.clipFileCount ?? 0,
    message: status.message,
  };
}

function scanStatusLabel(status: ScanJobStatus): string {
  switch (status) {
    case "completed":
      return "扫描完成";
    case "partial":
      return "扫描部分完成";
    case "cancelled":
      return "扫描已取消";
    case "failed":
      return "扫描失败";
    default:
      return "扫描结束";
  }
}
