import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import {
  cancelScan as requestScanCancellation,
  commandErrorMessage,
  discoverAndScanFixedDrives,
  getScanStatus,
  getScanSummary,
  listenToScanProgress,
  scanCommandErrorActiveJobId,
  scanCommandErrorCode,
  scanCommandErrorJobId,
  scanDefaultAclosDir,
  scanRoots,
  syncEnabledSources as requestEnabledSourceSync,
  syncScanSource as requestSourceSync,
} from "../api/backend";
import {
  scanTerminalActivityMessage,
} from "../lib/scanSummary";
import type {
  ScanJobResult,
  ScanJobStatus,
  ScanProgress,
  ScanStatus,
  ScanSummary,
} from "../types";

const SCAN_STATUS_RECOVERY_INTERVAL_MS = 400;
const TERMINAL_SUMMARY_TIMEOUT_MS = 2_500;
const TERMINAL_REFRESH_TIMEOUT_MS = 10_000;
const FINISHED_SCAN_JOB_LIMIT = 16;
const TERMINAL_NOTIFICATION_LIMIT = 32;

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
  syncSource: (sourceId: string) => Promise<void>;
  syncEnabledSources: () => Promise<void>;
  discoverAll: () => Promise<void>;
  cancelScan: () => Promise<void>;
  settleExternalTerminal: (input: ExternalTerminalInput) => Promise<void>;
  clearOutcome: () => void;
  clearSummary: () => void;
  reportError: (message: string) => void;
};

type TerminalScanJobStatus = Extract<
  ScanJobStatus,
  "completed" | "partial" | "cancelled" | "failed"
>;

type TerminalInput = {
  jobId: string;
  status: ScanJobStatus;
  message: string;
  summary?: ScanSummary | null;
};

type ExternalTerminalInput = Omit<TerminalInput, "status"> & {
  status: TerminalScanJobStatus;
};

export function useScanController({
  sourcePaths,
  refresh,
  notify,
}: UseScanControllerOptions): ScanController {
  const mountedRef = useRef(false);
  const scanRequestActiveRef = useRef(false);
  const requestSequenceRef = useRef(0);
  const activeRequestTokenRef = useRef(0);
  const cancelRequestActiveRef = useRef(false);
  const activeJobIdRef = useRef<string | null>(null);
  const statusRef = useRef<ScanJobStatus>("idle");
  const latestProgressRef = useRef<ScanProgress | null>(null);
  const trackingAdoptedJobRef = useRef(false);
  const finishedJobIdsRef = useRef(new Set<string>());
  const notifiedTerminalJobIdsRef = useRef(new Set<string>());
  const refreshedTerminalJobIdsRef = useRef(new Set<string>());
  const terminalSummaryHintsRef = useRef(new Map<string, ScanSummary>());
  const terminalSettlementsRef = useRef(new Map<string, Promise<void>>());
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
    if (!mountedRef.current) return;
    setMessage(notification.message);
    notifyRef.current(notification);
  }, []);

  const updateStatus = useCallback((nextStatus: ScanJobStatus) => {
    statusRef.current = nextStatus;
    if (mountedRef.current) setStatus(nextStatus);
  }, []);

  const rememberJob = useCallback((jobId: string) => {
    if (activeJobIdRef.current && activeJobIdRef.current !== jobId) return;
    activeJobIdRef.current = jobId;
    if (mountedRef.current) setActiveJobId(jobId);
  }, []);

  const beginRequest = useCallback((activityMessage: string): number | null => {
    if (scanRequestActiveRef.current) return null;

    const requestToken = requestSequenceRef.current + 1;
    requestSequenceRef.current = requestToken;
    activeRequestTokenRef.current = requestToken;
    scanRequestActiveRef.current = true;
    cancelRequestActiveRef.current = false;
    activeJobIdRef.current = null;
    statusRef.current = "running";
    latestProgressRef.current = null;
    trackingAdoptedJobRef.current = false;
    if (mountedRef.current) {
      setActiveJobId(null);
      setIsScanning(true);
      setStatus("running");
      setProgress(null);
      setErrorMessage(null);
      setSummary(null);
    }
    publish({ kind: "info", message: activityMessage });
    return requestToken;
  }, [publish]);

  const finishRequest = useCallback((jobId?: string, requestToken?: number) => {
    if (
      requestToken !== undefined &&
      activeRequestTokenRef.current !== requestToken
    ) {
      return;
    }
    if (
      jobId &&
      activeJobIdRef.current &&
      activeJobIdRef.current !== jobId
    ) {
      return;
    }
    const finishedJobId = jobId ?? activeJobIdRef.current;
    if (finishedJobId) {
      addBounded(finishedJobIdsRef.current, finishedJobId, FINISHED_SCAN_JOB_LIMIT);
    }
    activeJobIdRef.current = null;
    activeRequestTokenRef.current = 0;
    scanRequestActiveRef.current = false;
    cancelRequestActiveRef.current = false;
    trackingAdoptedJobRef.current = false;
    if (mountedRef.current) {
      setActiveJobId(null);
      setIsScanning(false);
    }
  }, []);

  const applyTerminalState = useCallback((input: TerminalInput, scanSummary: ScanSummary | null) => {
    if (
      activeJobIdRef.current &&
      activeJobIdRef.current !== input.jobId
    ) {
      return;
    }
    updateStatus(input.status);
    if (!mountedRef.current) return;
    setSummary(scanSummary);
    setProgress((current) => {
      const nextProgress = terminalProgress(
        input.jobId,
        input.status,
        input.message,
        scanSummary,
        current,
      );
      latestProgressRef.current = nextProgress;
      return nextProgress;
    });
  }, [updateStatus]);

  const publishTerminalOutcome = useCallback((input: TerminalInput, scanSummary: ScanSummary | null) => {
    if (
      activeJobIdRef.current &&
      activeJobIdRef.current !== input.jobId
    ) {
      return;
    }
    if (notifiedTerminalJobIdsRef.current.has(input.jobId)) return;
    addBounded(
      notifiedTerminalJobIdsRef.current,
      input.jobId,
      TERMINAL_NOTIFICATION_LIMIT,
    );

    const terminalMessage = scanTerminalActivityMessage(input.status, scanSummary);
    if (input.status === "completed") {
      setErrorMessage(null);
      publish({ kind: "success", message: terminalMessage });
    } else if (input.status === "partial") {
      setErrorMessage(`扫描部分完成：${scanSummary?.errors[0] ?? input.message}`);
      publish({ kind: "warning", message: terminalMessage });
    } else if (input.status === "cancelled") {
      setErrorMessage("扫描已取消；取消前已安全写入的索引已保留。");
      publish({ kind: "warning", message: terminalMessage });
    } else {
      setErrorMessage(`扫描失败：${input.message}`);
      publish({ kind: "error", message: terminalMessage });
    }
  }, [publish]);

  const settleTerminalJob = useCallback((input: TerminalInput): Promise<void> => {
    rememberJob(input.jobId);
    if (input.summary) {
      setBoundedMap(
        terminalSummaryHintsRef.current,
        input.jobId,
        input.summary,
        TERMINAL_NOTIFICATION_LIMIT,
      );
    }
    applyTerminalState(
      input,
      terminalSummaryHintsRef.current.get(input.jobId) ?? null,
    );

    const existing = terminalSettlementsRef.current.get(input.jobId);
    if (existing) return existing;

    const settlement = (async () => {
      let recoveredSummary = terminalSummaryHintsRef.current.get(input.jobId) ?? null;
      if (!recoveredSummary) {
        const summaryResult = await settleWithin(
          Promise.resolve().then(() => getScanSummary(input.jobId)),
          TERMINAL_SUMMARY_TIMEOUT_MS,
        );
        if (summaryResult.status === "fulfilled") {
          recoveredSummary = terminalSummaryHintsRef.current.get(input.jobId)
            ?? summaryResult.value;
        }
      }
      if (
        !mountedRef.current ||
        (
          scanRequestActiveRef.current &&
          activeJobIdRef.current !== input.jobId
        )
      ) {
        return;
      }

      applyTerminalState(input, recoveredSummary);
      publishTerminalOutcome(input, recoveredSummary);

      if (!refreshedTerminalJobIdsRef.current.has(input.jobId)) {
        addBounded(
          refreshedTerminalJobIdsRef.current,
          input.jobId,
          TERMINAL_NOTIFICATION_LIMIT,
        );
        const refreshResult = await settleWithin(
          Promise.resolve().then(() => refreshRef.current()),
          TERMINAL_REFRESH_TIMEOUT_MS,
        );
        if (
          !mountedRef.current ||
          (
            scanRequestActiveRef.current &&
            activeJobIdRef.current !== input.jobId
          )
        ) {
          return;
        }
        if (refreshResult.status === "timed-out") {
          const refreshMessage = `${scanTerminalActivityMessage(input.status, recoveredSummary)}；终态已确定，索引视图刷新超时，可稍后手动刷新`;
          setErrorMessage(refreshMessage);
          publish({ kind: "warning", message: refreshMessage });
        } else if (refreshResult.status === "rejected") {
          const refreshMessage = `${scanTerminalActivityMessage(input.status, recoveredSummary)}；终态已确定，但刷新索引视图失败：${commandErrorMessage(refreshResult.reason)}`;
          setErrorMessage(refreshMessage);
          publish({ kind: "error", message: refreshMessage });
        } else if (!refreshResult.value) {
          const refreshMessage = `${scanTerminalActivityMessage(input.status, recoveredSummary)}；终态已确定，但刷新索引视图失败`;
          setErrorMessage(refreshMessage);
          publish({ kind: "warning", message: refreshMessage });
        }
      }
    })().finally(() => {
      terminalSettlementsRef.current.delete(input.jobId);
    });
    terminalSettlementsRef.current.set(input.jobId, settlement);
    return settlement;
  }, [applyTerminalState, publish, publishTerminalOutcome, rememberJob]);

  const finishObservedJob = useCallback((terminalProgress: ScanProgress) => {
    const jobId = terminalProgress.jobId;
    if (
      activeJobIdRef.current &&
      activeJobIdRef.current !== jobId
    ) {
      return;
    }
    void settleTerminalJob({
      jobId,
      status: terminalProgress.status,
      message: terminalProgress.message,
    });
    finishRequest(jobId);
  }, [finishRequest, settleTerminalJob]);

  const trackConflictingJob = useCallback((error: unknown) => {
    const conflictJobId = scanCommandErrorActiveJobId(error)
      ?? scanCommandErrorJobId(error)
      ?? activeJobIdRef.current;
    const observedProgress = latestProgressRef.current;

    trackingAdoptedJobRef.current = true;
    if (conflictJobId) {
      finishedJobIdsRef.current.delete(conflictJobId);
      activeJobIdRef.current = conflictJobId;
      if (observedProgress?.jobId !== conflictJobId) latestProgressRef.current = null;
      if (mountedRef.current) {
        setActiveJobId(conflictJobId);
        if (observedProgress?.jobId !== conflictJobId) {
          setProgress(null);
          updateStatus("running");
        }
      }
    }

    if (
      observedProgress?.terminal &&
      (!conflictJobId || observedProgress.jobId === conflictJobId)
    ) {
      finishObservedJob(observedProgress);
    }
  }, [finishObservedJob, updateStatus]);

  const acceptProgress = useCallback((nextProgress: ScanProgress) => {
    if (!mountedRef.current || finishedJobIdsRef.current.has(nextProgress.jobId)) return;

    const latestProgress = latestProgressRef.current;
    if (
      (latestProgress?.jobId === nextProgress.jobId && latestProgress.terminal) ||
      (
        activeJobIdRef.current === nextProgress.jobId &&
        cancelRequestActiveRef.current &&
        statusRef.current === "cancelling" &&
        nextProgress.status === "running"
      )
    ) {
      return;
    }

    const currentJobId = activeJobIdRef.current;
    if (currentJobId && nextProgress.jobId !== currentJobId) return;
    if (!currentJobId) {
      if (!scanRequestActiveRef.current) {
        const requestToken = requestSequenceRef.current + 1;
        requestSequenceRef.current = requestToken;
        activeRequestTokenRef.current = requestToken;
        scanRequestActiveRef.current = true;
        trackingAdoptedJobRef.current = true;
        cancelRequestActiveRef.current = false;
        setIsScanning(true);
        setSummary(null);
        setErrorMessage(null);
      }
      activeJobIdRef.current = nextProgress.jobId;
      setActiveJobId(nextProgress.jobId);
    }

    statusRef.current = nextProgress.status;
    latestProgressRef.current = nextProgress;
    setStatus(nextProgress.status);
    setProgress(nextProgress);
    if (nextProgress.terminal) {
      finishObservedJob(nextProgress);
    } else {
      publish({ kind: "progress", message: nextProgress.message });
    }
  }, [finishObservedJob, publish]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listenToScanProgress((nextProgress) => {
      if (!disposed) acceptProgress(nextProgress);
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
        if (!isTauri() || scanRequestActiveRef.current) return;

        void getScanStatus()
          .then((recoveredStatus) => {
            const recoveredJobId = recoveredStatus.jobId;
            if (
              disposed ||
              scanRequestActiveRef.current ||
              !recoveredJobId ||
              !(
                recoveredStatus.status === "running" ||
                recoveredStatus.status === "cancelling" ||
                recoveredStatus.terminal
              )
            ) {
              return;
            }
            acceptProgress(progressFromRecoveredStatus(
              { ...recoveredStatus, jobId: recoveredJobId },
              null,
            ));
          })
          .catch(() => {
            // Live events remain authoritative when startup recovery is unavailable.
          });
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
    };
  }, [acceptProgress, publish]);

  useEffect(() => {
    if (!isScanning || !isTauri()) return;

    let disposed = false;
    let requestPending = false;
    const recoverActiveJob = async () => {
      if (
        disposed ||
        requestPending ||
        !scanRequestActiveRef.current
      ) {
        return;
      }

      requestPending = true;
      try {
        const recoveredStatus = await getScanStatus();
        const recoveredJobId = recoveredStatus.jobId;
        if (
          disposed ||
          !mountedRef.current ||
          !scanRequestActiveRef.current ||
          !recoveredJobId ||
          (activeJobIdRef.current && activeJobIdRef.current !== recoveredJobId) ||
          finishedJobIdsRef.current.has(recoveredJobId) ||
          !(
            recoveredStatus.status === "running" ||
            recoveredStatus.status === "cancelling" ||
            recoveredStatus.terminal
          )
        ) {
          return;
        }
        acceptProgress(progressFromRecoveredStatus(
          { ...recoveredStatus, jobId: recoveredJobId },
          latestProgressRef.current,
        ));
      } catch {
        // Polling only repairs missed events.
      } finally {
        requestPending = false;
      }
    };

    void recoverActiveJob();
    const interval = window.setInterval(
      () => void recoverActiveJob(),
      SCAN_STATUS_RECOVERY_INTERVAL_MS,
    );
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [acceptProgress, isScanning]);

  const handleFailedRequest = useCallback((error: unknown, context: string) => {
    const failureMessage = commandErrorMessage(error);
    const failedJobId = scanCommandErrorJobId(error);
    if (failedJobId) {
      rememberJob(failedJobId);
      void settleTerminalJob({
        jobId: failedJobId,
        status: "failed",
        message: failureMessage,
      });
      return;
    }

    updateStatus("failed");
    setErrorMessage(`${context}失败：${failureMessage}`);
    publish({ kind: "error", message: "扫描失败：新增数量不可用" });
    void settleWithin(
      Promise.resolve().then(() => refreshRef.current()),
      TERMINAL_REFRESH_TIMEOUT_MS,
    );
  }, [publish, rememberJob, settleTerminalJob, updateStatus]);

  const runSummaryScan = useCallback(async (
    preparingMessage: string,
    request: () => Promise<ScanJobResult<ScanSummary>>,
  ) => {
    const requestToken = beginRequest(preparingMessage);
    if (requestToken === null) return;
    let keepTrackingAdoptedJob = false;
    try {
      const response = await request();
      if (
        !mountedRef.current ||
        activeRequestTokenRef.current !== requestToken
      ) {
        return;
      }
      void settleTerminalJob({
        jobId: response.jobId,
        status: response.status,
        message: response.message,
        summary: response.result,
      });
    } catch (error) {
      if (
        !mountedRef.current ||
        activeRequestTokenRef.current !== requestToken
      ) {
        return;
      }
      if (scanCommandErrorCode(error) === "already-running") {
        keepTrackingAdoptedJob = true;
        trackConflictingJob(error);
        setErrorMessage(`扫描互斥冲突：${commandErrorMessage(error)}`);
        publish({ kind: "warning", message: "已有扫描任务正在运行" });
      } else {
        handleFailedRequest(error, "扫描");
      }
    } finally {
      if (!keepTrackingAdoptedJob) finishRequest(undefined, requestToken);
    }
  }, [beginRequest, finishRequest, handleFailedRequest, publish, settleTerminalJob, trackConflictingJob]);

  const startScan = useCallback(() => {
    const paths = [...sourcePathsRef.current];
    return runSummaryScan(
      `准备扫描 ${paths.length || 1} 个目录`,
      () => paths.length === 0 ? scanDefaultAclosDir() : scanRoots(paths),
    );
  }, [runSummaryScan]);

  const syncSource = useCallback((sourceId: string) => (
    runSummaryScan("正在准备同步视频来源", () => requestSourceSync(sourceId))
  ), [runSummaryScan]);

  const syncEnabledSources = useCallback(() => (
    runSummaryScan("正在准备同步全部已启用来源", requestEnabledSourceSync)
  ), [runSummaryScan]);

  const discoverAll = useCallback(async () => {
    const requestToken = beginRequest("正在全电脑发现无畏时刻素材");
    if (requestToken === null) return;
    let keepTrackingAdoptedJob = false;
    try {
      const response = await discoverAndScanFixedDrives();
      if (
        !mountedRef.current ||
        activeRequestTokenRef.current !== requestToken
      ) {
        return;
      }
      void settleTerminalJob({
        jobId: response.jobId,
        status: response.status,
        message: response.message,
        summary: response.result?.scanSummary ?? null,
      });
    } catch (error) {
      if (
        !mountedRef.current ||
        activeRequestTokenRef.current !== requestToken
      ) {
        return;
      }
      if (scanCommandErrorCode(error) === "already-running") {
        keepTrackingAdoptedJob = true;
        trackConflictingJob(error);
        setErrorMessage(`扫描互斥冲突：${commandErrorMessage(error)}`);
        publish({ kind: "warning", message: "已有扫描任务正在运行" });
      } else {
        handleFailedRequest(error, "全电脑发现");
      }
    } finally {
      if (!keepTrackingAdoptedJob) finishRequest(undefined, requestToken);
    }
  }, [beginRequest, finishRequest, handleFailedRequest, publish, settleTerminalJob, trackConflictingJob]);

  const cancelScan = useCallback(async () => {
    const jobId = activeJobIdRef.current;
    if (!jobId || statusRef.current === "cancelling" || cancelRequestActiveRef.current) return;

    cancelRequestActiveRef.current = true;
    updateStatus("cancelling");
    setProgress((current) => {
      const nextProgress = current?.jobId === jobId
        ? { ...current, phase: "cancelling", status: "cancelling" as const, message: "正在取消扫描" }
        : current;
      latestProgressRef.current = nextProgress;
      return nextProgress;
    });
    publish({ kind: "info", message: "正在取消扫描" });
    try {
      const result = await requestScanCancellation(jobId);
      if (!mountedRef.current) return;
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
      if (!mountedRef.current) return;
      cancelRequestActiveRef.current = false;
      setErrorMessage(`取消扫描失败：${commandErrorMessage(error)}`);
      publish({ kind: "error", message: "取消请求失败，扫描仍在运行" });
      updateStatus("running");
    }
  }, [publish, updateStatus]);

  const settleExternalTerminal = useCallback(async (input: ExternalTerminalInput) => {
    const settlement = settleTerminalJob(input);
    if (!activeJobIdRef.current || activeJobIdRef.current === input.jobId) {
      finishRequest(input.jobId);
    }
    await settlement;
  }, [finishRequest, settleTerminalJob]);

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
    syncSource,
    syncEnabledSources,
    discoverAll,
    cancelScan,
    settleExternalTerminal,
    clearOutcome: () => {
      setSummary(null);
      setErrorMessage(null);
    },
    clearSummary: () => setSummary(null),
    reportError: setErrorMessage,
  };
}

function terminalProgress(
  jobId: string,
  status: ScanJobStatus,
  message: string,
  summary: ScanSummary | null,
  current: ScanProgress | null,
): ScanProgress {
  const currentJobProgress = current?.jobId === jobId ? current : null;
  const processed = summary?.sourceDirCount ?? currentJobProgress?.processed ?? 0;
  return {
    jobId,
    phase: status,
    currentRoot: summary?.rootPath || currentJobProgress?.currentRoot || null,
    source: currentJobProgress?.source ?? null,
    processed,
    total: processed > 0 ? processed : currentJobProgress?.total ?? null,
    terminal: true,
    status,
    sourceDirCount: summary?.sourceDirCount ?? currentJobProgress?.sourceDirCount ?? 0,
    clipGroupCount: summary?.clipGroupCount ?? currentJobProgress?.clipGroupCount ?? 0,
    clipFileCount: currentJobProgress?.clipFileCount ?? 0,
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

function addBounded(values: Set<string>, value: string, limit: number): void {
  values.delete(value);
  values.add(value);
  while (values.size > limit) {
    const oldest = values.values().next().value;
    if (typeof oldest !== "string") break;
    values.delete(oldest);
  }
}

function setBoundedMap<T>(values: Map<string, T>, key: string, value: T, limit: number): void {
  values.delete(key);
  values.set(key, value);
  while (values.size > limit) {
    const oldest = values.keys().next().value;
    if (typeof oldest !== "string") break;
    values.delete(oldest);
  }
}

type SettledWithin<T> =
  | { status: "fulfilled"; value: T }
  | { status: "rejected"; reason: unknown }
  | { status: "timed-out" };

async function settleWithin<T>(promise: Promise<T>, timeoutMs: number): Promise<SettledWithin<T>> {
  let timeoutId: ReturnType<typeof window.setTimeout> | undefined;
  const outcome = await Promise.race<SettledWithin<T>>([
    promise.then<SettledWithin<T>, SettledWithin<T>>(
      (value) => ({ status: "fulfilled", value }),
      (reason: unknown) => ({ status: "rejected", reason }),
    ),
    new Promise<SettledWithin<T>>((resolve) => {
      timeoutId = window.setTimeout(
        () => resolve({ status: "timed-out" }),
        timeoutMs,
      );
    }),
  ]);
  if (timeoutId !== undefined) window.clearTimeout(timeoutId);
  return outcome;
}
