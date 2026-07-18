import { MagnifyingGlass, Plus, Tag as TagIcon } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { clipTagSelectionState } from "../lib/clipSelection";
import { filterCustomTags } from "../lib/tags";
import type { ClipSummary, Tag, TagColor } from "../types";
import { UiCheckbox } from "./ui/checkbox";
import {
  UiDialog,
  UiDialogClose,
  UiDialogContent,
  UiDialogDescription,
  UiDialogTitle,
} from "./ui/dialog";
import { UiScrollArea } from "./ui/scroll-area";

type BatchTagDialogProps = {
  open: boolean;
  selectedClips: ClipSummary[];
  tags: Tag[];
  isBusy: boolean;
  onOpenChange: (open: boolean) => void;
  onCreateTag: (name: string, color?: TagColor) => Promise<Tag | null>;
  onSetTag: (tagId: string, shouldAttach: boolean) => Promise<boolean>;
};

export function BatchTagDialog({
  open,
  selectedClips,
  tags,
  isBusy,
  onOpenChange,
  onCreateTag,
  onSetTag,
}: BatchTagDialogProps) {
  const [query, setQuery] = useState("");
  const [newTagName, setNewTagName] = useState("");
  const [feedback, setFeedback] = useState("");
  const filteredTags = useMemo(() => filterCustomTags(tags, query), [query, tags]);
  const dialogContextRef = useRef(0);
  const feedbackRequestRef = useRef(0);
  const contextKeyRef = useRef("");
  const contextKey = `${open}:${selectedClips.map((clip) => clip.id).join(",")}`;
  if (contextKeyRef.current !== contextKey) {
    contextKeyRef.current = contextKey;
    dialogContextRef.current += 1;
  }

  useEffect(() => {
    if (!open) {
      setQuery("");
      setNewTagName("");
      setFeedback("");
      return;
    }
    setFeedback("");
  }, [contextKey, open]);

  const handleToggleTag = async (tag: Tag) => {
    const state = clipTagSelectionState(selectedClips, tag.id);
    const dialogContext = dialogContextRef.current;
    const requestId = feedbackRequestRef.current + 1;
    feedbackRequestRef.current = requestId;
    setFeedback("");
    const succeeded = await onSetTag(tag.id, state !== true);
    if (
      dialogContextRef.current !== dialogContext ||
      feedbackRequestRef.current !== requestId
    ) {
      return;
    }
    setFeedback(succeeded ? `已更新标签：${tag.label}` : `标签更新失败：${tag.label}`);
  };

  const handleCreate = async (event: FormEvent) => {
    event.preventDefault();
    const name = newTagName.trim();
    if (!name || isBusy) return;

    const dialogContext = dialogContextRef.current;
    const requestId = feedbackRequestRef.current + 1;
    feedbackRequestRef.current = requestId;
    setFeedback("");
    const tag = await onCreateTag(name, "red");
    if (!tag) {
      if (
        dialogContextRef.current === dialogContext &&
        feedbackRequestRef.current === requestId
      ) {
        setFeedback("创建标签失败");
      }
      return;
    }

    const attached = await onSetTag(tag.id, true);
    if (
      dialogContextRef.current !== dialogContext ||
      feedbackRequestRef.current !== requestId
    ) {
      return;
    }
    if (attached) {
      setNewTagName("");
      setFeedback(`已创建并应用：${tag.label}`);
    } else {
      setFeedback(`标签已创建，但应用到素材失败：${tag.label}`);
    }
  };

  return (
    <UiDialog open={open} onOpenChange={onOpenChange}>
      <UiDialogContent className="batch-tag-dialog">
        <header className="batch-dialog-heading">
          <span><TagIcon weight="duotone" /></span>
          <div>
            <UiDialogTitle>批量编辑自定义标签</UiDialogTitle>
            <UiDialogDescription>
              已选择 {selectedClips.length} 条素材；勾选会应用到全部素材，取消会从全部素材移除。
            </UiDialogDescription>
          </div>
        </header>

        <label className="batch-tag-search">
          <MagnifyingGlass weight="bold" />
          <input
            aria-label="搜索标签"
            autoComplete="off"
            placeholder="搜索标签…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>

        <UiScrollArea className="batch-tag-list">
          {filteredTags.length > 0 ? filteredTags.map((tag) => {
            const checked = clipTagSelectionState(selectedClips, tag.id);
            return (
              <label className="batch-tag-option" key={tag.id}>
                <UiCheckbox
                  aria-label={`${checked === true ? "移除" : "添加"}${tag.label}标签`}
                  checked={checked}
                  disabled={isBusy}
                  onCheckedChange={() => void handleToggleTag(tag)}
                />
                <span className={`tag tag--${tag.color}`}>{tag.label}</span>
                <small>{checked === true ? "全部已添加" : checked === "indeterminate" ? "部分已添加" : "尚未添加"}</small>
              </label>
            );
          }) : (
            <div className="batch-tag-empty">没有匹配的标签</div>
          )}
        </UiScrollArea>

        <form className="batch-tag-create" onSubmit={(event) => void handleCreate(event)}>
          <input
            aria-label="新标签名称"
            maxLength={24}
            placeholder="新建标签并应用到所选素材"
            value={newTagName}
            onChange={(event) => setNewTagName(event.target.value)}
          />
          <button disabled={isBusy || !newTagName.trim()} type="submit">
            <Plus weight="bold" />
            创建并应用
          </button>
        </form>

        <footer className="batch-dialog-footer">
          <span aria-live="polite">{isBusy ? "正在同步素材标签…" : feedback}</span>
          <UiDialogClose disabled={isBusy}>完成</UiDialogClose>
        </footer>
      </UiDialogContent>
    </UiDialog>
  );
}
