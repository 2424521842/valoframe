import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { SourceRelocationDialog } from "../../src/components/SourceRelocationDialog";
import { ScanWorkspace } from "../../src/screens/ScanWorkspace";
import type {
  PendingManualClip,
  RelocateScanSourceResult,
  ScanSourceRelocationPreview,
  ScanSummary,
  SourceDir,
} from "../../src/types";

describe("ScanWorkspace freshness and terminal feedback", () => {
  it("surfaces an explicit NVIDIA import entry and opens the wizard with NVIDIA selected", async () => {
    const user = userEvent.setup();
    renderWorkspace();
    await openSourcesSection(user);

    const entryHeading = screen.getByRole("heading", { name: "导入 NVIDIA 录屏" });
    const entry = entryHeading.closest("section");
    expect(entry).not.toBeNull();
    expect(within(entry!).getByText(/NVIDIA App 保存录屏的 MP4 目录/)).toBeVisible();
    expect(within(entry!).getByText(/NVIDIA 私有元数据/)).toBeVisible();

    await user.click(within(entry!).getByRole("button", { name: "选择 NVIDIA 目录" }));

    const dialog = screen.getByRole("dialog", { name: "添加视频来源" });
    expect(within(dialog).getByRole("button", { name: /NVIDIA 录屏/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(dialog).getByText(/NVIDIA 私有元数据/)).toBeVisible();
  });

  it("keeps NVIDIA import available for explanation during a scan and keeps cancellation in the task section", async () => {
    const user = userEvent.setup();
    const onCancelScan = vi.fn();
    renderWorkspace({
      activeJobId: "scan-job",
      isScanning: true,
      onCancelScan,
      scanStatus: "running",
    });

    const navigation = screen.getByRole("navigation", { name: "扫描分类" });
    expect(within(navigation).getByRole("button", { name: /扫描任务/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await user.click(screen.getByRole("button", { name: "取消扫描" }));
    expect(onCancelScan).toHaveBeenCalledTimes(1);

    await user.click(within(navigation).getByRole("button", { name: /视频来源/ }));
    const entryHeading = screen.getByRole("heading", { name: "导入 NVIDIA 录屏" });
    const entry = entryHeading.closest("section");
    expect(entry).not.toBeNull();
    expect(within(entry!).getByRole("status")).toHaveTextContent("请先取消扫描或等待完成");

    const explanationButton = within(entry!).getByRole("button", { name: "查看导入说明" });
    expect(explanationButton).toBeEnabled();

    await user.click(explanationButton);
    const dialog = screen.getByRole("dialog", { name: "添加视频来源" });
    expect(within(dialog).getByRole("status")).toHaveTextContent("请先取消扫描或等待完成");
    expect(within(dialog).getByRole("button", { name: "选择目录" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "添加并首次同步" })).toBeDisabled();
  });

  it("uses the settings-style source, task, pending, and result navigation", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    const workflow = screen.getByRole("navigation", { name: "扫描分类" });
    const steps = within(workflow).getAllByRole("button");
    expect(steps).toHaveLength(4);
    expect(steps[0]).toHaveTextContent("扫描任务");
    expect(steps[1]).toHaveTextContent("视频来源");
    expect(steps[2]).toHaveTextContent("待录入");
    expect(steps[3]).toHaveTextContent("识别结果");
    expect(steps[0]).toHaveAttribute("aria-current", "page");

    await user.click(steps[3]);
    expect(steps[3]).toHaveAttribute("aria-current", "page");
  });

  it("always shows source-local freshness and excludes disabled sources from alerts", async () => {
    const user = userEvent.setup();
    renderWorkspace({
      sourceDirs: [
        source("today", "2026-08-09T00:00:00Z"),
        source("six", "2026-08-03T00:00:00Z"),
        source("seven", "2026-08-02T00:00:00Z"),
        source("disabled-never", null, false),
      ],
    });
    await openSourcesSection(user);

    expect(screen.getByText("今天扫描")).toBeInTheDocument();
    expect(screen.getByText("6 天未扫描")).toBeInTheDocument();
    expect(screen.getByText("7 天未扫描")).toBeInTheDocument();
    expect(screen.getByText("尚未完成首次扫描")).toBeInTheDocument();
    expect(screen.getByText("1 个视频来源需要扫描，最长 7 天未扫描")).toBeInTheDocument();
    expect(screen.queryByText(/其中 1 个尚未完成首次扫描/)).not.toBeInTheDocument();
  });

  it("labels source eligibility separately from the global startup scan setting", async () => {
    const user = userEvent.setup();
    const onSetSourceEnabled = vi.fn();
    renderWorkspace({
      sourceDirs: [source("archive", null, false)],
      onSetSourceEnabled,
    });
    await openSourcesSection(user);

    expect(screen.getByText("未加入自动同步")).toBeVisible();
    const includeSource = screen.getByRole("button", { name: "加入自动同步 archive" });
    expect(includeSource).toHaveAttribute("aria-pressed", "false");

    await user.click(includeSource);
    expect(onSetSourceEnabled).toHaveBeenCalledWith(
      expect.objectContaining({ id: "archive" }),
      true,
    );
  });

  it("aggregates overdue and first-scan enabled sources", async () => {
    const user = userEvent.setup();
    renderWorkspace({
      sourceDirs: [
        source("nine", "2026-07-31T00:00:00Z"),
        source("first", null),
      ],
    });
    await openSourcesSection(user);

    expect(screen.getByText(
      "2 个视频来源需要扫描，最长 9 天未扫描，其中 1 个尚未完成首次扫描",
    )).toBeInTheDocument();
  });

  it("uses the shared terminal formatter for zero and unavailable counts", async () => {
    const { rerender } = renderWorkspace({
      scanStatus: "completed",
      summary: { ...scanSummary(), newClipCount: 0 },
    });
    await userEvent.setup().click(screen.getByRole("button", { name: /扫描任务/ }));
    expect(screen.getAllByText("扫描完成：新增 0 个视频").length).toBeGreaterThan(0);

    rerender(workspace({ scanStatus: "completed", summary: null }));
    expect(screen.getAllByText("扫描完成：新增数量不可用").length).toBeGreaterThan(0);
  });

  it("opens relocation only from its source action and keeps cancellation side-effect free", async () => {
    const user = userEvent.setup();
    const chooseDirectory = vi.fn(async () => null);
    const previewRelocation = vi.fn();
    const relocateSource = vi.fn();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-09T00:00:00Z")],
      onChooseRelocationDirectory: chooseDirectory,
      onPreviewSourceRelocation: previewRelocation,
      onRelocateSource: relocateSource,
    });

    expect(screen.queryByRole("dialog", { name: "重新定位来源根目录" })).not.toBeInTheDocument();
    expect(previewRelocation).not.toHaveBeenCalled();
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));

    const relocationDialog = screen.getByRole("dialog", { name: "重新定位来源根目录" });
    expect(relocationDialog).toHaveTextContent("预览只读取目录与索引");
    expect(relocationDialog).toHaveTextContent("D:\\archive");
    await user.click(within(relocationDialog).getByRole("button", { name: "选择新的根目录" }));

    expect(chooseDirectory).toHaveBeenCalledTimes(1);
    expect(chooseDirectory).toHaveBeenCalledWith(expect.objectContaining({ id: "archive" }));
    expect(previewRelocation).not.toHaveBeenCalled();
    expect(relocateSource).not.toHaveBeenCalled();
    expect(relocationDialog).toHaveTextContent("已取消选择");
    expect(screen.getByText("今天扫描")).toBeVisible();
  });

  it("shows preview loading and does not commit or mutate the source before confirmation", async () => {
    const user = userEvent.setup();
    const request = deferred<ScanSourceRelocationPreview>();
    const previewRelocation = vi.fn(() => request.promise);
    const relocateSource = vi.fn();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-09T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: previewRelocation,
      onRelocateSource: relocateSource,
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));

    expect(screen.getByRole("status", { name: "" })).toHaveTextContent("正在生成重新定位预览");
    expect(previewRelocation).toHaveBeenCalledWith("archive", "E:\\Moved");
    expect(relocateSource).not.toHaveBeenCalled();
    expect(screen.getAllByText("D:\\archive").length).toBeGreaterThan(0);

    await act(async () => request.resolve(relocationPreview()));
    expect(await screen.findByText("预览通过，可以提交")).toBeVisible();
    expect(screen.getByLabelText("重新定位匹配统计")).toHaveTextContent("可信匹配11");
    expect(relocateSource).not.toHaveBeenCalled();
  });

  it("invalidates a late directory choice when the dialog source changes", async () => {
    const user = userEvent.setup();
    const directoryChoice = deferred<string | null>();
    const previewRelocation = vi.fn(async () => relocationPreview());
    const relocateSource = vi.fn();
    const props = {
      open: true,
      source: source("archive-a", null),
      onOpenChange: vi.fn(),
      onChooseDirectory: vi.fn(() => directoryChoice.promise),
      onPreview: previewRelocation,
      onRelocate: relocateSource,
    };
    const { rerender } = render(<SourceRelocationDialog {...props} />);
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));

    for (const closeButton of screen.getAllByRole("button", { name: "关闭" })) {
      expect(closeButton).toBeDisabled();
    }
    rerender(<SourceRelocationDialog {...props} source={source("archive-b", null)} />);
    await act(async () => directoryChoice.resolve("E:\\Moved-A"));

    expect(previewRelocation).not.toHaveBeenCalled();
    expect(relocateSource).not.toHaveBeenCalled();
    expect(screen.getByText("D:\\archive-b")).toBeVisible();
    expect(screen.getByText("尚未选择")).toBeVisible();
  });

  it("renders zero-match blockers and conflicts and prevents submission", async () => {
    const user = userEvent.setup();
    const previewRelocation = vi.fn(async () => relocationPreview({
      exactPathMatchCount: 0,
      identityMatchCount: 0,
      legacyFingerprintMatchCount: 0,
      expectedClipUpdateCount: 0,
      conflicts: [{
        code: "duplicate-identity",
        message: "同一稳定身份对应多个候选",
        oldClipIds: ["clip-7"],
        candidatePaths: ["E:\\Moved\\copy-a.mp4", "E:\\Moved\\copy-b.mp4"],
      }],
      blockers: [{
        code: "zero-trusted-matches",
        message: "新根目录中没有可信匹配",
      }],
      canRelocate: false,
    }));
    const relocateSource = vi.fn();
    renderWorkspace({
      sourceDirs: [source("archive", null)],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: previewRelocation,
      onRelocateSource: relocateSource,
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));

    expect(await screen.findByText("当前预览不可提交")).toBeVisible();
    expect(screen.getByText(/未找到可信匹配/)).toBeVisible();
    expect(screen.getByText("新根目录中没有可信匹配")).toBeVisible();
    const conflicts = screen.getByLabelText("重新定位冲突");
    expect(conflicts).toHaveTextContent("duplicate-identity");
    expect(conflicts).toHaveTextContent("clip-7");
    expect(conflicts).toHaveTextContent("copy-a.mp4");
    expect(screen.getByRole("button", { name: "继续确认" })).toBeDisabled();
    expect(relocateSource).not.toHaveBeenCalled();
  });

  it("requires a second confirmation and keeps a failed commit available for retry", async () => {
    const user = userEvent.setup();
    const relocateSource = vi.fn(async () => {
      throw new Error("数据库已锁定");
    });
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-03T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: vi.fn(async () => relocationPreview()),
      onRelocateSource: relocateSource,
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));

    expect(screen.getByText("提交前最后确认")).toBeVisible();
    expect(screen.getByText(/不会移动、复制、重命名或删除/)).toBeVisible();
    expect(relocateSource).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位失败：数据库已锁定")).toBeVisible();
    expect(relocateSource).toHaveBeenCalledWith("archive", "E:\\Moved");
    expect(screen.getByRole("button", { name: "确认重新定位" })).toBeEnabled();
    expect(screen.getByText("6 天未扫描")).toBeVisible();
  });

  it("reports committed relocation with pending sync without changing freshness", async () => {
    const user = userEvent.setup();
    const preview = relocationPreview();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-03T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: vi.fn(async () => preview),
      onRelocateSource: vi.fn(async () => relocationResult(preview, false)),
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位成功；同步尚未启动")).toBeVisible();
    expect(screen.getByText(/已原地更新 11 条素材索引/)).toBeVisible();
    expect(screen.getByText(/只会在完整同步成功后刷新/)).toBeVisible();
    expect(screen.getByText("6 天未扫描")).toBeVisible();
  });

  it("identifies a failed follow-up synchronization without calling it running", async () => {
    const user = userEvent.setup();
    const preview = relocationPreview();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-03T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: vi.fn(async () => preview),
      onRelocateSource: vi.fn(async () => relocationResult(
        preview,
        false,
        "failed-relocation-sync",
      )),
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位成功；同步失败，待重试")).toBeVisible();
    expect(screen.getByText("failed-relocation-sync")).toBeVisible();
    expect(screen.getByText(/失败\/待重试任务/)).toBeVisible();
    expect(screen.queryByText("重新定位成功，同步已完成")).not.toBeInTheDocument();
    expect(screen.queryByText(/同步尚未启动/)).not.toBeInTheDocument();
  });

  it.each([
    ["partial" as const, "重新定位成功；同步部分完成，建议重试"],
    ["cancelled" as const, "重新定位成功；同步已取消，待重试"],
  ])("renders the %s follow-up terminal without presenting it as complete", async (
    syncStatus,
    expectedTitle,
  ) => {
    const user = userEvent.setup();
    const preview = relocationPreview();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-03T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: vi.fn(async () => preview),
      onRelocateSource: vi.fn(async () => ({
        ...relocationResult(preview, true),
        syncStatus,
        syncMessage: `同步 ${syncStatus}`,
      })),
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText(expectedTitle)).toBeVisible();
    expect(screen.queryByText("重新定位成功，同步已完成")).not.toBeInTheDocument();
    expect(screen.getByText(/重新定位不会回滚/)).toBeVisible();
  });

  it("shows the synchronization job after a successful relocation commit", async () => {
    const user = userEvent.setup();
    const preview = relocationPreview();
    renderWorkspace({
      sourceDirs: [source("archive", "2026-08-09T00:00:00Z")],
      onChooseRelocationDirectory: vi.fn(async () => "E:\\Moved"),
      onPreviewSourceRelocation: vi.fn(async () => preview),
      onRelocateSource: vi.fn(async () => relocationResult(preview, true)),
    });
    await openSourcesSection(user);
    await user.click(screen.getByRole("button", { name: "重新定位 archive" }));
    await user.click(screen.getByRole("button", { name: "选择新的根目录" }));
    await user.click(await screen.findByRole("button", { name: "继续确认" }));
    await user.click(screen.getByRole("button", { name: "确认重新定位" }));

    expect(await screen.findByText("重新定位成功，同步已完成")).toBeVisible();
    expect(screen.getByText("relocation-sync-job")).toBeVisible();
  });

  it("lists pending NVIDIA recordings and opens the manual import dialog", async () => {
    const user = userEvent.setup();
    const onSetPendingIgnored = vi.fn();
    renderWorkspace({
      sourceDirs: [nvidiaSource()],
      pendingClips: [pendingClip()],
      onSetPendingIgnored,
    });

    const navigation = screen.getByRole("navigation", { name: "扫描分类" });
    await user.click(within(navigation).getByRole("button", { name: /待录入/ }));

    expect(screen.getByText("待录入的 NVIDIA 视频")).toBeVisible();
    expect(screen.getByText("Valorant 2026.07.02 - 21.58.03.07.mp4")).toBeVisible();
    expect(screen.getByText(/不会自动导入素材库/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "忽略" }));
    expect(onSetPendingIgnored).toHaveBeenCalledWith("pending-1", true);

    await user.click(screen.getByRole("button", { name: "录入" }));
    const dialog = await screen.findByRole("dialog", { name: "录入 NVIDIA 视频" });
    expect(within(dialog).getByText("Valorant 2026.07.02 - 21.58.03.07.mp4")).toBeVisible();
  });

  it("flags the pending nav entry when recordings await classification", () => {
    renderWorkspace({
      pendingClips: [pendingClip()],
    });

    const navigation = screen.getByRole("navigation", { name: "扫描分类" });
    const pendingNav = within(navigation).getByRole("button", { name: /待录入/ });
    expect(pendingNav).not.toHaveAttribute("aria-current");
    expect(within(pendingNav).getByLabelText(/有 1 个 NVIDIA 视频待录入/)).toBeVisible();
  });
});

function renderWorkspace(overrides: Partial<ComponentProps<typeof ScanWorkspace>>) {
  return render(workspace(overrides));
}

async function openSourcesSection(user: ReturnType<typeof userEvent.setup>) {
  const navigation = screen.getByRole("navigation", { name: "扫描分类" });
  await user.click(within(navigation).getByRole("button", { name: /视频来源/ }));
}

function workspace(overrides: Partial<ComponentProps<typeof ScanWorkspace>>) {
  return (
    <ScanWorkspace
      activeJobId={null}
      accounts={[]}
      activityMessage="当前索引已加载"
      errorMessage={null}
      facets={null}
      isLoading={false}
      isScanning={false}
      localDay={new Date("2026-08-09T12:00:00Z")}
      pendingClips={[]}
      pendingIgnoredCount={0}
      pendingError={null}
      isPendingLoading={false}
      importingPendingId={null}
      showIgnoredPending={false}
      manualAgentNames={["捷风"]}
      manualMapNames={["霓虹町"]}
      manualGameModes={["竞技模式"]}
      progress={null}
      scanStatus="idle"
      scanTargets={[]}
      sourceDirs={[]}
      summary={null}
      onCancelScan={vi.fn()}
      onChooseRelocationDirectory={vi.fn(async () => null)}
      onChooseSourceDirectory={vi.fn(async () => null)}
      onDiscoverAll={vi.fn()}
      onOpenLibrary={vi.fn()}
      onPreviewSourceRelocation={vi.fn()}
      onRegisterSource={vi.fn()}
      onRelocateSource={vi.fn()}
      onRemoveDirectory={vi.fn()}
      onSetSourceEnabled={vi.fn()}
      onStartScan={vi.fn()}
      onSyncEnabledSources={vi.fn()}
      onSyncSource={vi.fn()}
      onImportPendingClip={vi.fn(async () => true)}
      onSetPendingIgnored={vi.fn()}
      onToggleShowIgnoredPending={vi.fn()}
      {...overrides}
    />
  );
}

function source(id: string, lastScanAt: string | null, enabled = true): SourceDir {
  return {
    id,
    name: id,
    displayName: id,
    path: `D:\\${id}`,
    sourceKind: "generic",
    scanMode: "recursive-mp4",
    scanRootPath: `D:\\${id}`,
    enabled,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 0,
    lastScanAt,
  };
}

function nvidiaSource(): SourceDir {
  return {
    id: "nvidia",
    name: "NVIDIA 录屏",
    displayName: "NVIDIA 录屏",
    path: "D:\\Videos\\NVIDIA",
    sourceKind: "nvidia",
    scanMode: "recursive-mp4",
    scanRootPath: "D:\\Videos\\NVIDIA",
    enabled: true,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 0,
    lastScanAt: null,
  };
}

function pendingClip(): PendingManualClip {
  return {
    id: "pending-1",
    sourceDirId: "nvidia",
    sourceDirName: "NVIDIA 录屏",
    filePath: "D:\\Videos\\NVIDIA\\Valorant 2026.07.02 - 21.58.03.07.mp4",
    fileName: "Valorant 2026.07.02 - 21.58.03.07.mp4",
    fileSize: 84_500_000,
    modifiedAt: "2026-07-02T22:06:00Z",
    sourceRelativeDir: "",
    ignored: false,
    firstDiscoveredAt: "2026-07-02T22:45:00Z",
  };
}

function scanSummary(): ScanSummary {
  return {
    rootPath: "D:\\clips",
    sourceDirCount: 1,
    clipGroupCount: 0,
    newClipCount: 0,
    updatedClipCount: 0,
    missingClipCount: 0,
    coverMissingCount: 0,
    errors: [],
    message: null,
  };
}

function relocationPreview(
  overrides: Partial<ScanSourceRelocationPreview> = {},
): ScanSourceRelocationPreview {
  return {
    sourceId: "archive",
    oldRootPath: "D:\\archive",
    newRootPath: "E:\\Moved",
    affectedSources: [{
      id: "archive",
      displayName: "archive",
      oldSourcePath: "D:\\archive",
      newSourcePath: "E:\\Moved",
      clipCount: 12,
    }],
    exactPathMatchCount: 8,
    identityMatchCount: 2,
    legacyFingerprintMatchCount: 1,
    unmatchedCount: 1,
    newCandidateCount: 2,
    expectedClipUpdateCount: 11,
    expectedGroupUpdateCount: 4,
    expectedCoverUpdateCount: 5,
    expectedMetadataReferenceUpdateCount: 6,
    conflicts: [],
    blockers: [],
    canRelocate: true,
    ...overrides,
  };
}

function relocationResult(
  preview: ScanSourceRelocationPreview,
  syncStarted: boolean,
  syncJobId: string | null = syncStarted ? "relocation-sync-job" : null,
): RelocateScanSourceResult {
  const syncStatus = syncStarted ? "completed" : syncJobId ? "failed" : null;
  return {
    preview,
    relocatedClipCount: preview.expectedClipUpdateCount,
    syncJobId,
    syncStarted,
    syncStatus,
    syncMessage: syncStatus ? `同步${syncStatus}` : null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
