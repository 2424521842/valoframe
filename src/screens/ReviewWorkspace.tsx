import { useEffect, useMemo, useState } from "react";
import { ReviewComplete } from "../components/review/ReviewComplete";
import { ReviewSession as ReviewSessionPlayer } from "../components/review/ReviewSession";
import { ReviewSetup, type ReviewScopeEditor } from "../components/review/ReviewSetup";
import { useReviewSessionController } from "../hooks/useReviewSessionController";
import type { ClipSummary, ReviewCandidateScope, ReviewSession, ReviewSessionFilters, ReviewSessionSort } from "../types";

type ReviewWorkspaceProps = {
  inheritedFilters: ReviewSessionFilters;
  scopeEditor: ReviewScopeEditor;
  isScopeUpdating?: boolean;
  autoplay?: boolean;
  initialVolumePercent?: number;
  initialMuted?: boolean;
  onBack: () => void;
  onAudioPreferenceChange?: (preference: { volumePercent: number; muted: boolean }) => void;
  onOpenOriginal: (clipId: string) => void;
  onRemoveFromIndex: (clipId: string) => Promise<boolean>;
  onViewSelected: (
    session: ReviewSession,
    candidates: readonly ClipSummary[],
    autoOpenTagDialog: boolean,
  ) => void;
  onFavoriteSelected: (clipIds: string[]) => Promise<boolean>;
};

export function ReviewWorkspace({
  inheritedFilters,
  scopeEditor,
  isScopeUpdating = false,
  autoplay = true,
  initialVolumePercent = 100,
  initialMuted = false,
  onBack,
  onAudioPreferenceChange,
  onOpenOriginal,
  onRemoveFromIndex,
  onViewSelected,
  onFavoriteSelected,
}: ReviewWorkspaceProps) {
  const controller = useReviewSessionController();
  const [sort, setSort] = useState<ReviewSessionSort>(() => defaultReviewSort(inheritedFilters.query.sortBy));
  const [candidateScope, setCandidateScope] = useState<ReviewCandidateScope>("all");
  const sessionFilters = useMemo<ReviewSessionFilters>(() => ({
    ...inheritedFilters,
    labels: [...inheritedFilters.labels],
    query: { ...inheritedFilters.query },
    sort,
    candidateScope,
  }), [candidateScope, inheritedFilters, sort]);

  useEffect(() => {
    void controller.prepare(sessionFilters);
  }, [controller.prepare, sessionFilters]);

  if (controller.phase === "setup" || controller.phase === "loading") {
    return (
      <ReviewSetup
        candidateCount={controller.draftCandidates.length}
        candidateScope={candidateScope}
        error={controller.error}
        filterLabels={inheritedFilters.labels}
        isPreparing={controller.isPreparing || isScopeUpdating}
        resumableSession={controller.resumableSession}
        scopeEditor={scopeEditor}
        sort={sort}
        onCandidateScopeChange={setCandidateScope}
        onResume={() => {
          if (controller.resumableSession) void controller.resume(controller.resumableSession);
        }}
        onSortChange={setSort}
        onStart={() => {
          if (controller.resumableSession) {
            void controller.resume(controller.resumableSession);
          } else {
            void controller.startNew(sessionFilters);
          }
        }}
      />
    );
  }

  if (controller.phase === "completed" && controller.session) {
    return (
      <ReviewComplete
        candidates={controller.candidateClips}
        canUndo={controller.canUndo}
        counts={controller.counts}
        isUndoing={controller.isUndoing}
        session={controller.session}
        onContinuePending={() => controller.continuePending()}
        onFavoriteSelected={onFavoriteSelected}
        onOpenLibrary={onBack}
        onUndo={() => controller.undo()}
        onViewSelected={(autoOpenTagDialog) => onViewSelected(
          controller.session!,
          controller.candidateClips,
          autoOpenTagDialog,
        )}
      />
    );
  }

  return (
    <ReviewSessionPlayer
      autoplay={autoplay}
      controller={controller}
      initialMuted={initialMuted}
      initialVolumePercent={initialVolumePercent}
      onAudioPreferenceChange={onAudioPreferenceChange}
      onExit={onBack}
      onOpenOriginal={onOpenOriginal}
      onRemoveFromIndex={onRemoveFromIndex}
    />
  );
}

function defaultReviewSort(sortBy: ReviewSessionFilters["query"]["sortBy"]): ReviewSessionSort {
  if (sortBy === "modified-desc") return "latest";
  if (sortBy === "modified-asc") return "oldest";
  return "library";
}
