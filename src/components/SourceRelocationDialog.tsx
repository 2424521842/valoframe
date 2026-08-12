import {
  ArrowLeft,
  ArrowRight,
  CheckCircle,
  Database,
  FolderOpen,
  WarningCircle,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  RelocateScanSourceResult,
  ScanSourceRelocationPreview,
  SourceDir,
} from "../types";
import {
  UiDialog,
  UiDialogClose,
  UiDialogContent,
  UiDialogDescription,
  UiDialogTitle,
} from "./ui/dialog";

type SourceRelocationDialogProps = {
  open: boolean;
  source: SourceDir | null;
  onOpenChange: (open: boolean) => void;
  onChooseDirectory: (source: SourceDir) => Promise<string | null>;
  onPreview: (
    sourceId: string,
    newRootPath: string,
  ) => Promise<ScanSourceRelocationPreview>;
  onRelocate: (
    sourceId: string,
    newRootPath: string,
  ) => Promise<RelocateScanSourceResult>;
};

type RelocationStep = "preview" | "confirm" | "result";

export function SourceRelocationDialog({
  open,
  source,
  onOpenChange,
  onChooseDirectory,
  onPreview,
  onRelocate,
}: SourceRelocationDialogProps) {
  const [step, setStep] = useState<RelocationStep>("preview");
  const [newRootPath, setNewRootPath] = useState("");
  const [preview, setPreview] = useState<ScanSourceRelocationPreview | null>(null);
  const [result, setResult] = useState<RelocateScanSourceResult | null>(null);
  const [isChoosingDirectory, setIsChoosingDirectory] = useState(false);
  const [isPreviewing, setIsPreviewing] = useState(false);
  const [isCommitting, setIsCommitting] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [previewError, setPreviewError] = useState("");
  const [commitError, setCommitError] = useState("");
  const previewRequestRef = useRef(0);

  useEffect(() => {
    previewRequestRef.current += 1;
    setStep("preview");
    setNewRootPath("");
    setPreview(null);
    setResult(null);
    setIsChoosingDirectory(false);
    setIsPreviewing(false);
    setIsCommitting(false);
    setFeedback("");
    setPreviewError("");
    setCommitError("");
  }, [open, source?.id]);

  const trustedMatchCount = useMemo(
    () => preview
      ? preview.exactPathMatchCount
        + preview.identityMatchCount
        + preview.legacyFingerprintMatchCount
      : 0,
    [preview],
  );
  const isBusy = isChoosingDirectory || isPreviewing || isCommitting;

  const loadPreview = async (path: string, requestId: number) => {
    if (!source || previewRequestRef.current !== requestId) return;
    setStep("preview");
    setNewRootPath(path);
    setPreview(null);
    setResult(null);
    setPreviewError("");
    setCommitError("");
    setFeedback("正在只读检查新目录与现有索引…");
    setIsPreviewing(true);
    try {
      const nextPreview = await onPreview(source.id, path);
      if (previewRequestRef.current !== requestId) return;
      setPreview(nextPreview);
      setFeedback(
        nextPreview.canRelocate
          ? "预览完成，可以进入提交确认。"
          : "预览完成，但存在必须先解决的阻断项。",
      );
    } catch (error) {
      if (previewRequestRef.current !== requestId) return;
      setPreviewError(`预览失败：${errorMessage(error)}`);
      setFeedback("预览没有修改任何来源或素材索引。");
    } finally {
      if (previewRequestRef.current === requestId) setIsPreviewing(false);
    }
  };

  const chooseDirectory = async () => {
    if (!source || isBusy) return;
    const requestId = previewRequestRef.current + 1;
    previewRequestRef.current = requestId;
    setIsChoosingDirectory(true);
    setPreviewError("");
    setCommitError("");
    try {
      const path = await onChooseDirectory(source);
      if (previewRequestRef.current !== requestId) return;
      if (!path) {
        setFeedback("已取消选择，新目录未预览，索引未发生变化。");
        return;
      }
      await loadPreview(path, requestId);
    } catch (error) {
      if (previewRequestRef.current !== requestId) return;
      setPreviewError(`无法选择目录：${errorMessage(error)}`);
      setFeedback("索引未发生变化。");
    } finally {
      if (previewRequestRef.current === requestId) setIsChoosingDirectory(false);
    }
  };

  const commitRelocation = async () => {
    if (!source || !preview?.canRelocate || isBusy) return;
    setIsCommitting(true);
    setCommitError("");
    try {
      const nextResult = await onRelocate(source.id, preview.newRootPath);
      setResult(nextResult);
      setPreview(nextResult.preview);
      setNewRootPath(nextResult.preview.newRootPath);
      setStep("result");
      setFeedback("");
    } catch (error) {
      setCommitError(`重新定位失败：${errorMessage(error)}`);
    } finally {
      setIsCommitting(false);
    }
  };

  const handleOpenChange = (nextOpen: boolean) => {
    if (!nextOpen && isBusy) return;
    if (!nextOpen) previewRequestRef.current += 1;
    onOpenChange(nextOpen);
  };

  return (
    <UiDialog open={open} onOpenChange={handleOpenChange}>
      <UiDialogContent className="source-relocation-dialog" closeDisabled={isBusy}>
        <header className="source-relocation-heading">
          <span><FolderOpen weight="duotone" /></span>
          <div>
            <UiDialogTitle>重新定位来源根目录</UiDialogTitle>
            <UiDialogDescription>
              {source?.displayName ?? "视频来源"} · 先只读预览，再明确确认提交。
            </UiDialogDescription>
          </div>
        </header>

        <div className="source-relocation-body">
          <dl className="source-relocation-paths">
            <div>
              <dt>当前根目录</dt>
              <dd title={source?.scanRootPath}>{source?.scanRootPath ?? "—"}</dd>
            </div>
            <ArrowRight aria-hidden="true" weight="bold" />
            <div>
              <dt>候选新根目录</dt>
              <dd title={newRootPath}>{newRootPath || "尚未选择"}</dd>
            </div>
          </dl>

          {step === "preview" ? (
            <>
              <p className="source-relocation-readonly-note">
                <Database weight="bold" />
                预览只读取目录与索引，不修改路径、不刷新上次完整扫描时间，也不会启动同步。
              </p>

              {isPreviewing ? (
                <section aria-live="polite" className="source-relocation-loading" role="status">
                  <span aria-hidden="true" />
                  <strong>正在生成重新定位预览</strong>
                  <small>检查相对路径、稳定文件身份、旧指纹、重叠和安全阻断项…</small>
                </section>
              ) : null}

              {previewError ? (
                <section className="source-relocation-alert" role="alert">
                  <WarningCircle weight="fill" />
                  <span><strong>无法完成预览</strong>{previewError}</span>
                </section>
              ) : null}

              {preview ? (
                <RelocationPreview
                  preview={preview}
                  trustedMatchCount={trustedMatchCount}
                />
              ) : !isPreviewing && !previewError ? (
                <section className="source-relocation-empty">
                  <FolderOpen weight="duotone" />
                  <strong>选择移动或改名后的来源根目录</strong>
                  <small>瓦刻不会搜索未授权位置，也不会移动目录中的任何文件。</small>
                </section>
              ) : null}
            </>
          ) : step === "confirm" && preview ? (
            <section className="source-relocation-confirmation">
              <div className="source-relocation-alert" role="alert">
                <WarningCircle weight="fill" />
                <span>
                  <strong>提交前最后确认</strong>
                  后端会重新枚举并验证目录；若状态已变化、出现重叠或关键任务占用，将安全拒绝且不写入。
                </span>
              </div>
              <ul>
                <li>预计原地更新 {preview.expectedClipUpdateCount.toLocaleString("zh-CN")} 条素材路径，并保留 clip ID、收藏、标签、备注、评审和结构化元数据。</li>
                <li>不会移动、复制、重命名或删除磁盘上的视频文件。</li>
                <li>本次提交不会刷新上次完整扫描时间；提交后才尝试启动现有来源同步。</li>
              </ul>
              {commitError ? (
                <p className="source-relocation-commit-error" role="alert">{commitError}</p>
              ) : null}
            </section>
          ) : step === "result" && result ? (
            <RelocationResult result={result} />
          ) : null}
        </div>

        <footer className="source-relocation-footer">
          <span aria-live="polite">{feedback}</span>
          <div>
            {step === "preview" ? (
              <>
                <UiDialogClose disabled={isBusy}>关闭</UiDialogClose>
                <button
                  className="ui-dialog-button"
                  disabled={isBusy}
                  type="button"
                  onClick={() => void chooseDirectory()}
                >
                  <FolderOpen weight="bold" />
                  {newRootPath ? "更换目录" : "选择新的根目录"}
                </button>
                <button
                  className="cinematic-button cinematic-button--primary"
                  disabled={isBusy || !preview?.canRelocate}
                  type="button"
                  onClick={() => setStep("confirm")}
                >
                  继续确认
                </button>
              </>
            ) : step === "confirm" ? (
              <>
                <button
                  className="ui-dialog-button"
                  disabled={isCommitting}
                  type="button"
                  onClick={() => setStep("preview")}
                >
                  <ArrowLeft weight="bold" />返回预览
                </button>
                <button
                  className="cinematic-button cinematic-button--primary"
                  disabled={isCommitting}
                  type="button"
                  onClick={() => void commitRelocation()}
                >
                  {isCommitting ? "正在重新验证并提交…" : "确认重新定位"}
                </button>
              </>
            ) : (
              <UiDialogClose>完成</UiDialogClose>
            )}
          </div>
        </footer>
      </UiDialogContent>
    </UiDialog>
  );
}

function RelocationPreview({
  preview,
  trustedMatchCount,
}: {
  preview: ScanSourceRelocationPreview;
  trustedMatchCount: number;
}) {
  const metrics = [
    ["可信匹配", trustedMatchCount],
    ["精确路径", preview.exactPathMatchCount],
    ["稳定身份", preview.identityMatchCount],
    ["旧指纹", preview.legacyFingerprintMatchCount],
    ["未匹配旧素材", preview.unmatchedCount],
    ["新候选", preview.newCandidateCount],
  ] as const;
  const expectedUpdates = [
    ["素材路径", preview.expectedClipUpdateCount],
    ["分组", preview.expectedGroupUpdateCount],
    ["封面引用", preview.expectedCoverUpdateCount],
    ["元数据引用", preview.expectedMetadataReferenceUpdateCount],
  ] as const;

  return (
    <div className="source-relocation-preview">
      <section
        className={preview.canRelocate
          ? "source-relocation-eligibility source-relocation-eligibility--ready"
          : "source-relocation-eligibility source-relocation-eligibility--blocked"}
        role="status"
      >
        {preview.canRelocate ? <CheckCircle weight="fill" /> : <WarningCircle weight="fill" />}
        <span>
          <strong>{preview.canRelocate ? "预览通过，可以提交" : "当前预览不可提交"}</strong>
          {trustedMatchCount === 0
            ? "未找到可信匹配；请选择正确的新根目录。"
            : `共识别 ${trustedMatchCount.toLocaleString("zh-CN")} 条可信匹配。`}
        </span>
      </section>

      <div aria-label="重新定位匹配统计" className="source-relocation-metrics">
        {metrics.map(([label, value]) => (
          <article key={label}>
            <span>{label}</span>
            <strong>{value.toLocaleString("zh-CN")}</strong>
          </article>
        ))}
      </div>

      <section className="source-relocation-section">
        <header><strong>受影响来源</strong><small>{preview.affectedSources.length} 个</small></header>
        <ul aria-label="受影响来源">
          {preview.affectedSources.map((affected) => (
            <li key={affected.id}>
              <strong>{affected.displayName}</strong>
              <span title={`${affected.oldSourcePath} → ${affected.newSourcePath}`}>
                {affected.oldSourcePath} → {affected.newSourcePath}
              </span>
              <small>{affected.clipCount.toLocaleString("zh-CN")} 条索引素材</small>
            </li>
          ))}
        </ul>
      </section>

      <section className="source-relocation-section">
        <header><strong>预计引用更新</strong></header>
        <dl className="source-relocation-update-counts">
          {expectedUpdates.map(([label, value]) => (
            <div key={label}><dt>{label}</dt><dd>{value.toLocaleString("zh-CN")}</dd></div>
          ))}
        </dl>
      </section>

      {preview.blockers.length > 0 ? (
        <section className="source-relocation-issues source-relocation-issues--blockers" role="alert">
          <header><WarningCircle weight="fill" /><strong>阻断项</strong></header>
          <ul>
            {preview.blockers.map((blocker, index) => (
              <li key={`${blocker.code}:${index}`}>
                <code>{blocker.code}</code><span>{blocker.message}</span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {preview.conflicts.length > 0 ? (
        <section className="source-relocation-issues" aria-label="重新定位冲突">
          <header><WarningCircle weight="fill" /><strong>冲突与歧义</strong></header>
          <ul>
            {preview.conflicts.map((conflict, index) => (
              <li key={`${conflict.code}:${index}`}>
                <code>{conflict.code}</code>
                <span>{conflict.message}</span>
                {conflict.oldClipIds.length > 0 ? (
                  <small>旧素材 ID：{conflict.oldClipIds.join("、")}</small>
                ) : null}
                {conflict.candidatePaths.length > 0 ? (
                  <small title={conflict.candidatePaths.join(" · ")}>
                    候选路径：{conflict.candidatePaths.join(" · ")}
                  </small>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

function RelocationResult({ result }: { result: RelocateScanSourceResult }) {
  const syncCompleted = result.syncStatus === "completed";
  const title = result.syncStatus === "completed"
    ? "重新定位成功，同步已完成"
    : result.syncStatus === "partial"
      ? "重新定位成功；同步部分完成，建议重试"
      : result.syncStatus === "cancelled"
        ? "重新定位成功；同步已取消，待重试"
        : result.syncStatus === "failed"
          ? "重新定位成功；同步失败，待重试"
          : "重新定位成功；同步尚未启动";
  return (
    <section
      className={syncCompleted
        ? "source-relocation-result source-relocation-result--synced"
        : "source-relocation-result source-relocation-result--pending"}
      role="status"
    >
      {syncCompleted ? <CheckCircle weight="fill" /> : <WarningCircle weight="fill" />}
      <div>
        <strong>{title}</strong>
        <p>
          已原地更新 {result.relocatedClipCount.toLocaleString("zh-CN")} 条素材索引，
          磁盘视频和用户整理状态均未改变。
        </p>
        {result.syncJobId ? (
          <p>
            {result.syncStatus === "failed" ? "失败/待重试任务" : "同步任务"}：
            <code>{result.syncJobId}</code>
          </p>
        ) : null}
        {result.syncMessage ? (
          <p>{result.syncStatus === null ? "同步信息" : "同步终态"}：{result.syncMessage}</p>
        ) : null}
        {result.syncStatus === null ? (
          <p>同步尚未启动；请关闭后在来源卡点击“立即同步”重试。</p>
        ) : !syncCompleted ? (
          <p>重新定位不会回滚；请关闭后在来源卡点击“立即同步”重试。</p>
        ) : null}
        <small>
          重新定位不会因同步终态而回滚；“上次完整扫描”时间只会在完整同步成功后刷新。
        </small>
      </div>
    </section>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  return "未知错误";
}
