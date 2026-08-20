import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppUpdaterController } from "../../src/hooks/useAppUpdaterController";
import {
  APP_PREFERENCES_STORAGE_KEY,
  DEFAULT_APP_PREFERENCES,
} from "../../src/lib/appPreferences";
import type {
  CancelScanResult,
  ScanJobResult,
  ScanProgress,
  ScanSourceRelocationPreview,
  ScanSummary,
  SourceDir,
} from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  cancelScan: vi.fn(),
  getScanStatus: vi.fn(),
  getScanSummary: vi.fn(),
  getLibraryFacets: vi.fn(),
  listClips: vi.fn(),
  listClipPage: vi.fn(),
  listPendingManualClips: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  openDirectory: vi.fn(),
  previewScanSourceRelocation: vi.fn(),
  requestStartupSourceSync: vi.fn(),
  relocateScanSource: vi.fn(),
  scanDefaultAclosDir: vi.fn(),
  scanRoots: vi.fn(),
  updaterOptions: vi.fn(),
  progressListener: null as ((progress: ScanProgress) => void) | null,
  appUpdater: {
    runtimeInfo: {
      currentVersion: "0.2.1",
      channel: "stable",
      endpoint: "https://github.com/2424521842/valoframe/releases/latest/download/latest.json",
      configured: true,
    },
    runtimeStatus: "ready",
    runtimeError: null,
    phase: "idle",
    update: null,
    progress: { downloadedBytes: 0, totalBytes: null },
    message: "更新检查尚未运行",
    error: null,
    canCheck: true,
    canDownload: false,
    canCancelDownload: false,
    canInstall: false,
    refreshRuntimeInfo: vi.fn(async () => undefined),
    checkManually: vi.fn(async () => undefined),
    download: vi.fn(async () => undefined),
    cancelDownload: vi.fn(async () => undefined),
    installAndRestart: vi.fn(async () => undefined),
  } as AppUpdaterController,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.openDirectory,
}));

vi.mock("../../src/hooks/useAppUpdaterController", () => ({
  useAppUpdaterController: (options: unknown) => {
    mocks.updaterOptions(options);
    return mocks.appUpdater;
  },
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    cancelScan: mocks.cancelScan,
    getScanStatus: mocks.getScanStatus,
    getScanSummary: mocks.getScanSummary,
    getLibraryFacets: mocks.getLibraryFacets,
    listClips: mocks.listClips,
    listClipPage: mocks.listClipPage,
    listPendingManualClips: mocks.listPendingManualClips,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    previewScanSourceRelocation: mocks.previewScanSourceRelocation,
    requestStartupSourceSync: mocks.requestStartupSourceSync,
    relocateScanSource: mocks.relocateScanSource,
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
    resetAppUpdater();
    mocks.cancelScan.mockReset();
    mocks.getScanStatus.mockReset();
    mocks.getScanSummary.mockReset();
    mocks.getLibraryFacets.mockReset();
    mocks.listClips.mockReset();
    mocks.listClipPage.mockReset();
    mocks.listPendingManualClips.mockReset();
    mocks.listSources.mockReset();
    mocks.listTags.mockReset();
    mocks.openDirectory.mockReset();
    mocks.previewScanSourceRelocation.mockReset();
    mocks.requestStartupSourceSync.mockReset();
    mocks.relocateScanSource.mockReset();
    mocks.scanDefaultAclosDir.mockReset();
    mocks.scanRoots.mockReset();
    mocks.updaterOptions.mockReset();
    mocks.progressListener = null;
    mocks.listClipPage.mockResolvedValue({
      items: [],
      offset: 0,
      limit: 50,
      totalCount: 0,
      hasMore: false,
      nextOffset: null,
    });
    mocks.listPendingManualClips.mockResolvedValue([]);
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets());
    mocks.listSources.mockResolvedValue(sourceDirs);
    mocks.listTags.mockResolvedValue([]);
    mocks.openDirectory.mockResolvedValue(null);
    mocks.previewScanSourceRelocation.mockResolvedValue(relocationPreview());
    mocks.requestStartupSourceSync.mockResolvedValue(undefined);
    mocks.relocateScanSource.mockResolvedValue({
      preview: relocationPreview(),
      relocatedClipCount: 1,
      syncJobId: null,
      syncStarted: false,
      syncStatus: null,
      syncMessage: null,
    });
    mocks.scanDefaultAclosDir.mockResolvedValue(jobResult("default", "completed"));
    mocks.cancelScan.mockResolvedValue(cancelResult("scan-active"));
    mocks.getScanSummary.mockResolvedValue(summary("completed"));
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

  it("opens source management immediately when it is the saved startup destination", async () => {
    window.localStorage.setItem(APP_PREFERENCES_STORAGE_KEY, JSON.stringify({
      ...DEFAULT_APP_PREFERENCES,
      startupDestination: "scan",
    }));

    render(<App />);

    expect(await screen.findByRole("heading", { name: "扫描目录" })).toBeVisible();
    expect(screen.getByRole("button", { name: "扫描目录" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("hydrates the automatic update preference before creating the updater controller", () => {
    window.localStorage.setItem(APP_PREFERENCES_STORAGE_KEY, JSON.stringify({
      ...DEFAULT_APP_PREFERENCES,
      automaticUpdateCheck: false,
    }));

    render(<App />);

    expect(mocks.updaterOptions).toHaveBeenCalledWith({ automaticCheck: false });
  });

  it("keeps startup scanning off by default", async () => {
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    render(<App />);

    await waitFor(() => expect(mocks.listSources).toHaveBeenCalled());
    expect(mocks.requestStartupSourceSync).not.toHaveBeenCalled();
  });

  it("saves an enabled startup scan for the next launch without scanning immediately", async () => {
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(mocks.listSources).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: /^设置/ }));
    await user.click(await screen.findByRole("switch", { name: "启动时自动扫描" }));

    expect(mocks.requestStartupSourceSync).not.toHaveBeenCalled();
    expect(JSON.parse(window.localStorage.getItem(APP_PREFERENCES_STORAGE_KEY) ?? "{}"))
      .toEqual(expect.objectContaining({ scanOnStartup: true }));
  });

  it("requests one guarded startup sync when the saved preference is enabled", async () => {
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    window.localStorage.setItem(APP_PREFERENCES_STORAGE_KEY, JSON.stringify({
      ...DEFAULT_APP_PREFERENCES,
      scanOnStartup: true,
    }));

    render(<StrictMode><App /></StrictMode>);

    await waitFor(() => expect(mocks.requestStartupSourceSync).toHaveBeenCalledTimes(1));
  });

  it("scans multiple roots once, disables duplicate starts, and exposes cancelling state", async () => {
    const user = userEvent.setup();
    const scan = deferred<ScanJobResult<ScanSummary>>();
    const cancel = deferred<CancelScanResult>();
    mocks.scanRoots.mockReturnValueOnce(scan.promise);
    mocks.cancelScan.mockReturnValueOnce(cancel.promise);
    render(<App />);
    await openScanTaskWorkspace(user);
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
    await openScanTaskWorkspace(user);
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
    mocks.getScanStatus
      .mockResolvedValueOnce({
        jobId: null,
        phase: null,
        currentRoot: null,
        source: null,
        processed: 0,
        total: null,
        message: "当前没有扫描任务",
        terminal: false,
        status: "idle",
      })
      .mockResolvedValue({
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
    await openScanTaskWorkspace(user);

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
    await openScanTaskWorkspace(user);
    const loadsBeforeScan = collectionLoadCounts();

    await user.click(screen.getByRole("button", { name: "开始扫描" }));
    const partialAlert = await screen.findByRole("alert");
    expect(partialAlert).toHaveTextContent(/扫描部分完成/);
    await expectCollectionsRefreshed(loadsBeforeScan);
  });

  it("keeps the exact terminal count when a secondary scan refresh fails", async () => {
    const user = userEvent.setup();
    mocks.getLibraryFacets
      .mockReset()
      .mockResolvedValueOnce(libraryFacets())
      .mockRejectedValueOnce(new Error("facets offline"));
    mocks.scanRoots.mockResolvedValueOnce({
      ...jobResult("scan-refresh-failure", "completed"),
      result: { ...summary("completed"), newClipCount: 3 },
    });
    render(<App />);
    await openScanTaskWorkspace(user);
    await waitFor(() => expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: "开始扫描" }));

    const refreshAlert = await screen.findByRole("alert");
    expect(refreshAlert).toHaveTextContent(
      "扫描完成：新增 3 个视频；终态已确定，但刷新索引视图失败",
    );
    await user.click(screen.getByRole("button", { name: /识别结果/ }));
    expect(screen.getByText("最近素材").closest("article")).toHaveTextContent(
      "扫描完成：新增 3 个视频；终态已确定，但刷新索引视图失败",
    );
  });

  it("distinguishes ordinary failure from an already-running conflict", async () => {
    const user = userEvent.setup();
    mocks.scanRoots.mockRejectedValueOnce({
      code: "scan-failed",
      message: "磁盘读取失败",
      jobId: "scan-failed",
    });
    render(<App />);
    await openScanTaskWorkspace(user);
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

  it("refreshes sources, clips, and facets after relocation without inventing scan freshness", async () => {
    const user = userEvent.setup();
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    const movedSources = sourceDirs.map((item) => item.id === "1"
      ? { ...item, path: "E:\\Moved", scanRootPath: "E:\\Moved" }
      : item);
    const preview = relocationPreview();
    mocks.listSources
      .mockReset()
      .mockResolvedValueOnce(sourceDirs)
      .mockResolvedValue(movedSources);
    mocks.openDirectory.mockResolvedValueOnce("E:\\Moved");
    mocks.previewScanSourceRelocation.mockResolvedValueOnce(preview);
    mocks.relocateScanSource.mockResolvedValueOnce({
      preview,
      relocatedClipCount: 1,
      syncJobId: null,
      syncStarted: false,
      syncStatus: null,
      syncMessage: null,
    });
    render(<App />);
    await openScanWorkspace(user);

    await user.click(screen.getByRole("button", { name: "重新定位 来源 1" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    expect(mocks.openDirectory).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: "为 来源 1 选择新的来源根目录",
    });
    expect(await screen.findByText("预览通过，可以提交")).toBeVisible();
    expect(mocks.previewScanSourceRelocation).toHaveBeenCalledWith("1", "E:\\Moved");
    const loadsBeforeCommit = collectionLoadCounts();

    await user.click(screen.getByRole("button", { name: "继续确认" }));
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位成功；同步尚未启动")).toBeVisible();
    expect(mocks.relocateScanSource).toHaveBeenCalledWith("1", "E:\\Moved");
    await waitFor(() => {
      const current = collectionLoadCounts();
      expect(current.sources).toBeGreaterThan(loadsBeforeCommit.sources);
      expect(current.clips).toBeGreaterThan(loadsBeforeCommit.clips);
      expect(current.facets).toBeGreaterThan(loadsBeforeCommit.facets);
    });
    expect(screen.getAllByText("尚未完成首次扫描").length).toBeGreaterThan(0);
    expect(screen.getAllByText("E:\\Moved").length).toBeGreaterThan(0);
  });

  it("leaves a relocation follow-up job's terminal notification and refresh to the scan controller", async () => {
    const user = userEvent.setup();
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    const movedSources = sourceDirs.map((item) => item.id === "1"
      ? { ...item, path: "E:\\Moved", scanRootPath: "E:\\Moved" }
      : item);
    const preview = relocationPreview();
    const relocation = deferred<Awaited<ReturnType<typeof mocks.relocateScanSource>>>();
    mocks.listSources
      .mockReset()
      .mockResolvedValueOnce(sourceDirs)
      .mockResolvedValue(movedSources);
    mocks.openDirectory.mockResolvedValueOnce("E:\\Moved");
    mocks.previewScanSourceRelocation.mockResolvedValueOnce(preview);
    mocks.relocateScanSource.mockReturnValueOnce(relocation.promise);
    mocks.getScanSummary.mockResolvedValueOnce({
      ...summary("completed"),
      newClipCount: 4,
    });
    render(<App />);
    await openScanWorkspace(user);

    await user.click(screen.getByRole("button", { name: "重新定位 来源 1" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    const loadsBeforeCommit = collectionLoadCounts();
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));
    await waitFor(() => expect(mocks.relocateScanSource).toHaveBeenCalledTimes(1));

    emitProgress(terminalProgress("relocation-follow-up", "completed", "同步完成"));
    await waitFor(() => {
      expect(screen.getAllByText("扫描完成：新增 4 个视频").length).toBeGreaterThan(0);
    });
    await waitFor(() => {
      const current = collectionLoadCounts();
      expect(current).toEqual({
        clips: loadsBeforeCommit.clips + 1,
        facets: loadsBeforeCommit.facets + 1,
        sources: loadsBeforeCommit.sources + 1,
        tags: loadsBeforeCommit.tags + 1,
      });
    });

    await act(async () => {
      relocation.resolve({
        preview,
        relocatedClipCount: 1,
        syncJobId: "relocation-follow-up",
        syncStarted: true,
        syncStatus: "completed",
        syncMessage: "同步完成",
      });
      await relocation.promise;
    });
    expect(await screen.findByText("重新定位成功，同步已完成")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "完成" }));
    await user.click(screen.getByRole("button", { name: /识别结果/ }));
    expect(screen.getByText("最近素材").closest("article")).toHaveTextContent(
      "扫描完成：新增 4 个视频",
    );
    expect(collectionLoadCounts()).toEqual({
      clips: loadsBeforeCommit.clips + 1,
      facets: loadsBeforeCommit.facets + 1,
      sources: loadsBeforeCommit.sources + 1,
      tags: loadsBeforeCommit.tags + 1,
    });
  });

  it("settles a relocation follow-up from the command result when all live events were missed", async () => {
    const user = userEvent.setup();
    Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Reflect.defineProperty(globalThis, "isTauri", {
      configurable: true,
      value: true,
    });
    const movedSources = sourceDirs.map((item) => item.id === "1"
      ? { ...item, path: "E:\\Moved", scanRootPath: "E:\\Moved" }
      : item);
    const preview = relocationPreview();
    mocks.listSources
      .mockReset()
      .mockResolvedValueOnce(sourceDirs)
      .mockResolvedValue(movedSources);
    mocks.openDirectory.mockResolvedValueOnce("E:\\Moved");
    mocks.previewScanSourceRelocation.mockResolvedValueOnce(preview);
    mocks.relocateScanSource.mockResolvedValueOnce({
      preview,
      relocatedClipCount: 1,
      syncJobId: "relocation-missed-events",
      syncStarted: true,
      syncStatus: "completed",
      syncMessage: "同步完成",
    });
    mocks.getScanSummary.mockResolvedValueOnce({
      ...summary("completed"),
      newClipCount: 5,
    });
    render(<App />);
    await openScanWorkspace(user);

    await user.click(screen.getByRole("button", { name: "重新定位 来源 1" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    const loadsBeforeCommit = collectionLoadCounts();
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位成功，同步已完成")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "完成" }));
    await user.click(screen.getByRole("button", { name: /识别结果/ }));
    expect(screen.getByText("最近素材").closest("article")).toHaveTextContent(
      "扫描完成：新增 5 个视频",
    );
    expect(collectionLoadCounts()).toEqual({
      clips: loadsBeforeCommit.clips + 1,
      facets: loadsBeforeCommit.facets + 1,
      sources: loadsBeforeCommit.sources + 1,
      tags: loadsBeforeCommit.tags + 1,
    });
  });

  it.each(["resolve", "reject"] as const)(
    "blocks update installation while source relocation is pending and clears the blocker after %s",
    async (settlement) => {
      const user = userEvent.setup();
      Reflect.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        value: {},
      });
      Reflect.defineProperty(globalThis, "isTauri", {
        configurable: true,
        value: true,
      });
      const preview = relocationPreview();
      const relocation = deferred<Awaited<ReturnType<typeof mocks.relocateScanSource>>>();
      prepareDownloadedUpdater();
      mocks.openDirectory.mockResolvedValueOnce("E:\\Moved");
      mocks.previewScanSourceRelocation.mockResolvedValueOnce(preview);
      mocks.relocateScanSource.mockReturnValueOnce(relocation.promise);
      render(<App />);
      await openScanWorkspace(user);

      await user.click(screen.getByRole("button", { name: "重新定位 来源 1" }));
      await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
      await user.click(await screen.findByRole("button", { name: "继续确认" }));
      await user.click(screen.getByRole("button", { name: "确认重新定位" }));
      await waitFor(() => expect(mocks.relocateScanSource).toHaveBeenCalledTimes(1));

      fireEvent.click(screen.getByRole("button", { name: /^设置/, hidden: true }));
      expect(await screen.findByText(
        "来源重新定位任务正在运行，请等待任务结束后再安装",
      )).toBeVisible();
      expect(screen.getByRole("button", { name: "安装并重启" })).toBeDisabled();

      await act(async () => {
        if (settlement === "resolve") {
          relocation.resolve({
            preview,
            relocatedClipCount: 1,
            syncJobId: null,
            syncStarted: false,
            syncStatus: null,
            syncMessage: null,
          });
        } else {
          relocation.reject(new Error("重新定位失败"));
        }
        await relocation.promise.catch(() => undefined);
      });

      await waitFor(() => {
        expect(screen.queryByText(/来源重新定位任务正在运行/)).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "安装并重启" })).toBeEnabled();
      });
    },
  );
});

async function openScanWorkspace(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => expect(mocks.listSources).toHaveBeenCalled());
  await user.click(screen.getByRole("button", { name: "扫描目录" }));
  await screen.findByRole("heading", { name: "扫描目录" });
  const navigation = screen.getByRole("navigation", { name: "扫描分类" });
  await user.click(within(navigation).getByRole("button", { name: /视频来源/ }));
}

async function openScanTaskWorkspace(user: ReturnType<typeof userEvent.setup>) {
  await openScanWorkspace(user);
  await user.click(screen.getByRole("button", { name: /扫描任务/ }));
}

function source(id: string, path: string): SourceDir {
  return {
    id,
    name: `来源 ${id}`,
    displayName: `来源 ${id}`,
    path,
    sourceKind: "aclos",
    scanMode: "aclos-structured",
    scanRootPath: path,
    enabled: true,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 1,
    lastScanAt: null,
  };
}

function relocationPreview(): ScanSourceRelocationPreview {
  return {
    sourceId: "1",
    oldRootPath: sourceDirs[0].scanRootPath,
    newRootPath: "E:\\Moved",
    affectedSources: [{
      id: "1",
      displayName: "来源 1",
      oldSourcePath: sourceDirs[0].path,
      newSourcePath: "E:\\Moved",
      clipCount: 1,
    }],
    exactPathMatchCount: 1,
    identityMatchCount: 0,
    legacyFingerprintMatchCount: 0,
    unmatchedCount: 0,
    newCandidateCount: 0,
    expectedClipUpdateCount: 1,
    expectedGroupUpdateCount: 1,
    expectedCoverUpdateCount: 0,
    expectedMetadataReferenceUpdateCount: 0,
    conflicts: [],
    blockers: [],
    canRelocate: true,
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

function terminalProgress(
  jobId: string,
  status: "completed" | "partial" | "cancelled" | "failed",
  message: string,
): ScanProgress {
  return {
    jobId,
    phase: status,
    currentRoot: "E:\\Moved",
    source: null,
    processed: 1,
    total: 1,
    message,
    terminal: true,
    status,
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

function resetAppUpdater() {
  Object.assign(mocks.appUpdater, {
    runtimeInfo: {
      currentVersion: "0.2.1",
      channel: "stable",
      endpoint: "https://github.com/2424521842/valoframe/releases/latest/download/latest.json",
      configured: true,
    },
    runtimeStatus: "ready",
    runtimeError: null,
    phase: "idle",
    update: null,
    progress: { downloadedBytes: 0, totalBytes: null },
    message: "更新检查尚未运行",
    error: null,
    canCheck: true,
    canDownload: false,
    canCancelDownload: false,
    canInstall: false,
  });
}

function prepareDownloadedUpdater() {
  Object.assign(mocks.appUpdater, {
    phase: "downloaded",
    update: {
      currentVersion: "0.2.1",
      version: "0.2.2",
      notes: "自动更新集成测试",
      publishedAt: "2026-08-10T00:00:00Z",
    },
    message: "更新包已下载并通过签名验证，可以安装",
    error: null,
    canCheck: false,
    canDownload: false,
    canCancelDownload: false,
    canInstall: true,
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
