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
  isTauri: false,
  listeners: new Set<ProgressListener>(),
  listenToScanProgress: vi.fn(),
  scanDefaultAclosDir: vi.fn(),
  scanRoots: vi.fn(),
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
  listenToScanProgress: mocks.listenToScanProgress,
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
}));

import { useScanController } from "../../src/hooks/useScanController";

describe("useScanController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.cancelScan.mockReset();
    mocks.discoverAndScanFixedDrives.mockReset();
    mocks.getScanStatus.mockReset();
    mocks.listenToScanProgress.mockReset();
    mocks.scanDefaultAclosDir.mockReset();
    mocks.scanRoots.mockReset();
    mocks.isTauri = false;
    mocks.listeners.clear();
    mocks.unlisteners.length = 0;

    mocks.cancelScan.mockResolvedValue(cancelResult("scan-current"));
    mocks.getScanStatus.mockResolvedValue(idleStatus());
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
});

function renderController(reactStrictMode = false) {
  const refresh = vi.fn(async () => true);
  const notify = vi.fn();
  return renderHook(
    () => useScanController({
      sourcePaths: ["D:\\ArchiveA", "D:\\ArchiveB"],
      refresh,
      notify,
    }),
    { reactStrictMode },
  );
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
