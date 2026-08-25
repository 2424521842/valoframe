import { MonitorPlay, Plus, UserCircle, WarningCircle } from "@phosphor-icons/react";
import { useEffect, useState, type FormEvent } from "react";
import { pendingMediaUrlForId } from "../api/backend";
import { formatBytes } from "../lib/formatters";
import type { AccountSummary, ManualClipImportInput, PendingManualClip } from "../types";
import {
  EMPTY_MANUAL_IMPORT_FORM,
  NEW_ACCOUNT_OPTION_VALUE,
  manualImportInputFromForm,
  validateManualImportForm,
  type ManualImportFormErrors,
  type ManualImportFormState,
} from "../lib/manualImport";
import {
  UiDialog,
  UiDialogClose,
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

const EMPTY_GAME_MODE_VALUE = "__none__";

type ManualClipImportDialogProps = {
  open: boolean;
  clip: PendingManualClip | null;
  accounts: readonly AccountSummary[];
  mapNames: readonly string[];
  agentNames: readonly string[];
  gameModes: readonly string[];
  isSubmitting: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (input: ManualClipImportInput) => void;
};

export function ManualClipImportDialog({
  open,
  clip,
  accounts,
  mapNames,
  agentNames,
  gameModes,
  isSubmitting,
  error,
  onOpenChange,
  onSubmit,
}: ManualClipImportDialogProps) {
  const [form, setForm] = useState<ManualImportFormState>(EMPTY_MANUAL_IMPORT_FORM);
  const [fieldErrors, setFieldErrors] = useState<ManualImportFormErrors>({});
  const [previewFailed, setPreviewFailed] = useState(false);

  useEffect(() => {
    if (!open) return;
    setForm(EMPTY_MANUAL_IMPORT_FORM);
    setFieldErrors({});
    setPreviewFailed(false);
  }, [open, clip?.id]);

  const setField = <K extends keyof ManualImportFormState>(
    field: K,
    value: ManualImportFormState[K],
  ) => setForm((current) => ({ ...current, [field]: value }));

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (isSubmitting) return;
    const nextErrors = validateManualImportForm(form, accounts);
    setFieldErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) return;
    onSubmit(manualImportInputFromForm(form, accounts));
  };

  const accountMode = form.accountMode;
  const accountOptions = accounts.map((account) => ({
    id: account.id,
    label: account.displayName || account.id,
  }));

  return (
    <UiDialog open={open} onOpenChange={(nextOpen) => !isSubmitting && onOpenChange(nextOpen)}>
      <UiDialogContent className="manual-import-dialog">
        <header className="manual-import-heading">
          <span aria-hidden="true"><MonitorPlay weight="duotone" /></span>
          <div>
            <UiDialogTitle>录入 NVIDIA 视频</UiDialogTitle>
            <UiDialogDescription>
              NVIDIA 录屏没有对局元数据，请手动填写分类后再加入素材库。
            </UiDialogDescription>
          </div>
        </header>

        {clip ? (
          <section className="manual-import-file" aria-label="待录入文件">
            <video
              className="manual-import-video"
              controls
              preload="metadata"
              // Keyed so switching pending rows reloads the source instead of keeping the old
              // buffer, and reset on close so no stream stays open behind a hidden dialog.
              key={clip.id}
              src={pendingMediaUrlForId(clip.id)}
              onError={() => setPreviewFailed(true)}
              onLoadedMetadata={() => setPreviewFailed(false)}
            />
            {previewFailed ? (
              <p className="manual-import-video-error" role="status">
                <WarningCircle weight="fill" />
                无法在应用内预览该视频，可能是当前 WebView2 解码链不支持；分类信息仍可正常填写。
              </p>
            ) : null}
            <strong>{clip.fileName}</strong>
            <small>
              {clip.sourceDirName}
              {clip.sourceRelativeDir ? ` · ${clip.sourceRelativeDir}` : ""}
              {` · ${formatBytes(clip.fileSize)}`}
            </small>
          </section>
        ) : null}

        <form className="manual-import-form" onSubmit={submit}>
          <label className="manual-import-field">
            <span>账户</span>
            <UiSelect
              value={form.accountMode === "new" ? NEW_ACCOUNT_OPTION_VALUE : form.accountId}
              onValueChange={(value) => {
                if (value === NEW_ACCOUNT_OPTION_VALUE) {
                  setForm((current) => ({
                    ...current,
                    accountMode: "new",
                    accountId: "",
                  }));
                } else {
                  setForm((current) => ({
                    ...current,
                    accountMode: "existing",
                    accountId: value,
                  }));
                }
              }}
            >
              <UiSelectTrigger aria-label="选择账户">
                <UiSelectValue placeholder="选择已有账户" />
              </UiSelectTrigger>
              <UiSelectContent>
                {accountOptions.map((option) => (
                  <UiSelectItem key={option.id} value={option.id}>{option.label}</UiSelectItem>
                ))}
                <UiSelectItem value={NEW_ACCOUNT_OPTION_VALUE}>
                  <span className="manual-import-new-option"><Plus weight="bold" />新添加账户…</span>
                </UiSelectItem>
              </UiSelectContent>
            </UiSelect>
            {accountMode === "existing" && fieldErrors.accountName ? (
              <small className="manual-import-error">{fieldErrors.accountName}</small>
            ) : null}
          </label>

          {accountMode === "new" ? (
            <label className="manual-import-field">
              <span>新账户名称</span>
              <input
                aria-invalid={Boolean(fieldErrors.accountName) || undefined}
                className="manual-import-input"
                placeholder="例如：小号#1234"
                type="text"
                value={form.newAccountName}
                onChange={(event) => setField("newAccountName", event.target.value)}
              />
              {fieldErrors.accountName ? (
                <small className="manual-import-error">{fieldErrors.accountName}</small>
              ) : null}
            </label>
          ) : null}

          <label className="manual-import-field">
            <span>英雄</span>
            <UiSelect value={form.agentName} onValueChange={(value) => setField("agentName", value)}>
              <UiSelectTrigger aria-label="选择英雄" aria-invalid={Boolean(fieldErrors.agentName) || undefined}>
                <UiSelectValue placeholder="选择本局使用的英雄" />
              </UiSelectTrigger>
              <UiSelectContent>
                {agentNames.map((agentName) => (
                  <UiSelectItem key={agentName} value={agentName}>{agentName}</UiSelectItem>
                ))}
              </UiSelectContent>
            </UiSelect>
            {fieldErrors.agentName ? (
              <small className="manual-import-error">{fieldErrors.agentName}</small>
            ) : null}
          </label>

          <label className="manual-import-field">
            <span>地图</span>
            <UiSelect value={form.mapName} onValueChange={(value) => setField("mapName", value)}>
              <UiSelectTrigger aria-label="选择地图" aria-invalid={Boolean(fieldErrors.mapName) || undefined}>
                <UiSelectValue placeholder="选择本局地图" />
              </UiSelectTrigger>
              <UiSelectContent>
                {mapNames.map((mapName) => (
                  <UiSelectItem key={mapName} value={mapName}>{mapName}</UiSelectItem>
                ))}
              </UiSelectContent>
            </UiSelect>
            {fieldErrors.mapName ? (
              <small className="manual-import-error">{fieldErrors.mapName}</small>
            ) : null}
          </label>

          <label className="manual-import-field">
            <span>模式（可选）</span>
            <UiSelect
              value={form.gameMode || EMPTY_GAME_MODE_VALUE}
              onValueChange={(value) => setField("gameMode", value === EMPTY_GAME_MODE_VALUE ? "" : value)}
            >
              <UiSelectTrigger aria-label="选择模式">
                <UiSelectValue placeholder="不填写" />
              </UiSelectTrigger>
              <UiSelectContent>
                <UiSelectItem value={EMPTY_GAME_MODE_VALUE}>不填写</UiSelectItem>
                {gameModes.map((gameMode) => (
                  <UiSelectItem key={gameMode} value={gameMode}>{gameMode}</UiSelectItem>
                ))}
              </UiSelectContent>
            </UiSelect>
          </label>

          <label className="manual-import-field">
            <span>备注（可选）</span>
            <textarea
              className="manual-import-input manual-import-textarea"
              placeholder="例如：第三回合的残局反杀"
              rows={2}
              value={form.note}
              onChange={(event) => setField("note", event.target.value)}
            />
          </label>

          {error ? (
            <p className="manual-import-error manual-import-submit-error" role="alert">
              <WarningCircle weight="fill" />
              {error}
            </p>
          ) : null}

          <footer className="manual-import-actions">
            <UiDialogClose type="button">取消</UiDialogClose>
            <button
              className="cinematic-button cinematic-button--primary"
              disabled={isSubmitting}
              type="submit"
            >
              <UserCircle weight="fill" />
              {isSubmitting ? "正在录入…" : "录入到素材库"}
            </button>
          </footer>
        </form>
      </UiDialogContent>
    </UiDialog>
  );
}
