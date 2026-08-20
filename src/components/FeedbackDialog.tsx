import {
  CheckCircle,
  PaperPlaneTilt,
  SpinnerGap,
  WarningCircle,
} from "@phosphor-icons/react";
import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import {
  commandErrorMessage,
  discardFeedbackPackage,
  listenToFeedbackProgress,
  saveFeedbackPackage,
  submitFeedback,
} from "../api/backend";
import { formatBytes } from "../lib/formatters";
import {
  FEEDBACK_CATEGORY_OPTIONS,
  MAX_FEEDBACK_CONTACT_CHARS,
  MAX_FEEDBACK_DESCRIPTION_CHARS,
  MAX_FEEDBACK_VIDEO_BYTES,
} from "../lib/feedback";
import type {
  Clip,
  FeedbackCategory,
  FeedbackProgress,
  FeedbackSubmitResult,
} from "../types";
import { UiCheckbox } from "./ui/checkbox";
import {
  UiDialog,
  UiDialogContent,
  UiDialogDescription,
  UiDialogTitle,
} from "./ui/dialog";
import {
  UiSelect,
  UiSelectContent,
  UiSelectItem,
  UiSelectTrigger,
  UiSelectValue,
} from "./ui/select";

type FeedbackDialogProps = {
  open: boolean;
  clip: Clip | null;
  endpoint: string;
  onOpenChange: (open: boolean) => void;
};

type FeedbackPhase = "idle" | "submitting" | "success" | "saved" | "error";

export function FeedbackDialog({
  open,
  clip,
  endpoint,
  onOpenChange,
}: FeedbackDialogProps) {
  const [category, setCategory] = useState<FeedbackCategory>("mismatch");
  const [description, setDescription] = useState("");
  const [contact, setContact] = useState("");
  const [includeFrames, setIncludeFrames] = useState(true);
  const [includeVideo, setIncludeVideo] = useState(false);
  const [phase, setPhase] = useState<FeedbackPhase>("idle");
  const [progress, setProgress] = useState<FeedbackProgress | null>(null);
  const [errorMessage, setErrorMessage] = useState("");
  const [result, setResult] = useState<FeedbackSubmitResult | null>(null);
  const [savedPath, setSavedPath] = useState("");
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (open) return;
    unlistenRef.current?.();
    unlistenRef.current = null;
    setCategory("mismatch");
    setDescription("");
    setContact("");
    setIncludeFrames(true);
    setIncludeVideo(false);
    setPhase("idle");
    setProgress(null);
    setErrorMessage("");
    setResult(null);
    setSavedPath("");
  }, [open]);

  useEffect(() => () => {
    unlistenRef.current?.();
  }, []);

  const busy = phase === "submitting";
  const uploading = busy && progress?.phase === "uploading";
  const building = busy && progress?.phase === "building";
  const percent = progress && progress.totalBytes > 0
    ? Math.min(100, Math.round((progress.uploadedBytes / progress.totalBytes) * 100))
    : 0;
  const videoAttachDisabled = !clip
    || clip.fileStatus !== "available"
    || clip.sizeBytes > MAX_FEEDBACK_VIDEO_BYTES;
  const videoAttachHint = !clip
    ? ""
    : clip.fileStatus !== "available"
      ? "文件当前不可用，无法附带"
      : clip.sizeBytes > MAX_FEEDBACK_VIDEO_BYTES
        ? "文件超过 1 GiB，无法附带"
        : "会显著增大体积，仅在需要时勾选";
  const trimmedEndpoint = endpoint.trim();

  const handleSubmit = async () => {
    if (!clip || busy) return;
    const trimmedDescription = description.trim();
    if (!trimmedDescription) {
      setErrorMessage("请描述你遇到的问题");
      return;
    }
    setErrorMessage("");
    setPhase("submitting");
    setProgress(null);
    setResult(null);
    setSavedPath("");
    unlistenRef.current?.();
    unlistenRef.current = await listenToFeedbackProgress((next) => {
      setProgress(next);
    }).catch(() => null);
    try {
      const next = await submitFeedback({
        clipId: clip.id,
        category,
        description: trimmedDescription,
        contact: contact.trim(),
        includeFrames,
        includeVideo: includeVideo && !videoAttachDisabled,
        endpoint: trimmedEndpoint,
      });
      if (next.status === "uploaded") {
        setResult(next);
        setPhase("success");
      } else {
        await handleSaveFallback(next);
      }
    } catch (error) {
      setErrorMessage(commandErrorMessage(error));
      setPhase("error");
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
    }
  };

  const handleSaveFallback = async (next: FeedbackSubmitResult) => {
    let destination: string | null = null;
    try {
      destination = await save({
        title: "保存问题诊断包",
        defaultPath: next.suggestedFileName ?? "valoframe-feedback.zip",
        filters: [{ name: "问题诊断包", extensions: ["zip"] }],
      });
    } catch {
      destination = null;
    }
    if (destination && next.packagePath) {
      try {
        const saved = await saveFeedbackPackage(next.packagePath, destination);
        setResult(next);
        setSavedPath(saved.destinationPath);
        setPhase("saved");
      } catch (error) {
        setErrorMessage(commandErrorMessage(error));
        setPhase("error");
      }
      return;
    }
    if (next.packagePath) {
      void discardFeedbackPackage(next.packagePath);
    }
    setResult(next);
    setErrorMessage(next.uploadError
      ? `自动上传失败：${next.uploadError}`
      : "已取消提交；诊断包未保存。");
    setPhase("error");
  };

  const finished = phase === "success" || phase === "saved";

  return (
    <UiDialog open={open} onOpenChange={onOpenChange}>
      <UiDialogContent className="feedback-dialog">
        <UiDialogTitle>反馈问题</UiDialogTitle>
        <UiDialogDescription>
          上报视频与信息不匹配等问题，帮助改进瓦刻。
        </UiDialogDescription>
        {clip && finished && result ? (
          <div className="feedback-result" role="status">
            <CheckCircle aria-hidden="true" weight="fill" />
            <div>
              <strong>{result.message}</strong>
              <span>报告编号：{result.reportId}</span>
              {phase === "saved" && savedPath ? (
                <span className="feedback-result-path">
                  已保存到 {savedPath}，请通过 QQ 群、邮件或网盘发送给开发者。
                </span>
              ) : null}
            </div>
            <button
              className="cinematic-button cinematic-button--primary"
              type="button"
              onClick={() => onOpenChange(false)}
            >
              完成
            </button>
          </div>
        ) : clip ? (
          <>
            <p className="feedback-context">
              当前片段：<strong>{clip.fileName}</strong> · {formatBytes(clip.sizeBytes)}
            </p>
            <fieldset className="feedback-form" disabled={busy}>
              <label className="feedback-field">
                <span>问题类别</span>
                <UiSelect
                  value={category}
                  onValueChange={(value) => setCategory(value as FeedbackCategory)}
                >
                  <UiSelectTrigger aria-label="问题类别">
                    <UiSelectValue />
                  </UiSelectTrigger>
                  <UiSelectContent>
                    {FEEDBACK_CATEGORY_OPTIONS.map((option) => (
                      <UiSelectItem key={option.value} value={option.value}>
                        {option.label}
                      </UiSelectItem>
                    ))}
                  </UiSelectContent>
                </UiSelect>
                <small>
                  {FEEDBACK_CATEGORY_OPTIONS.find((option) => option.value === category)?.hint}
                </small>
              </label>
              <label className="feedback-field">
                <span>问题描述（必填）</span>
                <textarea
                  className="feedback-textarea"
                  maxLength={MAX_FEEDBACK_DESCRIPTION_CHARS}
                  placeholder="例如：这个片段显示的是另一局游戏的内容，击杀信息和画面对不上……"
                  rows={4}
                  value={description}
                  onChange={(event) => setDescription(event.currentTarget.value)}
                />
                <small>{description.length}/{MAX_FEEDBACK_DESCRIPTION_CHARS}</small>
              </label>
              <label className="feedback-field">
                <span>联系方式（选填）</span>
                <input
                  className="feedback-input"
                  maxLength={MAX_FEEDBACK_CONTACT_CHARS}
                  placeholder="QQ 号或邮箱，便于联系你核对问题"
                  type="text"
                  value={contact}
                  onChange={(event) => setContact(event.currentTarget.value)}
                />
              </label>
              <div className="feedback-attachments">
                <label className="feedback-check">
                  <UiCheckbox
                    checked={includeFrames}
                    id="feedback-include-frames"
                    onCheckedChange={(checked) => setIncludeFrames(checked === true)}
                  />
                  <span>
                    <strong>附带 3 张采样帧</strong>
                    <small>从视频头 / 中 / 尾自动截取，帮助定位画面</small>
                  </span>
                </label>
                <label
                  className={videoAttachDisabled
                    ? "feedback-check feedback-check--disabled"
                    : "feedback-check"}
                >
                  <UiCheckbox
                    checked={includeVideo}
                    disabled={videoAttachDisabled}
                    id="feedback-include-video"
                    onCheckedChange={(checked) => setIncludeVideo(checked === true)}
                  />
                  <span>
                    <strong>附带完整视频（{formatBytes(clip.sizeBytes)}）</strong>
                    <small>{videoAttachHint}</small>
                  </span>
                </label>
              </div>
              <div className="feedback-privacy">
                <WarningCircle aria-hidden="true" />
                <div>
                  <strong>将包含：</strong>
                  对局与素材信息、来源结构、文件信息、应用版本
                  {includeFrames ? "、采样帧" : ""}
                  {includeVideo ? "、视频本体" : ""}。
                  <br />
                  <strong>不包含：</strong>
                  账号 OpenID / PUUID、备注、标签与本机绝对路径。
                  提交即表示同意将这些数据提供给开发者用于问题分析。
                </div>
              </div>
            </fieldset>
            {errorMessage ? (
              <p className="feedback-error" role="alert">{errorMessage}</p>
            ) : null}
            {busy ? (
              <div className="feedback-progress" role="status">
                <SpinnerGap aria-hidden="true" className="feedback-spinner" />
                <span>
                  {uploading
                    ? `正在上传诊断包… ${percent}%`
                    : building && progress?.message
                      ? progress.message
                      : "正在准备诊断包…"}
                </span>
                {uploading ? (
                  <div
                    aria-valuemax={100}
                    aria-valuemin={0}
                    aria-valuenow={percent}
                    className="feedback-progress-track"
                    role="progressbar"
                  >
                    <div
                      className="feedback-progress-fill"
                      style={{ width: `${percent}%` }}
                    />
                  </div>
                ) : null}
              </div>
            ) : null}
            <div className="feedback-actions">
              <button
                className="cinematic-button cinematic-button--secondary"
                disabled={busy}
                type="button"
                onClick={() => onOpenChange(false)}
              >
                取消
              </button>
              <button
                className="cinematic-button cinematic-button--primary"
                disabled={busy || !description.trim()}
                type="button"
                onClick={() => void handleSubmit()}
              >
                <PaperPlaneTilt aria-hidden="true" weight="bold" />
                {trimmedEndpoint ? "上传反馈" : "生成诊断包"}
              </button>
            </div>
          </>
        ) : null}
      </UiDialogContent>
    </UiDialog>
  );
}
