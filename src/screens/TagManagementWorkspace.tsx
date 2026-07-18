import {
  ArrowLeft,
  FloppyDisk,
  FunnelSimple,
  MagnifyingGlass,
  PencilSimple,
  Plus,
  Tag as TagIcon,
  Trash,
  X,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  TAG_COLORS,
  filterCustomTags,
} from "../lib/tags";
import type { Tag, TagColor } from "../types";

type TagManagementWorkspaceProps = {
  activityMessage: string;
  taggedClipCount: number;
  tagUsageCounts: ReadonlyMap<string, number>;
  tags: Tag[];
  totalClipCount: number;
  onBack: () => void;
  onCreateTag: (name: string, color: TagColor) => Promise<Tag | null>;
  onDeleteTag: (tagId: string) => Promise<boolean>;
  onUpdateTag: (
    tagId: string,
    name: string,
    color: TagColor,
  ) => Promise<Tag | null>;
  onViewTag: (tagId: string) => void;
};

const COLOR_LABELS: Record<TagColor, string> = {
  red: "赤红",
  teal: "青绿",
  gold: "金色",
  blue: "蓝色",
  green: "绿色",
};

export function TagManagementWorkspace({
  activityMessage,
  taggedClipCount,
  tagUsageCounts,
  tags,
  totalClipCount,
  onBack,
  onCreateTag,
  onDeleteTag,
  onUpdateTag,
  onViewTag,
}: TagManagementWorkspaceProps) {
  const [query, setQuery] = useState("");
  const [newTagName, setNewTagName] = useState("");
  const [newTagColor, setNewTagColor] = useState<TagColor>("blue");
  const [editingTagId, setEditingTagId] = useState("");
  const [editName, setEditName] = useState("");
  const [editColor, setEditColor] = useState<TagColor>("blue");
  const [pendingTagId, setPendingTagId] = useState("");
  const [deleteCandidateId, setDeleteCandidateId] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [formError, setFormError] = useState("");

  const visibleTags = useMemo(
    () => filterCustomTags(tags, query),
    [query, tags],
  );
  const usedTagCount = tags.filter((tag) => (tagUsageCounts.get(tag.id) ?? 0) > 0).length;

  useEffect(() => {
    if (editingTagId && !tags.some((tag) => tag.id === editingTagId)) {
      setEditingTagId("");
    }
    if (
      deleteCandidateId &&
      !tags.some((tag) => tag.id === deleteCandidateId)
    ) {
      setDeleteCandidateId("");
    }
  }, [deleteCandidateId, editingTagId, tags]);

  const handleCreate = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = newTagName.trim();
    if (!name || isCreating) return;

    setIsCreating(true);
    setFormError("");
    const created = await onCreateTag(name, newTagColor);
    if (created) {
      setNewTagName("");
      setNewTagColor("blue");
    } else {
      setFormError("创建失败，请确认名称未被占用后重试");
    }
    setIsCreating(false);
  };

  const startEditing = (tag: Tag) => {
    setEditingTagId(tag.id);
    setEditName(tag.label);
    setEditColor(tag.color);
    setDeleteCandidateId("");
    setFormError("");
  };

  const saveEditing = async (tag: Tag) => {
    const name = editName.trim();
    if (!name || pendingTagId) return;

    setPendingTagId(tag.id);
    setFormError("");
    const updated = await onUpdateTag(tag.id, name, editColor);
    if (updated) {
      setEditingTagId("");
    } else {
      setFormError("保存失败，请检查名称或稍后重试");
    }
    setPendingTagId("");
  };

  const confirmDelete = async (tag: Tag) => {
    if (pendingTagId) return;

    setPendingTagId(tag.id);
    setFormError("");
    const deleted = await onDeleteTag(tag.id);
    if (deleted) {
      setDeleteCandidateId("");
    } else {
      setFormError("删除失败，请稍后重试");
    }
    setPendingTagId("");
  };

  return (
    <section className="tag-management-workspace" aria-label="自定义标签管理">
      <header className="tag-management-heading">
        <div>
          <button className="tag-management-back" type="button" onClick={onBack}>
            <ArrowLeft weight="bold" />
            返回素材库
          </button>
          <span className="cinematic-eyebrow">TACTICAL ARCHIVE / TAG CONTROL</span>
          <h1>自定义标签</h1>
          <p>创建和维护你自己的标签，并查看每个标签关联的素材数量。</p>
        </div>
        <div className="tag-management-heading-status" aria-live="polite">
          <strong>{tags.length} 个自定义标签</strong>
          <span>全部由用户创建</span>
          <small>{activityMessage}</small>
        </div>
      </header>

      <div className="tag-management-overview">
        <form className="tag-management-create" onSubmit={handleCreate}>
          <div className="tag-management-section-title">
            <span><Plus weight="bold" />创建标签</span>
            <small>名称最多 24 个字符</small>
          </div>
          <label>
            <span>标签名称</span>
            <input
              autoComplete="off"
              maxLength={24}
              placeholder="例如：精彩残局"
              value={newTagName}
              onChange={(event) => setNewTagName(event.currentTarget.value)}
            />
          </label>
          <ColorPicker
            label="标签颜色"
            value={newTagColor}
            onChange={setNewTagColor}
          />
          <button
            className="tag-management-primary-button"
            disabled={isCreating || !newTagName.trim()}
            type="submit"
          >
            <Plus weight="bold" />
            {isCreating ? "创建中…" : "创建标签"}
          </button>
        </form>

        <div className="tag-management-metrics" aria-label="标签使用概览">
          <div><strong>{tags.length}</strong><span>自定义标签</span></div>
          <div><strong>{usedTagCount}</strong><span>使用中的标签</span></div>
          <div><strong>{taggedClipCount}</strong><span>已标记素材</span></div>
          <div><strong>{Math.max(0, totalClipCount - taggedClipCount)}</strong><span>未标记素材</span></div>
          <p><TagIcon weight="fill" />视频类型由扫描结果单独维护，不会混入这里的自定义标签。</p>
        </div>
      </div>

      <section className="tag-management-catalog" aria-label="标签目录">
        <header>
          <div className="tag-management-section-title">
            <span><TagIcon weight="duotone" />标签目录</span>
            <small>{visibleTags.length} / {tags.length}</small>
          </div>
          <label className="tag-management-search">
            <MagnifyingGlass weight="bold" />
            <input
              aria-label="搜索标签"
              placeholder="搜索标签名称"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
            {query ? (
              <button aria-label="清除搜索" type="button" onClick={() => setQuery("")}>
                <X weight="bold" />
              </button>
            ) : null}
          </label>
        </header>

        {formError ? <p className="tag-management-error" role="alert">{formError}</p> : null}

        <div className="tag-management-list">
          {visibleTags.map((tag) => {
            const usageCount = tagUsageCounts.get(tag.id) ?? 0;
            const isEditing = editingTagId === tag.id;
            const isPending = pendingTagId === tag.id;
            const isConfirmingDelete = deleteCandidateId === tag.id;

            return (
              <article className="tag-management-row" key={tag.id}>
                <div className="tag-management-identity">
                  <span className={`tag tag--${tag.color}`}>{tag.label}</span>
                  <span className="tag-kind">自定义</span>
                </div>
                <div className="tag-management-usage">
                  <strong>{usageCount.toLocaleString("zh-CN")}</strong>
                  <span>条素材</span>
                </div>
                <div className="tag-management-row-actions">
                  <button
                    disabled={usageCount === 0 || isPending}
                    title={usageCount === 0 ? "该标签暂未关联素材" : "在素材库中查看"}
                    type="button"
                    onClick={() => onViewTag(tag.id)}
                  >
                    <FunnelSimple weight="bold" />查看素材
                  </button>
                  <button disabled={isPending} type="button" onClick={() => startEditing(tag)}>
                    <PencilSimple weight="bold" />编辑
                  </button>
                  {isConfirmingDelete ? (
                    <>
                      <button
                        className="tag-management-danger-button"
                        disabled={isPending}
                        type="button"
                        onClick={() => void confirmDelete(tag)}
                      >
                        <Trash weight="fill" />{isPending ? "删除中…" : "确认删除"}
                      </button>
                      <button type="button" onClick={() => setDeleteCandidateId("")}>取消</button>
                    </>
                  ) : (
                    <button
                      disabled={isPending}
                      type="button"
                      onClick={() => {
                        setDeleteCandidateId(tag.id);
                        setEditingTagId("");
                      }}
                    >
                      <Trash weight="bold" />删除
                    </button>
                  )}
                </div>

                {isEditing ? (
                  <form
                    className="tag-management-edit"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void saveEditing(tag);
                    }}
                  >
                    <label>
                      <span>名称</span>
                      <input
                        autoFocus
                        maxLength={24}
                        value={editName}
                        onChange={(event) => setEditName(event.currentTarget.value)}
                      />
                    </label>
                    <ColorPicker
                      compact
                      label="颜色"
                      value={editColor}
                      onChange={setEditColor}
                    />
                    <div>
                      <button
                        disabled={isPending || !editName.trim() || (editName.trim() === tag.label && editColor === tag.color)}
                        type="submit"
                      >
                        <FloppyDisk weight="bold" />{isPending ? "保存中…" : "保存"}
                      </button>
                      <button type="button" onClick={() => setEditingTagId("")}>
                        <X weight="bold" />取消
                      </button>
                    </div>
                  </form>
                ) : null}
              </article>
            );
          })}

          {visibleTags.length === 0 ? (
            <div className="tag-management-empty">
              <TagIcon weight="duotone" />
              <strong>{tags.length === 0 ? "还没有标签" : "没有匹配的标签"}</strong>
              <span>{tags.length === 0 ? "在上方创建第一个标签" : "尝试更换搜索关键词"}</span>
            </div>
          ) : null}
        </div>
      </section>
    </section>
  );
}

type ColorPickerProps = {
  compact?: boolean;
  label: string;
  value: TagColor;
  onChange: (color: TagColor) => void;
};

function ColorPicker({ compact = false, label, value, onChange }: ColorPickerProps) {
  return (
    <fieldset className={compact ? "tag-color-picker tag-color-picker--compact" : "tag-color-picker"}>
      <legend>{label}</legend>
      <div>
        {TAG_COLORS.map((color) => (
          <button
            aria-label={COLOR_LABELS[color]}
            aria-pressed={value === color}
            className={`tag-color-choice tag-color-choice--${color}`}
            key={color}
            title={COLOR_LABELS[color]}
            type="button"
            onClick={() => onChange(color)}
          />
        ))}
      </div>
    </fieldset>
  );
}
