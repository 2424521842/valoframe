import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CancelScanResult,
  ScanJobResult,
  ScanProgress,
  ScanSummary,
  SourceDir,
} from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  cancelScan: vi.fn(),
  getScanStatus: vi.fn(),
  getLibraryFacets: vi.fn(),
  listClips: vi.fn(),
  listClipPage: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  scanDefaultAclosDir: vi.fn(),
  scanRoots: vi.fn(),
  progressListener: null as ((progress: ScanProgress) => void) | null,
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    cancelScan: mocks.cancelScan,
    getScanStatus: mocks.getScanStatus,
    getLibraryFacets: mocks.getLibraryFacets,
    listClips: mocks.listClips,
    listClipPage: mocks.listClipPage,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    scanDefaultAclosDir: mocks.scanDefaultAclosDir,
    scanRoots: mocks.scanRoots,
    listenToScanProgress: vi.fn(async (listener: (progress: ScanProgress) => void) => {
      mocks.progressListener = listener;
      return () => {
        if (mocks.progressListener === listener) {
          mocks.progressListener = null;
        }
      };
    }),
  };
});

import App from "../../src/App";

const sourceDirs: SourceDir[] = [
  source("1", "D:\\ArchiveA\\wonderfulVideos1001"),
  source("2", "D:\\ArchiveB\\wonderfulVideos2002"),
];

describe("production scan lifecycle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.cancelScan.mockReset();
    mocks.getScanStatus.mockReset();
    mocks.getLibraryFacets.mockReset();
    mocks.listClips.mockReset();
    mocks.listClipPage.mockReset();
    mocks.listSources.mockReset();
    mocks.listTags.mockReset();
    mocks.scanDefaultAclosDir.mockReset();
    mocks.scanRoots.mockReset();
    mocks.progressListener = null;
    mocks.listClipPage.mockResolvedValue({
      items: [],
      offset: 0,
      limit: 50,
      totalCount: 0,
      hasMore: false,
      nextOffset: null,
    });
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets());
    mocks.listSources.mockResolvedValue(sourceDirs);
    mocks.listTags.mockResolvedValue([]);
    mocks.scanDefaultAclosDir.mockResolvedValue(jobResult("default", "completed"));
    mocks.cancelScan.mockResolvedValue(cancelResult("scan-active"));
    mocks.getScanStatus.mockResolvedValue({
      jobId: null,
      phase: null,
      currentRoot: null,
      source: null,
      processed: 0,
      total: null,
      message: "当前没有扫描任务",
      terminal: false,
      status: "idle",
    });
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    Reflect.deleteProperty(globalThis, "isTauri");
  });

  it("scans multiple roots once, disables duplicate starts, and exposes cancelling state", async () => {
    const user = userEvent.setup();
    const scan = deferred<ScanJobResult<ScanSummary>>();
    const cancel = deferred<CancelScanResult>();
    mocks.scanRoots.mockReturnValueOnce(scan.promise);
    mocks.cancelScan.mockReturnValueOnce(cancel.promise);
    render(<App />);
    await openScanWorkspace(user);
    const loadsBeforeScan = collectionLoadCounts();

    const start = screen.getByRole("button", { name: "开始扫描" });
    await user.click(start);
    expect(mocks.scanRoots).toHaveBeenCalledTimes(1);
    expect(mocks.scanRoots).toHaveBeenCalledWith(["D:\\ArchiveA", "D:\\ArchiveB"]);
    expect(screen.getByRole("button", { name: "正在扫描" })).toBeDisabled();

    emitProgress(progress("scan-active", "正在扫描第一个来源"));
    const cancelButton = await screen.findByRole("button", { name: "取消扫描" });
    expect(cancelButton).toBeEnabled();
    await user.click(cancelButton);
    expect(mocks.cancelScan).toHaveBeenCalledWith("scan-active");
    expect(screen.getByRole("button", { name: "正在取消" })).toBeDisabled();

    cancel.resolve(cancelResult("scan-active"));
    scan.resolve(jobResult("scan-active", "cancelled"));
    await screen.findByText(/扫描已取消；取消前已安全写入的索引已保留/);
    await expectCollectionsRefreshed(loadsBeforeScan);
    expect(mocks.scanRoots).toHaveBeenCalledTimes(1);
  });

  it("ignores delayed progress from an old job while a new scan is active", async () => {
    const user = userEvent.setup();
    const first = deferred<ScanJobResult<ScanSummary>>();
    const second = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render(<App />);
    await openScanWorkspace(user);
    const loadsBeforeCompletedScan = collectionLoadCounts();

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    emitProgress(progress("scan-old", "旧任务进行中"));
    first.resolve(jobResult("scan-old", "completed"));
    await waitFor(() => expect(screen.getByRole("button", { name: "开始扫描" })).toBeEnabled());
    expect(screen.getAllByText(/扫描完成/).length).toBeGreaterThan(0);
    await expectCollectionsRefreshed(loadsBeforeCompletedScan);

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    emitProgress(progress("scan-new", "新任务进度"));
    emitProgress(progress("scan-old", "旧任务迟到事件"));
    expect(screen.getAllByText("新任务进度").length).toBeGreaterThan(0);
    expect(screen.queryByText("旧任务迟到事件")).not.toBeInTheDocument();

    second.resolve(jobResult("scan-new", "completed"));
    await waitFor(() => expect(mocks.scanRoots).toHaveBeenCalledTimes(2));
  });

  it("recovers the active job id when the starting progress event is missed", async () => {
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    const user = userEvent.setup();
    const scan = deferred<ScanJobResult<ScanSummary>>();
    mocks.scanRoots.mockReturnValueOnce(scan.promise);
    mocks.getScanStatus.mockResolvedValue({
      jobId: "scan-recovered",
      phase: "scanning",
      currentRoot: "D:\\ArchiveA",
      source: null,
      processed: 0,
      total: null,
      message: "已恢复扫描状态",
      terminal: false,
      status: "running",
    });
    render(<App />);
    await openScanWorkspace(user);

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    const cancelButton = await screen.findByRole("button", { name: "取消扫描" });
    await waitFor(() => expect(cancelButton).toBeEnabled());
    await user.click(cancelButton);
    expect(mocks.cancelScan).toHaveBeenCalledWith("scan-recovered");

    scan.resolve(jobResult("scan-recovered", "cancelled"));
    await screen.findByText(/扫描已取消；取消前已安全写入的索引已保留/);
  });

  it("shows a partial result and refreshes all indexed collections", async () => {
    const user = userEvent.setup();
    mocks.scanRoots.mockResolvedValueOnce(jobResult("scan-partial", "partial"));
    render(<App />);
    await openScanWorkspace(user);
    const loadsBeforeScan = collectionLoadCounts();

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    const partialAlert = await screen.findByRole("alert");
    expect(partialAlert).toHaveTextContent(/扫描部分完成/);
    await expectCollectionsRefreshed(loadsBeforeScan);
  });

  it("distinguishes ordinary failure from an already-running conflict", async () => {
    const user = userEvent.setup();
    mocks.scanRoots.mockRejectedValueOnce({
      code: "scan-failed",
      message: "磁盘读取失败",
      jobId: "scan-failed",
    });
    render(<App />);
    await openScanWorkspace(user);
    const loadsBeforeFailure = collectionLoadCounts();

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    const failedAlert = await screen.findByRole("alert");
    expect(failedAlert).toHaveTextContent(/扫描失败：磁盘读取失败/);
    await expectCollectionsRefreshed(loadsBeforeFailure);

    mocks.scanRoots.mockRejectedValueOnce({
      code: "already-running",
      message: "已有扫描任务正在运行：scan-other",
      activeJobId: "scan-other",
    });
    const loadsBeforeConflict = collectionLoadCounts();
    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(
        /扫描互斥冲突：已有扫描任务正在运行/,
      );
    });
    expect(collectionLoadCounts()).toEqual(loadsBeforeConflict);
  });
});

async function openScanWorkspace(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => expect(mocks.listSources).toHaveBeenCalled());
  await user.click(screen.getByRole("button", { name: "扫描目录" }));
  await screen.findByRole("heading", { name: "扫描战术影像" });
}

function source(id: string, path: string): SourceDir {
  return {
    id,
    name: `来源 ${id}`,
    displayName: `来源 ${id}`,
    path,
    enabled: true,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 1,
    lastScanAt: null,
  };
}

function summary(status: "completed" | "partial" | "cancelled"): ScanSummary {
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
    metadataWarningCount: status === "partial" ? 1 : 0,
    errors: status === "partial" ? ["一个来源不可访问"] : [],
    message: status,
  };
}

function jobResult(
  jobId: string,
  status: "completed" | "partial" | "cancelled",
): ScanJobResult<ScanSummary> {
  return {
    jobId,
    status,
    result: summary(status),
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
    message,
    terminal: false,
    status: "running",
    sourceDirCount: 1,
    clipGroupCount: 1,
    clipFileCount: 1,
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
  act(() => {
    mocks.progressListener?.(value);
  });
}

function collectionLoadCounts() {
  return {
    clips: mocks.listClipPage.mock.calls.length,
    facets: mocks.getLibraryFacets.mock.calls.length,
    sources: mocks.listSources.mock.calls.length,
    tags: mocks.listTags.mock.calls.length,
  };
}

async function expectCollectionsRefreshed(previous: ReturnType<typeof collectionLoadCounts>) {
  await waitFor(() => {
    const current = collectionLoadCounts();
    expect(current.clips).toBeGreaterThan(previous.clips);
    expect(current.facets).toBeGreaterThan(previous.facets);
    expect(current.sources).toBeGreaterThan(previous.sources);
    expect(current.tags).toBeGreaterThan(previous.tags);
  });
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
