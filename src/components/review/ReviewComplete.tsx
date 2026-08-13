import {
  ArrowCounterClockwise,
  CheckCircle,
  Heart,
  Tag,
  ArrowSquareOut,
  Hourglass,
} from "@phosphor-icons/react";
import { useState } from "react";
import { useReviewShortcuts } from "../../hooks/useReviewShortcuts";
import type { ClipSummary, ReviewSession } from "../../types";
import type { ReviewSessionCounts } from "../../lib/reviewSessions";

type ReviewCompleteProps = {
  session: ReviewSession;
  candidates: readonly ClipSummary[];
  counts: ReviewSessionCounts;
  canUndo: boolean;
  isUndoing: boolean;
  onUndo: () => void;
  onContinuePending: () => void;
  onViewSelected: (autoOpenTagDialog: boolean) => void;
  onFavoriteSelected: (clipIds: string[]) => Promise<boolean>;
  onOpenLibrary: () => void;
};

export function ReviewComplete({
  session,
  candidates,
  counts,
  canUndo,
  isUndoing,
  onUndo,
  onContinuePending,
  onViewSelected,
  onFavoriteSelected,
  onOpenLibrary,
}: ReviewCompleteProps) {
  const [isFavoriting, setIsFavoriting] = useState(false);
  const [favoriteFeedback, setFavoriteFeedback] = useState("");
  const selectedIds = session.items
    .filter((item) => item.decision === "selected")
    .map((item) => item.videoId);
  const visibleSelectedCount = candidates.filter((clip) => selectedIds.includes(clip.id)).length;
  const endedEarly = counts.remaining > 0;

  const handleFavorite = async () => {
    if (isFavoriting || selectedIds.length === 0) return;
    setIsFavoriting(true);
    setFavoriteFeedback("");
    try {
      const succeeded = await onFavoriteSelected(selectedIds);
      setFavoriteFeedback(succeeded ? "已加入收藏；挑片结果保持不变。" : "收藏更新失败，请在素材库中重试。");
    } finally {
      setIsFavoriting(false);
    }
  };

  // Keep the last decision undoable even after the final card transitions to
  // the results view. The shared shortcut hook still guards focused controls.
  useReviewShortcuts({
    active: canUndo && !isUndoing,
    isBusy: isUndoing,
    canUndo,
    onUndo,
  });

  return (
    <section aria-labelledby="review-complete-title" className="review-workspace review-complete">
      <div className="review-complete-mark"><CheckCircle weight="duotone" /></div>
      <h1 id="review-complete-title">{endedEarly ? "本轮挑片已提前结束" : "本轮挑片完成"}</h1>
      <p>
        {endedEarly
          ? `已浏览 ${counts.reviewed} / ${counts.total} 条素材，剩余 ${counts.remaining} 条未处理。已做出的挑片决定会保留。`
          : `${counts.total} 条素材已浏览。挑片结果只属于这一轮会话，不会自动修改收藏、标签或回收站。`}
      </p>

      <dl className="review-complete-stats" aria-label="本轮挑片统计">
        <div><dt>入选</dt><dd>{counts.selected}</dd></div>
        <div><dt>待定</dt><dd>{counts.pending}</dd></div>
        <div><dt>跳过</dt><dd>{counts.skipped}</dd></div>
      </dl>

      <div className="review-complete-actions">
        <button
          className="review-complete-primary"
          disabled={visibleSelectedCount === 0}
          type="button"
          onClick={() => onViewSelected(false)}
        >
          <ArrowSquareOut weight="bold" />查看入选素材 ({counts.selected})
        </button>
        <button disabled={counts.pending === 0} type="button" onClick={onContinuePending}>
          <Hourglass weight="bold" />继续处理待定 ({counts.pending})
        </button>
        <button disabled={visibleSelectedCount === 0} type="button" onClick={() => onViewSelected(true)}>
          <Tag weight="bold" />批量添加标签
        </button>
        <button disabled={selectedIds.length === 0 || isFavoriting} type="button" onClick={() => void handleFavorite()}>
          <Heart weight="bold" />{isFavoriting ? "正在收藏…" : "收藏入选素材"}
        </button>
        <button aria-keyshortcuts="Z" disabled={!canUndo || isUndoing} type="button" onClick={onUndo}>
          <ArrowCounterClockwise weight="bold" />撤销上一步
        </button>
        <button type="button" onClick={onOpenLibrary}>打开素材库</button>
      </div>
      {favoriteFeedback ? <p aria-live="polite" className="review-complete-feedback">{favoriteFeedback}</p> : null}
    </section>
  );
}
