import {
  ArrowRight,
  ArrowsClockwise,
  ChartBar,
  ClipboardText,
  Database,
  Eye,
  EyeSlash,
  FolderOpen,
  HardDrives,
  MonitorPlay,
  Play,
  Plus,
  Power,
  Stop,
  Timer,
  UserCircle,
  WarningCircle,
  X,
  type Icon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ManualClipImportDialog } from "../components/ManualClipImportDialog";
import { SourceRelocationDialog } from "../components/SourceRelocationDialog";
import { SourceWizardDialog } from "../components/SourceWizardDialog";
import {
  UiSelect,
  UiSelectContent,
  UiSelectItem,
  UiSelectTrigger,
  UiSelectValue,
} from "../components/ui/select";
import { formatBytes, formatDateTime } from "../lib/formatters";
import {
  sourceScanFreshness,
  summarizeScanFreshness,
} from "../lib/scanFreshness";
import { scanTerminalActivityMessage } from "../lib/scanSummary";
import { scanPathKey, scanTargetPathForSource } from "../lib/scanTargets";
import type {
  AccountSummary,
  LibraryFacets,
  ManualClipImportInput,
  PendingManualClip,
  RegisterScanSourceInput,
  RegisterScanSourceResult,
  RelocateScanSourceResult,
  ScanJobStatus,
  ScanProgress,
  ScanSourceRelocationPreview,
  ScanSummary,
  ScanTarget,
  SourceDir,
  SourceKind,
} from "../types";

type ScanWorkspaceProps = {
  activeJobId: string | null;
  facets: LibraryFacets | null;
  accounts: AccountSummary[];
  scanTargets: ScanTarget[];
  sourceDirs: SourceDir[];
  pendingClips: PendingManualClip[];
  pendingIgnoredCount: number;
  pendingError: string | null;
  isPendingLoading: boolean;
  importingPendingId: string | null;
  showIgnoredPending: boolean;
  manualAgentNames: readonly string[];
  manualMapNames: readonly string[];
  manualGameModes: readonly string[];
  isLoading: boolean;
  isScanning: boolean;
  progress: ScanProgress | null;
  scanStatus: ScanJobStatus;
  summary: ScanSummary | null;
  errorMessage: string | null;
  activityMessage: string;
  localDay: Date;
  onChooseSourceDirectory: (kind: SourceKind) => Promise<string | null>;
  onChooseRelocationDirectory: (source: SourceDir) => Promise<string | null>;
  onPreviewSourceRelocation: (
    sourceId: string,
    newRootPath: string,
  ) => Promise<ScanSourceRelocationPreview>;
  onRelocateSource: (
    sourceId: string,
    newRootPath: string,
  ) => Promise<RelocateScanSourceResult>;
  onRegisterSource: (input: RegisterScanSourceInput) => Promise<RegisterScanSourceResult>;
  onCancelScan: () => void;
  onDiscoverAll: () => void;
  onOpenLibrary: () => void;
  onRemoveDirectory: (target: ScanTarget) => void;
  onSetSourceEnabled: (source: SourceDir, enabled: boolean) => void;
  onStartScan: () => void;
  onSyncEnabledSources: () => void;
  onSyncSource: (source: SourceDir) => void;
  onImportPendingClip: (pendingId: string, input: ManualClipImportInput) => Promise<boolean>;
  onSetPendingIgnored: (pendingId: string, ignored: boolean) => void;
  onToggleShowIgnoredPending: () => void;
};

type ScanWorkspaceSection = "sources" | "task" | "pending" | "results";

type ScanSectionDefinition = {
  id: ScanWorkspaceSection;
  label: string;
  description: string;
  icon: Icon;
};

const SCAN_SECTIONS: ScanSectionDefinition[] = [
  { id: "task", label: "扫描任务", description: "选择范围并查看进度", icon: ArrowsClockwise },
  { id: "sources", label: "视频来源", description: "添加、启用与维护目录", icon: FolderOpen },
  { id: "pending", label: "待录入", description: "手动补全 NVIDIA 录屏分类", icon: ClipboardText },
  { id: "results", label: "识别结果", description: "核对统计、账号与素材库", icon: ChartBar },
];

export function ScanWorkspace({
  activeJobId,
  facets,
  accounts,
  scanTargets,
  sourceDirs,
  pendingClips,
  pendingIgnoredCount,
  pendingError,
  isPendingLoading,
  importingPendingId,
  showIgnoredPending,
  manualAgentNames,
  manualMapNames,
  manualGameModes,
  isLoading,
  isScanning,
  progress,
  scanStatus,
  summary,
  errorMessage,
  activityMessage,
  localDay,
  onChooseSourceDirectory,
  onChooseRelocationDirectory,
  onPreviewSourceRelocation,
  onRelocateSource,
  onRegisterSource,
  onCancelScan,
  onDiscoverAll,
  onOpenLibrary,
  onRemoveDirectory,
  onSetSourceEnabled,
  onStartScan,
  onSyncEnabledSources,
  onSyncSource,
  onImportPendingClip,
  onSetPendingIgnored,
  onToggleShowIgnoredPending,
}: ScanWorkspaceProps) {
  const [isSourceWizardOpen, setIsSourceWizardOpen] = useState(false);
  const [relocationSource, setRelocationSource] = useState<SourceDir | null>(null);
  const [importTarget, setImportTarget] = useState<PendingManualClip | null>(null);
  const [activeSection, setActiveSection] = useState<ScanWorkspaceSection>("task");
  const workspaceRef = useRef<HTMLElement>(null);
  const sectionHeadingRef = useRef<HTMLHeadingElement>(null);
  const previousSectionRef = useRef(activeSection);
  const wasScanningRef = useRef(isScanning);
  const observedProgressJobRef = useRef(progress?.jobId ?? null);
  const totalSize = facets?.activeSizeBytes ?? 0;
  const latestModifiedAt = facets?.modifiedAtMax
    ? new Date(facets.modifiedAtMax * 1_000).toISOString()
    : null;
  const hasDeterminateProgress = progress?.total !== null
    && progress?.total !== undefined && progress.total > 0;
  const progressPercent = hasDeterminateProgress && progress
    ? Math.min(100, Math.round((progress.processed / progress.total!) * 100))
    : progress?.terminal
      ? 100
      : null;
  const freshnessBySourceId = useMemo(() => new Map(
    sourceDirs.map((source) => [
      source.id,
      sourceScanFreshness(source.lastScanAt, localDay),
    ]),
  ), [localDay, sourceDirs]);
  const freshnessSummary = useMemo(
    () => summarizeScanFreshness(sourceDirs, localDay),
    [localDay, sourceDirs],
  );
  const terminalMessage = isTerminalScanStatus(scanStatus)
    ? scanTerminalActivityMessage(scanStatus, summary)
    : null;
  const sourceWizardDisabledReason = isScanning
    ? scanStatus === "cancelling"
      ? "当前扫描正在安全取消，完成后即可选择 NVIDIA 目录。"
      : "当前扫描任务正在运行。请先取消扫描或等待完成，再选择 NVIDIA 目录。"
    : isLoading
      ? "本地索引正在载入，请稍候再选择 NVIDIA 目录。"
      : null;
  const indexedTargetByPath = new Map(
    scanTargets
      .filter((target) => target.origin === "indexed")
      .map((target) => [scanPathKey(target.path), target]),
  );
  const manualTargets = scanTargets.filter((target) => target.origin === "manual");
  const displayedDirectories = [
    ...sourceDirs.map((source) => {
      const target = indexedTargetByPath.get(scanPathKey(scanTargetPathForSource(source)));
      return {
        id: `source:${source.id}`,
        name: source.displayName,
        path: source.scanRootPath,
        target: target ?? null,
        source,
      };
    }),
    ...manualTargets.map((target) => ({
      id: target.id,
      name: target.name,
      path: target.path,
      target,
      source: null,
    })),
  ];
  const enabledSourceCount = sourceDirs.filter((source) => source.enabled).length;
  const indexedClipCount = facets?.activeCount ?? accounts.reduce(
    (count, account) => count + account.clipCount,
    0,
  );
  const activeDefinition = SCAN_SECTIONS.find((section) => section.id === activeSection)
    ?? SCAN_SECTIONS[0];
  const visiblePendingClips = useMemo(
    () => pendingClips.filter((pending) => showIgnoredPending || !pending.ignored),
    [pendingClips, showIgnoredPending],
  );
  const activePendingCount = pendingClips.filter((pending) => !pending.ignored).length;
  const hasNvidiaSource = sourceDirs.some((source) => source.sourceKind === "nvidia");

  useEffect(() => {
    const startedScanning = isScanning && !wasScanningRef.current;
    wasScanningRef.current = isScanning;
    if (startedScanning) setActiveSection("task");
  }, [isScanning]);

  useEffect(() => {
    const progressJobId = progress?.jobId;
    if (!progressJobId || observedProgressJobRef.current === progressJobId) return;
    observedProgressJobRef.current = progressJobId;
    setActiveSection("task");
  }, [progress?.jobId]);

  useEffect(() => {
    if (previousSectionRef.current === activeSection) return;
    previousSectionRef.current = activeSection;
    const workspace = workspaceRef.current;
    if (workspace) {
      if (typeof workspace.scrollTo === "function") {
        workspace.scrollTo({ top: 0, behavior: "auto" });
      } else {
        workspace.scrollTop = 0;
      }
    }
    sectionHeadingRef.current?.focus();
  }, [activeSection]);

  const changeSection = (section: ScanWorkspaceSection) => {
    setActiveSection(section);
  };

  return (
    <section className="scan-workspace" aria-labelledby="scan-heading">
      <header className="scan-header">
        <div>
          <h1 id="scan-heading">扫描目录</h1>
          <p>管理录像来源、运行本地只读扫描，并查看自动识别的素材结果。</p>
        </div>
        <div
          className="scan-overview-badge"
          aria-label={`当前索引 ${indexedClipCount.toLocaleString("zh-CN")} 个素材，已配置 ${sourceDirs.length} 个视频来源`}
        >
          <span>当前索引</span>
          <strong>{indexedClipCount.toLocaleString("zh-CN")} 个素材</strong>
          <small>{sourceDirs.length} 个视频来源 · 本地只读</small>
        </div>
      </header>

      <div className="scan-layout">
        <nav className="scan-nav" aria-label="扫描分类">
          {SCAN_SECTIONS.map((section) => {
            const SectionIcon = section.icon;
            const isActive = activeSection === section.id;
            const needsAttention = section.id === "task"
              ? isScanning || Boolean(errorMessage) || freshnessSummary.needsAttention
              : section.id === "pending" && activePendingCount > 0;
            return (
              <button
                key={section.id}
                aria-current={isActive ? "page" : undefined}
                className="scan-nav-button"
                data-active={isActive || undefined}
                type="button"
                onClick={() => changeSection(section.id)}
              >
                <SectionIcon aria-hidden="true" weight={isActive ? "fill" : "regular"} />
                <span>
                  <strong>{section.label}</strong>
                  <small>{section.description}</small>
                </span>
                {needsAttention ? (
                  <em aria-label={
                    section.id === "pending"
                      ? `有 ${activePendingCount} 个 NVIDIA 视频待录入`
                      : isScanning
                        ? "扫描任务正在运行"
                        : "扫描任务需要注意"
                  } />
                ) : null}
              </button>
            );
          })}
        </nav>

        <div className="scan-category-picker">
          <span id="scan-category-label">扫描分类</span>
          <UiSelect value={activeSection} onValueChange={(value) => changeSection(value as ScanWorkspaceSection)}>
            <UiSelectTrigger aria-labelledby="scan-category-label">
              <UiSelectValue />
            </UiSelectTrigger>
            <UiSelectContent>
              {SCAN_SECTIONS.map((section) => (
                <UiSelectItem key={section.id} value={section.id}>{section.label}</UiSelectItem>
              ))}
            </UiSelectContent>
          </UiSelect>
        </div>

        <main ref={workspaceRef} className="scan-content">
          <section className="scan-section" aria-labelledby={`scan-section-${activeSection}`}>
            <header className="scan-section-heading">
              <h2 id={`scan-section-${activeSection}`} ref={sectionHeadingRef} tabIndex={-1}>
                {activeDefinition.label}
              </h2>
              <p>{activeDefinition.description}</p>
            </header>

            {activeSection === "sources" ? (
              <>
                {freshnessSummary.message ? (
                  <section className="scan-warning" role="status">
                    <WarningCircle weight="fill" />
                    <span><strong>来源需要扫描</strong>{freshnessSummary.message}</span>
                  </section>
                ) : null}

                <section className="scan-nvidia-entry" aria-labelledby="scan-nvidia-import-title">
                  <span className="scan-nvidia-entry-icon" aria-hidden="true">
                    <MonitorPlay weight="duotone" />
                  </span>
                  <div className="scan-nvidia-entry-copy">
                    <h3 id="scan-nvidia-import-title">导入 NVIDIA 录屏</h3>
                    <p id="scan-nvidia-import-description">
                      选择 NVIDIA App 保存录屏的 MP4 目录。仅在本机建立只读索引，不读取 NVIDIA 私有元数据。
                    </p>
                    {sourceWizardDisabledReason ? (
                      <span className="scan-nvidia-entry-status" role="status">
                        <WarningCircle weight="fill" />
                        {sourceWizardDisabledReason}
                      </span>
                    ) : null}
                  </div>
                  <div className="scan-nvidia-entry-actions">
                    <button
                      aria-describedby="scan-nvidia-import-description"
                      className="cinematic-button cinematic-button--primary"
                      type="button"
                      onClick={() => setIsSourceWizardOpen(true)}
                    >
                      <MonitorPlay weight="fill" />
                      {sourceWizardDisabledReason ? "查看导入说明" : "选择 NVIDIA 目录"}
                    </button>
                  </div>
                </section>

                <section className="cinematic-panel scan-directory-panel">
                  <header className="cinematic-section-heading">
                    <div>
                      <FolderOpen weight="duotone" />
                      <span>扫描目录</span>
                    </div>
                    <small>{sourceDirs.length} 个已索引来源 · {manualTargets.length} 个待扫描目录</small>
                  </header>
                  <div
                    aria-label="全部扫描目录"
                    className="scan-directory-grid"
                    role="region"
                    tabIndex={displayedDirectories.length > 0 ? 0 : undefined}
                  >
                    {displayedDirectories.map((directory) => (
                      <article
                        className="scan-directory-row"
                        data-accessible={directory.source?.accessibility ?? undefined}
                        data-enabled={directory.source?.enabled ?? undefined}
                        data-source-status={directory.source?.status}
                        key={directory.id}
                      >
                        <span className="scan-directory-icon"><FolderOpen weight="duotone" /></span>
                        <span>
                          <strong>{directory.name}</strong>
                          <small title={sourceDetailTitle(directory.source, directory.path)}>
                            {directory.path}
                          </small>
                          {directory.source ? (
                            <>
                              <small className="scan-source-kind">
                                {sourceKindLabel(directory.source.sourceKind)} · {directory.source.scanMode}
                              </small>
                              <small
                                className="scan-source-kind"
                                data-needs-attention={
                                  freshnessBySourceId.get(directory.source.id)?.needsAttention || undefined
                                }
                                title={freshnessIssueTitle(
                                  freshnessBySourceId.get(directory.source.id)?.issue ?? null,
                                )}
                              >
                                <Timer weight="bold" />
                                {freshnessBySourceId.get(directory.source.id)?.label}
                              </small>
                            </>
                          ) : null}
                        </span>
                        <em title={directory.source?.lastError ?? undefined}>
                          {directory.source ? sourceStatusLabel(directory.source) : "待扫描"}
                        </em>
                        {directory.source ? (
                          <span className="scan-source-actions">
                            <button
                              aria-label={`立即同步 ${directory.name}`}
                              disabled={isScanning}
                              title="立即同步"
                              type="button"
                              onClick={() => onSyncSource(directory.source!)}
                            >
                              <ArrowsClockwise weight="bold" />
                            </button>
                            <button
                              aria-label={`重新定位 ${directory.name}`}
                              disabled={isScanning || isLoading}
                              title="重新定位来源根目录"
                              type="button"
                              onClick={() => setRelocationSource(directory.source)}
                            >
                              <FolderOpen weight="bold" />
                            </button>
                            <button
                              aria-label={`${directory.source.enabled ? "移出" : "加入"}自动同步 ${directory.name}`}
                              aria-pressed={directory.source.enabled}
                              disabled={isScanning}
                              title={directory.source.enabled ? "移出自动同步" : "加入自动同步"}
                              type="button"
                              onClick={() => onSetSourceEnabled(directory.source!, !directory.source!.enabled)}
                            >
                              <Power weight={directory.source.enabled ? "fill" : "regular"} />
                            </button>
                          </span>
                        ) : directory.target ? (
                          <button
                            aria-label={`从扫描队列移除 ${directory.name}`}
                            disabled={isScanning}
                            type="button"
                            onClick={() => onRemoveDirectory(directory.target!)}
                          >
                            <X weight="bold" />
                          </button>
                        ) : null}
                      </article>
                    ))}
                    {displayedDirectories.length === 0 ? (
                      <p className="scan-inline-empty">先添加一个或多个录像目录，再进入扫描任务。</p>
                    ) : null}
                    <button
                      className="scan-directory-add"
                      disabled={isScanning || isLoading}
                      type="button"
                      onClick={() => setIsSourceWizardOpen(true)}
                    >
                      <Plus weight="bold" />
                      添加视频来源
                    </button>
                  </div>
                </section>
              </>
            ) : null}

            {activeSection === "task" ? (
              <>
                {errorMessage ? (
                  <section className="scan-warning" role="alert">
                    <WarningCircle weight="fill" />
                    <span><strong>扫描未完成</strong>{errorMessage}</span>
                  </section>
                ) : null}

                <section className="cinematic-panel scan-task-panel" aria-labelledby="scan-scope-heading">
                  <header className="cinematic-section-heading">
                    <div>
                      <ArrowsClockwise weight="duotone" />
                      <span id="scan-scope-heading">选择扫描范围</span>
                    </div>
                    <small>从精确范围到全盘发现</small>
                  </header>
                  <div className="scan-task-actions">
                    <article data-primary="true">
                      <span className="scan-task-action-icon"><Play weight="fill" /></span>
                      <div>
                        <strong>扫描当前目录</strong>
                        <small>{scanTargets.length > 0
                          ? `扫描已列出的 ${scanTargets.length} 个根目录。NVIDIA 视频会先进入待录入列表。`
                          : "尚未添加来源时，将尝试默认 ACLOS 目录。"}</small>
                      </div>
                      <button
                        className="cinematic-button cinematic-button--primary"
                        disabled={isScanning || isLoading}
                        type="button"
                        onClick={onStartScan}
                      >
                        <Play weight="fill" />
                        {scanStatus === "cancelling" ? "扫描取消中" : isScanning ? "正在扫描" : "开始扫描"}
                      </button>
                    </article>
                    <article>
                      <span className="scan-task-action-icon"><ArrowsClockwise weight="bold" /></span>
                      <div>
                        <strong>同步已启用来源</strong>
                        <small>同步 {enabledSourceCount} 个已启用的持久来源，NVIDIA 新视频会等待手动分类，适合日常更新。</small>
                      </div>
                      <button
                        className="cinematic-button cinematic-button--secondary"
                        disabled={isScanning || isLoading || enabledSourceCount === 0}
                        type="button"
                        onClick={onSyncEnabledSources}
                      >
                        <ArrowsClockwise weight="bold" />
                        同步全部来源
                      </button>
                    </article>
                    <article>
                      <span className="scan-task-action-icon"><HardDrives weight="duotone" /></span>
                      <div>
                        <strong>在全电脑中发现</strong>
                        <small>遍历本机固定磁盘，发现未登记素材后立即建立索引。</small>
                      </div>
                      <button
                        className="cinematic-button cinematic-button--secondary"
                        disabled={isScanning || isLoading}
                        type="button"
                        onClick={onDiscoverAll}
                      >
                        <HardDrives weight="bold" />
                        全电脑发现
                      </button>
                    </article>
                  </div>
                </section>

                <section className="scan-progress-panel" aria-live="polite" data-running={isScanning || undefined}>
                  <header>
                    <span className="scan-progress-icon" aria-hidden="true">
                      <MonitorPlay weight={isScanning ? "fill" : "duotone"} />
                    </span>
                    <div>
                      <small>{isScanning ? "当前任务" : "任务状态"}</small>
                      <strong>{scanStatus === "cancelling"
                        ? "正在完成安全取消…"
                        : isScanning
                          ? progress?.message ?? "正在分析录像内容…"
                          : terminalMessage ?? "尚未开始新的扫描"}</strong>
                    </div>
                    <b>{progressPercent === null ? isScanning ? "处理中" : "—" : `${progressPercent}%`}</b>
                  </header>
                  <div
                    aria-label="扫描进度"
                    aria-valuemax={100}
                    aria-valuemin={0}
                    aria-valuenow={progressPercent ?? undefined}
                    className={isScanning && !hasDeterminateProgress ? "cinematic-progress cinematic-progress--busy" : "cinematic-progress"}
                    role="progressbar"
                  >
                    <span style={progressPercent === null ? undefined : { width: `${progressPercent}%` }} />
                  </div>
                  <footer>
                    <span>{progress?.currentRoot
                      ? `当前目录：${progress.currentRoot}`
                      : terminalMessage ?? activityMessage}</span>
                    {isScanning ? (
                      <button
                        className="cinematic-button cinematic-button--secondary"
                        disabled={!activeJobId || scanStatus === "cancelling"}
                        type="button"
                        onClick={onCancelScan}
                      >
                        <Stop weight="fill" />
                        {scanStatus === "cancelling" ? "正在取消" : "取消扫描"}
                      </button>
                    ) : null}
                  </footer>
                </section>
              </>
            ) : null}

            {activeSection === "pending" ? (
              <>
                {!hasNvidiaSource ? (
                  <section className="scan-warning" role="status">
                    <WarningCircle weight="fill" />
                    <span>
                      <strong>还没有 NVIDIA 来源</strong>
                      先在“视频来源”中选择 NVIDIA 目录；首次同步后，待录入的视频会出现在这里。
                    </span>
                  </section>
                ) : null}

                <section className="cinematic-panel scan-pending-panel" aria-labelledby="scan-pending-heading">
                  <header className="cinematic-section-heading">
                    <div>
                      <ClipboardText weight="duotone" />
                      <span id="scan-pending-heading">待录入的 NVIDIA 视频</span>
                    </div>
                    <small>{activePendingCount} 个待分类 · 不会自动导入素材库</small>
                  </header>
                  <p className="scan-pending-hint">
                    NVIDIA 录屏没有可靠的对局元数据。首次同步、启动自动扫描或日常同步发现文件后，只会登记在这里，不会直接进入素材库；请为每条视频填写账户、英雄与地图，确认后才加入素材库。
                  </p>

                  {isPendingLoading && visiblePendingClips.length === 0 ? (
                    <p className="scan-inline-empty">正在加载待录入列表…</p>
                  ) : null}

                  <div
                    aria-label="待录入的 NVIDIA 视频列表"
                    className="scan-pending-grid"
                    role="region"
                    tabIndex={visiblePendingClips.length > 0 ? 0 : undefined}
                  >
                    {visiblePendingClips.map((pending) => (
                      <article className="scan-pending-row" data-ignored={pending.ignored || undefined} key={pending.id}>
                        <span className="scan-pending-icon" aria-hidden="true"><MonitorPlay weight="duotone" /></span>
                        <span className="scan-pending-copy">
                          <strong>{pending.fileName}</strong>
                          <small>
                            {pending.sourceDirName}
                            {pending.sourceRelativeDir ? ` · ${pending.sourceRelativeDir}` : ""}
                            {pending.modifiedAt ? ` · ${formatDateTime(pending.modifiedAt)}` : ""}
                            {` · ${formatBytes(pending.fileSize)}`}
                          </small>
                          {pending.ignored ? <em>已忽略，重新扫描不会自动录入</em> : null}
                        </span>
                        <span className="scan-pending-actions">
                          <button
                            className="cinematic-button cinematic-button--primary"
                            disabled={importingPendingId !== null || isScanning}
                            type="button"
                            onClick={() => setImportTarget(pending)}
                          >
                            <UserCircle weight="fill" />
                            录入
                          </button>
                          <button
                            className="cinematic-button cinematic-button--secondary"
                            disabled={importingPendingId !== null}
                            type="button"
                            onClick={() => onSetPendingIgnored(pending.id, !pending.ignored)}
                          >
                            {pending.ignored ? <Eye weight="bold" /> : <EyeSlash weight="bold" />}
                            {pending.ignored ? "恢复" : "忽略"}
                          </button>
                        </span>
                      </article>
                    ))}
                    {!isPendingLoading && visiblePendingClips.length === 0 ? (
                      <p className="scan-inline-empty">
                        还没有待录入的视频：首次同步、自动扫描或来源行同步发现的新录屏会登记到这里等待分类。
                      </p>
                    ) : null}
                  </div>

                  {pendingIgnoredCount > 0 || showIgnoredPending ? (
                    <footer className="scan-pending-footer">
                      <span>已忽略 {pendingIgnoredCount} 个视频</span>
                      <button type="button" onClick={onToggleShowIgnoredPending}>
                        {showIgnoredPending ? "隐藏已忽略" : "显示已忽略"}
                      </button>
                    </footer>
                  ) : null}
                </section>
              </>
            ) : null}

            {activeSection === "results" ? (
              <>
                <section className="cinematic-panel scan-statistics-panel">
                  <header className="cinematic-section-heading">
                    <div>
                      <ChartBar weight="duotone" />
                      <span>扫描统计</span>
                    </div>
                    <small>最近一次任务</small>
                  </header>
                  <div className="scan-stat-grid">
                    <ScanMetric
                      icon={<Database weight="duotone" />}
                      label="本次新增"
                      value={summary
                        ? summary.newClipCount.toLocaleString("zh-CN")
                        : terminalMessage
                          ? "—"
                          : (facets?.activeCount ?? 0).toLocaleString("zh-CN")}
                      suffix={summary ? "个" : undefined}
                      detail={terminalMessage ?? "当前索引"}
                    />
                    <ScanMetric icon={<UserCircle weight="duotone" />} label="识别账号" value={accounts.length.toLocaleString("zh-CN")} suffix="个" detail="自动归组" />
                    {((summary?.pendingClipCount ?? 0) > 0 || activePendingCount > 0) ? (
                      <ScanMetric
                        icon={<ClipboardText weight="duotone" />}
                        label="待录入 NVIDIA"
                        value={Math.max(summary?.pendingClipCount ?? 0, activePendingCount).toLocaleString("zh-CN")}
                        suffix="个"
                        detail={activePendingCount > 0 ? "到“待录入”手动补全分类" : "本次扫描新发现"}
                      />
                    ) : null}
                    <ScanMetric icon={<HardDrives weight="duotone" />} label="占用空间" value={formatBytes(totalSize)} detail={summary ? `${summary.sourceDirCount} 个来源` : "本地只读"} />
                    <ScanMetric icon={<Timer weight="duotone" />} label="最近素材" value={latestModifiedAt ? formatDateTime(latestModifiedAt) : "暂无记录"} compact detail={activityMessage} />
                  </div>
                </section>

                <section className="cinematic-panel scan-account-panel">
                  <header className="cinematic-section-heading">
                    <div>
                      <UserCircle weight="duotone" />
                      <span>检测到的账号</span>
                    </div>
                    <small>自动归组</small>
                  </header>
                  <div
                    aria-label="检测到的全部账号"
                    className="scan-account-grid"
                    role="region"
                    tabIndex={accounts.length > 0 ? 0 : undefined}
                  >
                    {accounts.map((account) => (
                      <article className="scan-account-card" key={account.id}>
                        <span className="scan-account-avatar"><UserCircle weight="fill" /></span>
                        <span>
                          <strong>{account.displayName}</strong>
                          <small>{account.clipCount.toLocaleString("zh-CN")} 个片段</small>
                        </span>
                      </article>
                    ))}
                    {accounts.length === 0 ? (
                      <p className="scan-inline-empty">完成首次扫描后，账号会自动显示在这里。</p>
                    ) : null}
                  </div>
                </section>

                <footer className="scan-results-footer">
                  <div>
                    <strong>{indexedClipCount > 0 ? "索引结果已可浏览" : "等待首次扫描结果"}</strong>
                    <span>{indexedClipCount > 0
                      ? `素材库中已有 ${indexedClipCount.toLocaleString("zh-CN")} 个本地素材。`
                      : "完成扫描后，可在素材库中按账号、对局与高光类型浏览。"}</span>
                  </div>
                  <button className="cinematic-button cinematic-button--primary" type="button" onClick={onOpenLibrary}>
                    进入素材库
                    <ArrowRight weight="bold" />
                  </button>
                </footer>
              </>
            ) : null}
          </section>
        </main>
      </div>

      <SourceWizardDialog
        initialSourceKind="nvidia"
        interactionDisabledReason={sourceWizardDisabledReason}
        open={isSourceWizardOpen}
        onChooseDirectory={onChooseSourceDirectory}
        onOpenChange={setIsSourceWizardOpen}
        onRegister={onRegisterSource}
      />
      <SourceRelocationDialog
        open={relocationSource !== null}
        source={relocationSource}
        onChooseDirectory={onChooseRelocationDirectory}
        onOpenChange={(open) => !open && setRelocationSource(null)}
        onPreview={onPreviewSourceRelocation}
        onRelocate={onRelocateSource}
      />
      <ManualClipImportDialog
        open={importTarget !== null}
        clip={importTarget}
        accounts={accounts}
        agentNames={manualAgentNames}
        mapNames={manualMapNames}
        gameModes={manualGameModes}
        isSubmitting={importingPendingId !== null}
        error={pendingError}
        onOpenChange={(open) => !open && setImportTarget(null)}
        onSubmit={(input) => {
          if (!importTarget) return;
          void onImportPendingClip(importTarget.id, input).then((imported) => {
            if (imported) setImportTarget(null);
          });
        }}
      />
    </section>
  );
}

function ScanMetric({ icon, label, value, suffix, detail, compact = false }: {
  icon: ReactNode;
  label: string;
  value: string;
  suffix?: string;
  detail: string;
  compact?: boolean;
}) {
  return (
    <article className={compact ? "scan-metric scan-metric--compact" : "scan-metric"}>
      <span className="scan-metric-label">{icon}{label}</span>
      <strong>{value}{suffix ? <small>{suffix}</small> : null}</strong>
      <span className="scan-metric-detail">{detail}</span>
    </article>
  );
}

function sourceKindLabel(kind: SourceKind): string {
  switch (kind) {
    case "nvidia": return "NVIDIA";
    case "tracker": return "Tracker";
    case "generic": return "普通目录";
    case "aclos": return "ACLOS";
  }
}

function sourceStatusLabel(source: SourceDir): string {
  if (!source.enabled) return "未加入自动同步";
  if (!source.accessibility) return source.status || "不可访问";
  return `${source.clipCount.toLocaleString("zh-CN")} 个片段`;
}

function sourceDetailTitle(source: SourceDir | null, path: string): string {
  if (!source) return path;
  return [
    path,
    `状态：${source.status}`,
    source.lastScanAt ? `最近扫描：${source.lastScanAt}` : "尚未扫描",
    source.lastError ? `错误：${source.lastError}` : "",
  ].filter(Boolean).join(" · ");
}

function isTerminalScanStatus(
  status: ScanJobStatus,
): status is Extract<ScanJobStatus, "completed" | "partial" | "cancelled" | "failed"> {
  return ["completed", "partial", "cancelled", "failed"].includes(status);
}

function freshnessIssueTitle(issue: "invalid" | "future" | null): string | undefined {
  if (issue === "invalid") return "最近扫描时间无效，已按尚未完成首次扫描处理";
  if (issue === "future") return "最近扫描时间位于未来，已按今天扫描处理";
  return undefined;
}
