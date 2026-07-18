import {
  ArrowRight,
  ChartBar,
  Database,
  FolderOpen,
  HardDrives,
  Play,
  Plus,
  Stop,
  Timer,
  UserCircle,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { formatBytes, formatDateTime } from "../lib/formatters";
import type {
  AccountSummary,
  LibraryFacets,
  ScanProgress,
  ScanJobStatus,
  ScanSummary,
  ScanTarget,
  SourceDir,
} from "../types";
import { scanPathKey, scanRootFromSourcePath } from "../lib/scanTargets";

type ScanWorkspaceProps = {
  activeJobId: string | null;
  facets: LibraryFacets | null;
  accounts: AccountSummary[];
  scanTargets: ScanTarget[];
  sourceDirs: SourceDir[];
  isLoading: boolean;
  isScanning: boolean;
  progress: ScanProgress | null;
  scanStatus: ScanJobStatus;
  summary: ScanSummary | null;
  errorMessage: string | null;
  activityMessage: string;
  onAddDirectory: () => void;
  onCancelScan: () => void;
  onDiscoverAll: () => void;
  onOpenLibrary: () => void;
  onRemoveDirectory: (target: ScanTarget) => void;
  onStartScan: () => void;
};

export function ScanWorkspace({
  activeJobId,
  facets,
  accounts,
  scanTargets,
  sourceDirs,
  isLoading,
  isScanning,
  progress,
  scanStatus,
  summary,
  errorMessage,
  activityMessage,
  onAddDirectory,
  onCancelScan,
  onDiscoverAll,
  onOpenLibrary,
  onRemoveDirectory,
  onStartScan,
}: ScanWorkspaceProps) {
  const totalSize = facets?.activeSizeBytes ?? 0;
  const latestModifiedAt = facets?.modifiedAtMax
    ? new Date(facets.modifiedAtMax * 1_000).toISOString()
    : null;
  const hasDeterminateProgress = progress?.total !== null &&
    progress?.total !== undefined && progress.total > 0;
  const progressPercent = hasDeterminateProgress && progress
    ? Math.min(100, Math.round((progress.processed / progress.total!) * 100))
    : progress?.terminal
      ? 100
      : null;
  const indexedTargetByPath = new Map(
    scanTargets
      .filter((target) => target.origin === "indexed")
      .map((target) => [scanPathKey(target.path), target]),
  );
  const manualTargets = scanTargets.filter(
    (target) => target.origin === "manual",
  );
  const displayedDirectories = [
    ...sourceDirs.flatMap((source) => {
      const target = indexedTargetByPath.get(
        scanPathKey(scanRootFromSourcePath(source.path)),
      );
      return target
        ? [
          {
            id: `source:${source.id}`,
            name: source.displayName,
            path: source.path,
            target,
            source,
          },
        ]
        : [];
    }),
    ...manualTargets.map((target) => ({
      id: target.id,
      name: target.name,
      path: target.path,
      target,
      source: null,
    })),
  ];

  return (
    <section className="scan-workspace" aria-label="扫描目录">
      <header className="cinematic-page-heading">
        <div>
          <span className="cinematic-eyebrow">LIBRARY INITIALIZATION / 01</span>
          <h1>扫描战术影像</h1>
          <p>添加录像目录，终端将自动识别账号、对局与高光片段。</p>
        </div>
        <div className="scan-heading-actions">
          <button
            className="cinematic-button cinematic-button--secondary"
            disabled={isScanning || isLoading}
            type="button"
            onClick={onDiscoverAll}
          >
            <HardDrives weight="bold" />
            全电脑发现
          </button>
          <button
            className="cinematic-button cinematic-button--primary"
            disabled={isScanning || isLoading}
            type="button"
            onClick={onStartScan}
          >
            <Play weight="fill" />
            {scanStatus === "cancelling" ? "扫描取消中" : isScanning ? "正在扫描" : "开始扫描"}
          </button>
        </div>
      </header>

      <div className="scan-workspace-scroll">
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
                </span>
                <em title={directory.source?.lastError ?? undefined}>
                  {directory.source ? sourceStatusLabel(directory.source) : "待扫描"}
                </em>
                <button
                  aria-label={`从扫描队列移除 ${directory.name}`}
                  disabled={isScanning}
                  type="button"
                  onClick={() => onRemoveDirectory(directory.target)}
                >
                  <X weight="bold" />
                </button>
              </article>
            ))}
            {displayedDirectories.length === 0 ? (
              <p className="scan-inline-empty">先添加一个或多个录像目录，再统一开始扫描。</p>
            ) : null}
            <button
              className="scan-directory-add"
              disabled={isScanning || isLoading}
              type="button"
              onClick={onAddDirectory}
            >
              <Plus weight="bold" />
              添加目录
            </button>
          </div>
        </section>

        <section className="cinematic-panel scan-account-panel">
          <header className="cinematic-section-heading">
            <div>
              <UserCircle weight="duotone" />
              <span>检测到的账号</span>
            </div>
            <small>AUTO MATCHED</small>
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

        <section className="cinematic-panel scan-statistics-panel">
          <header className="cinematic-section-heading">
            <div>
              <ChartBar weight="duotone" />
              <span>扫描统计</span>
            </div>
            <small>LAST OPERATION</small>
          </header>
          <div className="scan-stat-grid">
            <ScanMetric icon={<Database weight="duotone" />} label="发现片段" value={(facets?.activeCount ?? 0).toLocaleString("zh-CN")} suffix="个" detail={summary ? `新增 ${summary.newClipCount}` : "当前索引"} />
            <ScanMetric icon={<UserCircle weight="duotone" />} label="识别账号" value={accounts.length.toLocaleString("zh-CN")} suffix="个" detail="自动归组" />
            <ScanMetric icon={<HardDrives weight="duotone" />} label="占用空间" value={formatBytes(totalSize)} detail={summary ? `${summary.sourceDirCount} 个来源` : "本地只读"} />
            <ScanMetric icon={<Timer weight="duotone" />} label="最近素材" value={latestModifiedAt ? formatDateTime(latestModifiedAt) : "暂无记录"} compact detail={activityMessage} />
          </div>
        </section>

        {errorMessage ? (
          <section className="scan-warning" role="alert">
            <WarningCircle weight="fill" />
            <span><strong>扫描未完成</strong>{errorMessage}</span>
          </section>
        ) : null}

        <section className="scan-progress-footer" aria-live="polite">
          <div className="scan-progress-copy">
            <span>{scanStatus === "cancelling" ? "正在完成安全取消…" : isScanning ? progress?.message ?? "正在分析录像内容…" : activityMessage}</span>
            <strong>{progressPercent === null ? "处理中" : `${progressPercent}%`}</strong>
          </div>
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
          <button className="cinematic-button cinematic-button--secondary" type="button" onClick={onOpenLibrary}>
            进入素材库
            <ArrowRight weight="bold" />
          </button>
        </section>
      </div>
    </section>
  );
}

type ScanMetricProps = {
  icon: ReactNode;
  label: string;
  value: string;
  suffix?: string;
  detail: string;
  compact?: boolean;
};

function ScanMetric({ icon, label, value, suffix, detail, compact = false }: ScanMetricProps) {
  return (
    <article className={compact ? "scan-metric scan-metric--compact" : "scan-metric"}>
      <span className="scan-metric-label">{icon}{label}</span>
      <strong>{value}{suffix ? <small>{suffix}</small> : null}</strong>
      <span className="scan-metric-detail">{detail}</span>
    </article>
  );
}

function sourceStatusLabel(source: SourceDir): string {
  if (!source.enabled) {
    return "已停用";
  }
  if (!source.accessibility) {
    return source.status || "不可访问";
  }
  return `${source.clipCount.toLocaleString("zh-CN")} 个片段`;
}

function sourceDetailTitle(source: SourceDir | null, path: string): string {
  if (!source) {
    return path;
  }

  return [
    path,
    `状态：${source.status}`,
    source.lastScanAt ? `最近扫描：${source.lastScanAt}` : "尚未扫描",
    source.lastError ? `错误：${source.lastError}` : "",
  ].filter(Boolean).join(" · ");
}
