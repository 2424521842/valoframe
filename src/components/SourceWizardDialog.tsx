import {
  Crosshair,
  FolderOpen,
  MonitorPlay,
  ShieldCheck,
  Sparkle,
  WarningCircle,
} from "@phosphor-icons/react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import type {
  RegisterScanSourceInput,
  RegisterScanSourceResult,
  SourceKind,
} from "../types";
import {
  UiDialog,
  UiDialogClose,
  UiDialogContent,
  UiDialogDescription,
  UiDialogTitle,
} from "./ui/dialog";

const SOURCE_OPTIONS: Array<{
  kind: SourceKind;
  title: string;
  description: string;
  icon: ReactNode;
}> = [
  {
    kind: "nvidia",
    title: "NVIDIA 录屏",
    description: "递归读取 NVIDIA 输出目录中的 MP4",
    icon: <MonitorPlay weight="duotone" />,
  },
  {
    kind: "tracker",
    title: "Tracker 录制",
    description: "只读取 Tracker 录制目录，不访问插件数据库",
    icon: <Crosshair weight="duotone" />,
  },
  {
    kind: "generic",
    title: "其他录制目录",
    description: "递归整理任意授权目录中的 MP4",
    icon: <FolderOpen weight="duotone" />,
  },
  {
    kind: "aclos",
    title: "ACLOS / 无畏时刻",
    description: "保留现有 wonderfulVideos 结构化扫描",
    icon: <Sparkle weight="duotone" />,
  },
];

type SourceWizardDialogProps = {
  initialSourceKind?: SourceKind;
  interactionDisabledReason?: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChooseDirectory: (kind: SourceKind) => Promise<string | null>;
  onRegister: (input: RegisterScanSourceInput) => Promise<RegisterScanSourceResult>;
};

export function SourceWizardDialog({
  initialSourceKind = "nvidia",
  interactionDisabledReason = null,
  open,
  onOpenChange,
  onChooseDirectory,
  onRegister,
}: SourceWizardDialogProps) {
  const [sourceKind, setSourceKind] = useState<SourceKind>(initialSourceKind);
  const [scanRootPath, setScanRootPath] = useState("");
  const [displayName, setDisplayName] = useState(sourceTitle(initialSourceKind));
  const [enabled, setEnabled] = useState(true);
  const [isBusy, setIsBusy] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [overlapResult, setOverlapResult] = useState<RegisterScanSourceResult | null>(null);
  const interactionDisabled = isBusy || Boolean(interactionDisabledReason);

  useEffect(() => {
    setSourceKind(initialSourceKind);
    setScanRootPath("");
    setDisplayName(sourceTitle(initialSourceKind));
    setEnabled(true);
    setIsBusy(false);
    setFeedback("");
    setOverlapResult(null);
  }, [initialSourceKind, open]);

  const chooseSourceKind = (kind: SourceKind) => {
    setSourceKind(kind);
    setDisplayName(SOURCE_OPTIONS.find((option) => option.kind === kind)?.title ?? "视频来源");
    setOverlapResult(null);
    setFeedback("");
  };

  const chooseDirectory = async () => {
    if (interactionDisabled) return;
    setFeedback("");
    const path = await onChooseDirectory(sourceKind);
    if (path) {
      setScanRootPath(path);
      setOverlapResult(null);
    }
  };

  const register = async (allowOverlap: boolean) => {
    if (!scanRootPath.trim() || !displayName.trim() || interactionDisabled) return;
    setIsBusy(true);
    setFeedback(allowOverlap ? "正在确认并注册重叠来源…" : "正在校验并注册来源…");
    try {
      const result = await onRegister({
        sourceKind,
        scanRootPath,
        displayName,
        enabled,
        allowOverlap,
      });
      if (result.requiresOverlapConfirmation) {
        setOverlapResult(result);
        setFeedback("所选目录与已有来源重叠，请确认后再继续。");
        return;
      }
      setFeedback(result.duplicateCount > 0 ? "已复用现有来源，正在后台同步。" : "来源已添加，正在后台首次同步。");
      onOpenChange(false);
    } catch (error) {
      setFeedback(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void register(false);
  };

  return (
    <UiDialog open={open} onOpenChange={(nextOpen) => !isBusy && onOpenChange(nextOpen)}>
      <UiDialogContent className="source-wizard-dialog">
        <header className="source-wizard-heading">
          <span><FolderOpen weight="duotone" /></span>
          <div>
            <UiDialogTitle>添加视频来源</UiDialogTitle>
            <UiDialogDescription>
              瓦刻只读索引本地 MP4，不会移动、改名或删除原视频。
            </UiDialogDescription>
          </div>
        </header>

        <form onSubmit={submit}>
          {interactionDisabledReason ? (
            <section className="source-wizard-unavailable" role="status">
              <WarningCircle weight="fill" />
              <div>
                <strong>当前暂不能选择目录</strong>
                <p>{interactionDisabledReason}</p>
              </div>
            </section>
          ) : null}

          <fieldset className="source-kind-grid" disabled={interactionDisabled}>
            <legend>选择来源类型</legend>
            {SOURCE_OPTIONS.map((option) => (
              <button
                aria-pressed={sourceKind === option.kind}
                className={sourceKind === option.kind ? "source-kind-card source-kind-card--active" : "source-kind-card"}
                key={option.kind}
                type="button"
                onClick={() => chooseSourceKind(option.kind)}
              >
                <span>{option.icon}</span>
                <strong>{option.title}</strong>
                <small>{option.description}</small>
              </button>
            ))}
          </fieldset>

          {sourceKind === "nvidia" ? (
            <p className="source-wizard-source-note">
              <ShieldCheck weight="fill" />
              <span>选择 NVIDIA App 保存录屏的 MP4 目录；瓦刻只读索引视频文件，不会读取 NVIDIA 私有元数据。</span>
            </p>
          ) : null}

          <label className="source-wizard-field">
            <span>扫描根目录</span>
            <div>
              <input
                aria-label="扫描根目录"
                disabled={interactionDisabled}
                readOnly
                value={scanRootPath}
                placeholder={sourceKind === "nvidia"
                  ? "请选择 NVIDIA App 的 MP4 保存目录"
                  : "请选择第三方录制输出目录"}
              />
              <button disabled={interactionDisabled} type="button" onClick={() => void chooseDirectory()}>
                <FolderOpen weight="bold" />选择目录
              </button>
            </div>
          </label>

          <label className="source-wizard-field">
            <span>显示名称</span>
              <input
                aria-label="来源显示名称"
                disabled={interactionDisabled}
              maxLength={80}
              value={displayName}
              onChange={(event) => setDisplayName(event.currentTarget.value)}
            />
          </label>

          <label className="source-wizard-toggle">
            <input
              checked={enabled}
              disabled={interactionDisabled}
              type="checkbox"
              onChange={(event) => setEnabled(event.currentTarget.checked)}
            />
            <span>
              <strong>应用启动时同步</strong>
              <small>后台增量检查；关闭后仍可手动“立即同步”</small>
            </span>
          </label>

          {overlapResult ? (
            <section className="source-overlap-warning" role="alert">
              <WarningCircle weight="fill" />
              <div>
                <strong>目录与已有来源重叠</strong>
                <p>重叠目录可能扫描到同一视频；同一文件始终只归属一个来源。</p>
                <ul>
                  {overlapResult.overlaps.map((overlap) => (
                    <li key={overlap.id}>{overlap.displayName} · {overlap.scanRootPath}</li>
                  ))}
                </ul>
                <button disabled={isBusy} type="button" onClick={() => void register(true)}>
                  确认重叠并继续
                </button>
              </div>
            </section>
          ) : null}

          <footer className="source-wizard-footer">
            <span aria-live="polite">{feedback}</span>
            <div>
              <UiDialogClose disabled={isBusy}>取消</UiDialogClose>
              <button
                className="cinematic-button cinematic-button--primary"
                disabled={interactionDisabled || !scanRootPath.trim() || !displayName.trim()}
                type="submit"
              >
                {isBusy ? "正在处理…" : "添加并首次同步"}
              </button>
            </div>
          </footer>
        </form>
      </UiDialogContent>
    </UiDialog>
  );
}

function sourceTitle(kind: SourceKind): string {
  return SOURCE_OPTIONS.find((option) => option.kind === kind)?.title ?? "视频来源";
}
