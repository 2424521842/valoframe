import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CancelScanResult,
  ScanJobResult,
  ScanProgress,
  ScanStatus,
  ScanSummary,
} from "../../src/types";

type ProgressListener = (progress: ScanProgress) => void;

const mocks = vi.hoisted(() => ({
  cancelScan: vi.fn(),
  discoverAndScanFixedDrives: vi.fn(),
  getScanStatus: vi.fn(),
  getScanSummary: vi.fn(),
  isTauri: false,
  listeners: new Set<ProgressListener>(),
  listenToScanProgress: vi.fn(),
  scanDefaultAclosDir: vi.fn(),
  scanRoots: vi.fn(),
  syncEnabledSources: vi.fn(),
  syncScanSource: vi.fn(),
  unlisteners: [] as Array<ReturnType<typeof vi.fn>>,
}));

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => mocks.isTauri,
}));

vi.mock("../../src/api/backend", () => ({
  cancelScan: mocks.cancelScan,
  commandErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  discoverAndScanFixedDrives: mocks.discoverAndScanFixedDrives,
  getScanStatus: mocks.getScanStatus,
  getScanSummary: mocks.getScanSummary,
  listenToScanProgress: mocks.listenToScanProgress,
  scanCommandErrorActiveJobId: (error: unknown) => {
    if (typeof error !== "object" || error === null || !("activeJobId" in error)) {
      return null;
    }
    return typeof error.activeJobId === "string" ? error.activeJobId : null;
  },
  scanCommandErrorCode: (error: unknown) =>
    typeof error === "object" && error !== null && "code" in error
      ? String(error.code)
      : null,
  scanCommandErrorJobId: (error: unknown) =>
    typeof error === "object" && error !== null && "jobId" in error
      ? String(error.jobId)
      : null,
  scanDefaultAclosDir: mocks.scanDefaultAclosDir,
  scanRoots: mocks.scanRoots,
  syncEnabledSources: mocks.syncEnabledSources,
  syncScanSource: mocks.syncScanSource,
}));

import { useScanController } from "../../src/hooks/useScanController";

describe("useScanController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.cancelScan.mockReset();
    mocks.discoverAndScanFixedDrives.mockReset();
    mocks.getScanStatus.mockReset();
    mocks.getScanSummary.mockReset();
    mocks.listenToScanProgress.mockReset();
    mocks.scanDefaultAclosDir.mockReset();
    mocks.scanRoots.mockReset();
    mocks.syncEnabledSources.mockReset();
    mocks.syncScanSource.mockReset();
    mocks.isTauri = false;
    mocks.listeners.clear();
    mocks.unlisteners.length = 0;

    mocks.cancelScan.mockResolvedValue(cancelResult("scan-current"));
    mocks.getScanStatus.mockResolvedValue(idleStatus());
    mocks.getScanSummary.mockResolvedValue(summary());
    mocks.listenToScanProgress.mockImplementation(async (listener: ProgressListener) => {
      mocks.listeners.add(listener);
      const unlisten = vi.fn(() => {
        mocks.listeners.delete(listener);
      });
      mocks.unlisteners.push(unlisten);
      return unlisten;
    });
    mocks.scanDefaultAclosDir.mockResolvedValue(jobResult("scan-default", "completed"));
    mocks.scanRoots.mockResolvedValue(jobResult("scan-current", "completed"));
    mocks.syncEnabledSources.mockResolvedValue(jobResult("sync-enabled", "completed"));
    mocks.syncScanSource.mockResolvedValue(jobResult("sync-source", "completed"));
  });

  it("creates one progress listener and cleans it up on unmount", async () => {
    const controller = renderController();

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    expect(mocks.listenToScanProgress).toHaveBeenCalledTimes(1);

    controller.unmount();

    await waitFor(() => expect(mocks.listeners.size).toBe(0));
    expect(mocks.unlisteners).toHaveLength(1);
    expect(mocks.unlisteners[0]).toHaveBeenCalledTimes(1);
  });

  it("suppresses duplicate scan starts while a request is active", async () => {
    const scan = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValue(scan.promise);
    const { result } = renderController();
    let firstRequest!: Promise<void>;

    act(() => {
      firstRequest = result.current.startScan();
      void result.current.startScan();
    });

    expect(mocks.scanRoots).toHaveBeenCalledTimes(1);
    expect(result.current.isScanning).toBe(true);

    await act(async () => {
      scan.resolve(jobResult("scan-current", "completed"));
      await firstRequest;
    });
    expect(result.current.isScanning).toBe(false);
  });

  it("routes single-source synchronization through the shared scan lifecycle", async () => {
    const { result, refresh } = renderController();

    await act(async () => {
      await result.current.syncSource("42");
    });

    expect(mocks.syncScanSource).toHaveBeenCalledWith("42");
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("completed");
    expect(result.current.summary).toEqual(summary());
  });

  it("synchronizes all enabled sources as one shared scan job", async () => {
    const { result, refresh } = renderController();

    await act(async () => {
      await result.current.syncEnabledSources();
    });

    expect(mocks.syncEnabledSources).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("completed");
  });

  it("adopts backend startup synchronization and refreshes at its terminal event", async () => {
    const { result, refresh } = renderController();
    await waitFor(() => expect(mocks.listeners.size).toBe(1));

    act(() => {
      emitProgress(progress("startup-sync", "正在后台同步启用来源"));
    });
    expect(result.current.isScanning).toBe(true);
    expect(result.current.activeJobId).toBe("startup-sync");

    act(() => {
      emitProgress(terminalScanProgress("startup-sync", "completed", "启动同步完成"));
    });
    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("completed");
  });

  it("recovers a startup synchronization that finished before event subscription", async () => {
    mocks.isTauri = true;
    mocks.getScanStatus.mockResolvedValue(
      terminalStatus("startup-finished", "completed", "启动同步已完成"),
    );
    const { result, refresh } = renderController();

    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(result.current.status).toBe("completed");
    expect(result.current.progress?.jobId).toBe("startup-finished");
  });

  it("recovers the active job id when the first progress event is missed", async () => {
    mocks.isTauri = true;
    const scan = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValue(scan.promise);
    mocks.getScanStatus.mockResolvedValue(runningStatus("scan-recovered"));
    const { result } = renderController();
    let request!: Promise<void>;

    act(() => {
      request = result.current.startScan();
    });

    await waitFor(() => expect(result.current.activeJobId).toBe("scan-recovered"));
    expect(result.current.progress?.message).toBe("已恢复扫描状态");

    await act(async () => {
      scan.resolve(jobResult("scan-recovered", "completed"));
      await request;
    });
  });

  it("settles a locally requested job from polling when its invoke never resolves", async () => {
    mocks.isTauri = true;
    const lostInvoke = deferred<ScanJobResult<ScanSummary>>();
    const nextInvoke = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots
      .mockReturnValueOnce(lostInvoke.promise)
      .mockReturnValueOnce(nextInvoke.promise);
    mocks.getScanStatus.mockResolvedValue(runningStatus("scan-lost-invoke"));
    const { result, refresh } = renderController();
    let lostRequest!: Promise<void>;
    let nextRequest!: Promise<void>;

    act(() => {
      lostRequest = result.current.startScan();
    });
    await waitFor(() => expect(result.current.activeJobId).toBe("scan-lost-invoke"));

    mocks.getScanStatus.mockResolvedValue(
      terminalStatus("scan-lost-invoke", "completed", "轮询恢复终态"),
    );
    await waitFor(() => expect(result.current.isScanning).toBe(false), { timeout: 1_500 });
    expect(result.current.activeJobId).toBeNull();
    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));

    act(() => {
      nextRequest = result.current.startScan();
      emitProgress(progress("scan-after-lost-invoke", "新任务正在扫描"));
    });
    expect(result.current.isScanning).toBe(true);
    expect(result.current.activeJobId).toBe("scan-after-lost-invoke");

    await act(async () => {
      lostInvoke.resolve(jobResult("scan-lost-invoke", "completed"));
      await lostRequest;
    });
    expect(result.current.isScanning).toBe(true);
    expect(result.current.activeJobId).toBe("scan-after-lost-invoke");
    expect(result.current.progress?.message).toBe("新任务正在扫描");

    await act(async () => {
      nextInvoke.resolve(jobResult("scan-after-lost-invoke", "completed"));
      await nextRequest;
    });
    expect(result.current.isScanning).toBe(false);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("ignores delayed progress from a stale job while a new job is active", async () => {
    const first = deferred<ScanJobResult<ScanSummary>>();
    const second = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result } = renderController();
    let firstRequest!: Promise<void>;
    let secondRequest!: Promise<void>;

    act(() => {
      firstRequest = result.current.startScan();
      emitProgress(progress("scan-old", "旧任务进行中"));
    });
    await act(async () => {
      first.resolve(jobResult("scan-old", "completed"));
      await firstRequest;
    });

    act(() => {
      secondRequest = result.current.startScan();
      emitProgress(progress("scan-new", "新任务进度"));
      emitProgress(progress("scan-old", "旧任务迟到事件"));
    });

    expect(result.current.activeJobId).toBe("scan-new");
    expect(result.current.progress?.message).toBe("新任务进度");

    await act(async () => {
      second.resolve(jobResult("scan-new", "completed"));
      await secondRequest;
    });
  });

  it("keeps tracking a running job when progress arrives before the conflict response", async () => {
    const scan = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValue(scan.promise);
    const { result, refresh } = renderController();
    let request!: Promise<void>;

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    act(() => {
      request = result.current.startScan();
      emitProgress(progress("scan-existing", "冲突响应前的进度"));
    });

    await act(async () => {
      scan.reject(alreadyRunningError("scan-existing"));
      await request;
    });

    expect(result.current.isScanning).toBe(true);
    expect(result.current.activeJobId).toBe("scan-existing");
    expect(result.current.progress?.message).toBe("冲突响应前的进度");
    expect(refresh).not.toHaveBeenCalled();

    act(() => {
      emitProgress(progress("scan-existing", "冲突后的后续进度"));
      emitProgress(terminalScanProgress("scan-existing", "completed", "后台扫描完成"));
      emitProgress(terminalScanProgress("scan-existing", "completed", "重复的终态事件"));
    });

    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("completed");
    expect(result.current.errorMessage).toBeNull();
  });

  it("settles an already-running job from status recovery when no later event arrives", async () => {
    mocks.isTauri = true;
    mocks.scanRoots.mockRejectedValueOnce(alreadyRunningError("scan-recovered"));
    mocks.getScanStatus.mockResolvedValue(runningStatus("scan-recovered"));
    const { result, refresh } = renderController();

    await act(async () => {
      await result.current.startScan();
    });

    expect(result.current.isScanning).toBe(true);
    expect(result.current.activeJobId).toBe("scan-recovered");

    mocks.getScanStatus.mockResolvedValue(
      terminalStatus("scan-recovered", "completed", "后台扫描完成"),
    );

    await waitFor(() => expect(result.current.isScanning).toBe(false), { timeout: 1_500 });
    expect(refresh).toHaveBeenCalledTimes(1);

    act(() => {
      emitProgress(terminalScanProgress("scan-recovered", "completed", "迟到的终态事件"));
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("releases a recovered terminal state without waiting for a hanging refresh", async () => {
    mocks.isTauri = true;
    mocks.scanRoots.mockRejectedValueOnce(alreadyRunningError("scan-recovered"));
    mocks.getScanStatus.mockResolvedValue(runningStatus("scan-recovered"));
    const refreshResult = deferred<boolean>();
    const { result, refresh } = renderController();
    refresh.mockReturnValue(refreshResult.promise);

    await act(async () => {
      await result.current.startScan();
    });
    mocks.getScanStatus.mockResolvedValue(
      terminalStatus("scan-recovered", "completed", "后台扫描完成"),
    );
    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1), { timeout: 1_500 });
    expect(result.current.isScanning).toBe(false);
    expect(result.current.activeJobId).toBeNull();

    act(() => {
      emitProgress(progress("scan-recovered", "迟到的运行中事件"));
    });
    expect(result.current.status).toBe("completed");
    expect(result.current.progress?.terminal).toBe(true);

    await act(async () => {
      refreshResult.resolve(true);
      await refreshResult.promise;
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("preserves an in-flight cancellation while adopting a conflicting job", async () => {
    const scan = deferred<ScanJobResult<ScanSummary>>();
    const cancellation = deferred<CancelScanResult>();
    mocks.scanRoots.mockReturnValue(scan.promise);
    mocks.cancelScan.mockReturnValue(cancellation.promise);
    const { result, refresh } = renderController();
    let scanRequest!: Promise<void>;
    let cancelRequest!: Promise<void>;

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    act(() => {
      scanRequest = result.current.startScan();
      emitProgress(progress("scan-existing", "已有任务正在扫描"));
      cancelRequest = result.current.cancelScan();
    });
    expect(mocks.cancelScan).toHaveBeenCalledTimes(1);

    await act(async () => {
      scan.reject(alreadyRunningError("scan-existing"));
      await scanRequest;
    });
    act(() => {
      emitProgress(progress("scan-existing", "迟到的运行中事件"));
      void result.current.cancelScan();
    });
    expect(result.current.status).toBe("cancelling");
    expect(mocks.cancelScan).toHaveBeenCalledTimes(1);

    await act(async () => {
      cancellation.resolve(cancelResult("scan-existing"));
      await cancelRequest;
    });
    act(() => {
      emitProgress(terminalScanProgress("scan-existing", "cancelled", "后台扫描已取消"));
    });
    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("keeps cancellation monotonic when an adopted job has no progress snapshot", async () => {
    mocks.scanRoots.mockRejectedValueOnce(alreadyRunningError("scan-existing"));
    const { result } = renderController();

    await act(async () => {
      await result.current.startScan();
    });
    expect(result.current.activeJobId).toBe("scan-existing");
    expect(result.current.progress).toBeNull();

    await act(async () => {
      await result.current.cancelScan();
    });
    act(() => {
      emitProgress(progress("scan-existing", "迟到的运行中事件"));
      void result.current.cancelScan();
    });
    expect(result.current.status).toBe("cancelling");
    expect(mocks.cancelScan).toHaveBeenCalledTimes(1);

    act(() => {
      emitProgress(terminalScanProgress("scan-existing", "cancelled", "后台扫描已取消"));
    });
    await waitFor(() => expect(result.current.isScanning).toBe(false));
  });

  it("enters cancelling and cancels only the current job", async () => {
    const scan = deferred<ScanJobResult<ScanSummary>>();
    const cancellation = deferred<CancelScanResult>();
    mocks.scanRoots.mockReturnValue(scan.promise);
    mocks.cancelScan.mockReturnValue(cancellation.promise);
    const { result } = renderController();
    let scanRequest!: Promise<void>;
    let cancelRequest!: Promise<void>;

    act(() => {
      scanRequest = result.current.startScan();
      emitProgress(progress("scan-current", "正在扫描当前任务"));
      cancelRequest = result.current.cancelScan();
      void result.current.cancelScan();
    });

    expect(result.current.status).toBe("cancelling");
    expect(result.current.progress?.message).toBe("正在取消扫描");
    expect(mocks.cancelScan).toHaveBeenCalledTimes(1);
    expect(mocks.cancelScan).toHaveBeenCalledWith("scan-current");

    await act(async () => {
      cancellation.resolve(cancelResult("scan-current"));
      await cancelRequest;
      scan.resolve(jobResult("scan-current", "cancelled"));
      await scanRequest;
    });
  });

  it("clears active run state after a request settles", async () => {
    const { result } = renderController();

    await act(async () => {
      await result.current.startScan();
    });

    expect(result.current.activeJobId).toBeNull();
    expect(result.current.isScanning).toBe(false);
    expect(result.current.status).toBe("completed");
    expect(result.current.progress?.terminal).toBe(true);
  });

  it("keeps one effective listener in StrictMode and cleans up every subscription", async () => {
    const controller = renderController(true);

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    expect(mocks.listenToScanProgress.mock.calls.length).toBeGreaterThan(1);
    expect(mocks.unlisteners).toHaveLength(mocks.listenToScanProgress.mock.calls.length);
    expect(mocks.unlisteners.filter((unlisten) => unlisten.mock.calls.length === 0)).toHaveLength(1);

    controller.unmount();

    await waitFor(() => expect(mocks.listeners.size).toBe(0));
    for (const unlisten of mocks.unlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  });

  it("deduplicates the terminal event and promise result by job id", async () => {
    const scan = deferred<ScanJobResult<ScanSummary>>();
    const zeroSummary = { ...summary(), newClipCount: 0 };
    mocks.scanRoots.mockReturnValue(scan.promise);
    mocks.getScanSummary.mockResolvedValue(zeroSummary);
    const { result, refresh, notify } = renderController();
    let request!: Promise<void>;

    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    act(() => {
      request = result.current.startScan();
      emitProgress(progress("scan-double", "正在扫描"));
      emitProgress(terminalScanProgress("scan-double", "completed", "完成"));
    });
    await waitFor(() => expect(notify).toHaveBeenCalledWith({
      kind: "success",
      message: "扫描完成：新增 0 个视频",
    }));

    await act(async () => {
      scan.resolve({
        jobId: "scan-double",
        status: "completed",
        result: zeroSummary,
        message: "完成",
      });
      await request;
    });

    expect(mocks.getScanSummary).toHaveBeenCalledWith("scan-double");
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(notify.mock.calls.filter((call) => (
      call[0].message === "扫描完成：新增 0 个视频"
    ))).toHaveLength(1);
  });

  it("recovers an adopted job summary by exact job id", async () => {
    const recovered = { ...summary(), newClipCount: 4 };
    mocks.getScanSummary.mockResolvedValue(recovered);
    const { result, notify } = renderController();
    await waitFor(() => expect(mocks.listeners.size).toBe(1));

    act(() => {
      emitProgress(terminalScanProgress("startup-exact", "partial", "部分完成"));
    });

    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(mocks.getScanSummary).toHaveBeenCalledWith("startup-exact");
    expect(notify).toHaveBeenCalledWith({
      kind: "warning",
      message: "扫描部分完成：已安全新增 4 个视频",
    });
  });

  it("does not guess zero when an adopted job has no persisted summary", async () => {
    mocks.getScanSummary.mockResolvedValue(null);
    const { result, notify } = renderController();
    await waitFor(() => expect(mocks.listeners.size).toBe(1));

    act(() => {
      emitProgress(terminalScanProgress("startup-unknown", "cancelled", "已取消"));
    });

    await waitFor(() => expect(result.current.isScanning).toBe(false));
    expect(result.current.summary).toBeNull();
    expect(notify).toHaveBeenCalledWith({
      kind: "warning",
      message: "扫描已取消：新增数量不可用",
    });
  });

  it("recovers safe additions for a failed command before refreshing", async () => {
    mocks.scanRoots.mockRejectedValueOnce({
      code: "scan-failed",
      message: "磁盘暂时不可读",
      jobId: "scan-failed-summary",
    });
    mocks.getScanSummary.mockResolvedValue({ ...summary(), newClipCount: 2 });
    const { result, refresh, notify } = renderController();

    await act(async () => {
      await result.current.startScan();
    });

    expect(result.current.status).toBe("failed");
    expect(mocks.getScanSummary).toHaveBeenCalledWith("scan-failed-summary");
    expect(notify).toHaveBeenCalledWith({
      kind: "error",
      message: "扫描失败：已安全新增 2 个视频",
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("settles a relocation follow-up terminal by exact job id when all events were missed", async () => {
    const recovered = { ...summary(), newClipCount: 7 };
    mocks.getScanSummary.mockResolvedValueOnce(recovered);
    const { result, refresh, notify } = renderController();

    await act(async () => {
      await result.current.settleExternalTerminal({
        jobId: "relocation-missed-events",
        status: "completed",
        message: "重新定位后的同步完成",
      });
    });

    expect(mocks.getScanSummary).toHaveBeenCalledWith("relocation-missed-events");
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenCalledWith({
      kind: "success",
      message: "扫描完成：新增 7 个视频",
    });
    expect(result.current.isScanning).toBe(false);
    expect(result.current.status).toBe("completed");
  });

  it("deduplicates an external terminal settlement against the same live event", async () => {
    const { result, refresh, notify } = renderController();
    await waitFor(() => expect(mocks.listeners.size).toBe(1));
    act(() => emitProgress(terminalScanProgress("relocation-dedup", "partial", "部分完成")));
    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.settleExternalTerminal({
        jobId: "relocation-dedup",
        status: "partial",
        message: "部分完成",
      });
    });

    expect(refresh).toHaveBeenCalledTimes(1);
    expect(notify.mock.calls.filter((call) => (
      call[0].message === "扫描部分完成：已安全新增 1 个视频"
    ))).toHaveLength(1);
  });

  it("keeps the exact terminal count in the unified refresh failure message", async () => {
    const { result, refresh, notify } = renderController();
    refresh.mockResolvedValueOnce(false);

    await act(async () => {
      await result.current.startScan();
    });

    expect(notify).toHaveBeenCalledWith({
      kind: "warning",
      message: "扫描完成：新增 1 个视频；终态已确定，但刷新索引视图失败",
    });
    expect(result.current.errorMessage).toBe(
      "扫描完成：新增 1 个视频；终态已确定，但刷新索引视图失败",
    );
  });
});

function renderController(reactStrictMode = false) {
  const refresh = vi.fn(async () => true);
  const notify = vi.fn();
  const controller = renderHook(
    () => useScanController({
      sourcePaths: ["D:\\ArchiveA", "D:\\ArchiveB"],
      refresh,
      notify,
    }),
    { reactStrictMode },
  );
  return { ...controller, refresh, notify };
}

function summary(): ScanSummary {
  return {
    rootPath: "D:\\ArchiveA; D:\\ArchiveB",
    sourceDirCount: 2,
    clipGroupCount: 2,
    newClipCount: 1,
    updatedClipCount: 1,
    missingClipCount: 0,
    coverMissingCount: 0,
    metadataMatchCount: 0,
    metadataEnrichedClipCount: 0,
    metadataEventCount: 0,
    metadataWarningCount: 0,
    errors: [],
    message: "completed",
  };
}

function jobResult(
  jobId: string,
  status: "completed" | "cancelled",
): ScanJobResult<ScanSummary> {
  return {
    jobId,
    status,
    result: summary(),
    message: status,
  };
}

function progress(jobId: string, message: string): ScanProgress {
  return {
    jobId,
    phase: "scanning",
    currentRoot: "D:\\ArchiveA",
    source: "D:\\ArchiveA\\wonderfulVideos1001",
    processed: 1,
    total: 2,
    terminal: false,
    status: "running",
    sourceDirCount: 1,
    clipGroupCount: 1,
    clipFileCount: 1,
    message,
  };
}

function idleStatus(): ScanStatus {
  return {
    jobId: null,
    phase: null,
    currentRoot: null,
    source: null,
    processed: 0,
    total: null,
    terminal: false,
    status: "idle",
    message: "当前没有扫描任务",
  };
}

function runningStatus(jobId: string): ScanStatus {
  return {
    jobId,
    phase: "scanning",
    currentRoot: "D:\\ArchiveA",
    source: null,
    processed: 0,
    total: null,
    terminal: false,
    status: "running",
    message: "已恢复扫描状态",
  };
}

function terminalStatus(
  jobId: string,
  status: "completed" | "partial" | "cancelled" | "failed",
  message: string,
): ScanStatus {
  return {
    jobId,
    phase: status,
    currentRoot: "D:\\ArchiveA",
    source: null,
    processed: 2,
    total: 2,
    terminal: true,
    status,
    message,
  };
}

function terminalScanProgress(
  jobId: string,
  status: "completed" | "partial" | "cancelled" | "failed",
  message: string,
): ScanProgress {
  return {
    ...progress(jobId, message),
    phase: status,
    processed: 2,
    terminal: true,
    status,
  };
}

function alreadyRunningError(jobId: string) {
  return {
    code: "already-running",
    message: `已有扫描任务正在运行：${jobId}`,
    activeJobId: jobId,
  };
}

function cancelResult(jobId: string): CancelScanResult {
  return {
    accepted: true,
    reason: "accepted",
    jobId,
    activeJobId: jobId,
    status: "cancelling",
    message: "正在取消扫描",
  };
}

function emitProgress(value: ScanProgress) {
  for (const listener of mocks.listeners) {
    listener(value);
  }
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
