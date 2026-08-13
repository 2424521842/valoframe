import {
  ArrowCounterClockwise,
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  CornersIn,
  CornersOut,
  Crosshair,
  Database,
  WarningCircle,
  FolderOpen,
  GameController,
  MapTrifold,
  Pause,
  Play,
  SpeakerHigh,
  SpeakerSlash,
  UserCircle,
} from "@phosphor-icons/react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { commandErrorMessage, getClipMedia } from "../../api/backend";
import { UiAlertDialog, UiAlertDialogAction, UiAlertDialogCancel, UiAlertDialogContent, UiAlertDialogDescription, UiAlertDialogTitle } from "../ui/alert-dialog";
import { useElementFullscreen } from "../../hooks/useElementFullscreen";
import { useReviewShortcuts } from "../../hooks/useReviewShortcuts";
import type { ReviewSessionController } from "../../hooks/useReviewSessionController";
import { formatBytes, formatDateTime } from "../../lib/formatters";
import { valorantAgentDisplayIconUrl, valorantMapListViewIconUrl } from "../../lib/valorantAssets";
import type { ClipMedia, ReviewItemDecision } from "../../types";

const REVIEW_EXIT_DISTANCE = 720;
const REVIEW_EXIT_MS = 190;
const SWIPE_DECISION_THRESHOLD = 86;

type ReviewSessionProps = {
  autoplay: boolean;
  initialVolumePercent?: number;
  initialMuted?: boolean;
  controller: ReviewSessionController;
  onAudioPreferenceChange?: (preference: { volumePercent: number; muted: boolean }) => void;
  onExit: () => void;
  onOpenOriginal: (clipId: string) => void;
  onRemoveFromIndex: (clipId: string) => Promise<boolean>;
};

export function ReviewSession({
  autoplay,
  initialVolumePercent = 100,
  initialMuted = false,
  controller,
  onAudioPreferenceChange,
  onExit,
  onOpenOriginal,
  onRemoveFromIndex,
}: ReviewSessionProps) {
  const clip = controller.currentClip;
  const [media, setMedia] = useState<ClipMedia | null>(null);
  const [mediaError, setMediaError] = useState("");
  const [isMediaLoading, setIsMediaLoading] = useState(false);
  const [autoplayBlocked, setAutoplayBlocked] = useState(false);
  const initialVolume = normalizeVolumePercent(initialVolumePercent) / 100;
  const [volume, setVolume] = useState(initialVolume);
  const [isMuted, setIsMuted] = useState(initialMuted || initialVolume === 0);
  const [drag, setDrag] = useState({ x: 0, y: 0 });
  const [exitDecision, setExitDecision] = useState<Exclude<ReviewItemDecision, "unreviewed"> | null>(null);
  const [exitDialogOpen, setExitDialogOpen] = useState(false);
  const [removeDialogOpen, setRemoveDialogOpen] = useState(false);
  const [isRemoving, setIsRemoving] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const mediaTokenRef = useRef(0);
  const pointerStartRef = useRef<{ pointerId: number; x: number; y: number } | null>(null);
  const transitionBusyRef = useRef(false);
  const dragRef = useRef(drag);
  const isApplyingAudioPreferenceRef = useRef(false);
  const lastReportedAudioPreferenceRef = useRef({
    volumePercent: Math.round(initialVolume * 100),
    muted: initialMuted || initialVolume === 0,
  });
  dragRef.current = drag;

  const activeMedia = media?.clipId === clip?.id ? media : null;
  const isBusy = controller.isDeciding || controller.isUndoing || transitionBusyRef.current || isRemoving;
  const mediaUnavailable = Boolean(mediaError || activeMedia?.message);
  const fullscreenEnabled = Boolean(
    activeMedia?.playable
    && activeMedia.mediaUrl
    && !mediaUnavailable
    && !isBusy,
  );
  const {
    clearFullscreenError,
    elementRef: mediaShellRef,
    exitFullscreen,
    fullscreenError,
    isFullscreen,
    shouldIgnoreEscape: shouldIgnoreFullscreenEscape,
    toggleFullscreen,
  } = useElementFullscreen<HTMLDivElement>({ enabled: fullscreenEnabled });
  const progress = controller.counts.total === 0
    ? 0
    : Math.min(100, controller.counts.reviewed / controller.counts.total * 100);

  useEffect(() => {
    const token = mediaTokenRef.current + 1;
    mediaTokenRef.current = token;
    setMedia(null);
    setMediaError("");
    setAutoplayBlocked(false);
    clearFullscreenError();
    setIsMediaLoading(Boolean(clip));
    if (!clip) return undefined;
    void getClipMedia(clip.id)
      .then((nextMedia) => {
        if (mediaTokenRef.current === token) setMedia(nextMedia);
      })
      .catch((requestError) => {
        if (mediaTokenRef.current === token) {
          setMediaError(commandErrorMessage(requestError));
        }
      })
      .finally(() => {
        if (mediaTokenRef.current === token) setIsMediaLoading(false);
      });
    return () => {
      if (mediaTokenRef.current === token) mediaTokenRef.current += 1;
    };
  }, [clip?.id]);

  const nextPoster = useMemo(() => {
    if (!clip) return null;
    const index = controller.candidateClips.findIndex((candidate) => candidate.id === clip.id);
    return index >= 0 ? controller.candidateClips[index + 1]?.thumbnailUrl ?? null : null;
  }, [clip, controller.candidateClips]);

  useEffect(() => {
    if (!nextPoster) return undefined;
    const image = new Image();
    image.decoding = "async";
    image.src = nextPoster;
    return () => {
      image.src = "";
    };
  }, [nextPoster]);

  const syncAudioPreference = useCallback((video: HTMLVideoElement) => {
    const nextVolume = Number.isFinite(video.volume)
      ? Math.max(0, Math.min(1, video.volume))
      : 0;
    const nextPreference = {
      volumePercent: Math.round(nextVolume * 100),
      muted: video.muted || nextVolume === 0,
    };
    setVolume(nextVolume);
    setIsMuted(nextPreference.muted);
    const previousPreference = lastReportedAudioPreferenceRef.current;
    if (
      previousPreference.volumePercent === nextPreference.volumePercent
      && previousPreference.muted === nextPreference.muted
    ) {
      return;
    }
    lastReportedAudioPreferenceRef.current = nextPreference;
    onAudioPreferenceChange?.(nextPreference);
  }, [onAudioPreferenceChange]);

  const applyAudioPreference = useCallback((video: HTMLVideoElement) => {
    isApplyingAudioPreferenceRef.current = true;
    try {
      video.volume = volume;
      video.muted = isMuted || volume === 0;
    } finally {
      isApplyingAudioPreferenceRef.current = false;
    }
  }, [isMuted, volume]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !activeMedia?.playable || !activeMedia.mediaUrl) return undefined;
    applyAudioPreference(video);
    if (!autoplay) return () => video.pause();
    const playResult = video.play();
    void playResult?.then(
      () => setAutoplayBlocked(false),
      () => setAutoplayBlocked(true),
    );
    return () => video.pause();
  }, [activeMedia?.mediaUrl, activeMedia?.playable, applyAudioPreference, autoplay, clip?.id]);

  const togglePlayback = useCallback(() => {
    const video = videoRef.current;
    if (!video || !activeMedia?.playable || !activeMedia.mediaUrl) return;
    try {
      if (video.paused) {
        const playResult = video.play();
        void playResult?.then(
          () => setAutoplayBlocked(false),
          () => setAutoplayBlocked(true),
        );
      } else {
        video.pause();
      }
    } catch {
      setAutoplayBlocked(true);
    }
  }, [activeMedia?.mediaUrl, activeMedia?.playable]);

  const toggleAudioMute = useCallback(() => {
    const video = videoRef.current;
    if (!video || !activeMedia?.playable || !activeMedia.mediaUrl) return;
    isApplyingAudioPreferenceRef.current = true;
    try {
      const shouldMute = !(video.muted || video.volume === 0);
      if (!shouldMute && video.volume === 0) {
        video.volume = volume > 0 ? volume : 1;
      }
      video.muted = shouldMute;
    } finally {
      isApplyingAudioPreferenceRef.current = false;
    }
    syncAudioPreference(video);
  }, [activeMedia?.mediaUrl, activeMedia?.playable, syncAudioPreference, volume]);

  const runDecision = useCallback(async (decision: Exclude<ReviewItemDecision, "unreviewed">) => {
    if (transitionBusyRef.current || isRemoving) return;
    transitionBusyRef.current = true;
    releaseVideo(videoRef.current);
    setExitDecision(decision);
    await controller.decide(decision, waitForReviewExit);
    transitionBusyRef.current = false;
    setExitDecision(null);
    setDrag({ x: 0, y: 0 });
  }, [controller, isRemoving]);

  const runUndo = useCallback(() => {
    if (transitionBusyRef.current || isRemoving || !controller.canUndo) return;
    releaseVideo(videoRef.current);
    controller.undo();
    setExitDecision(null);
    setDrag({ x: 0, y: 0 });
  }, [controller, isRemoving]);

  const saveAndExit = useCallback(() => {
    controller.saveProgress();
    onExit();
  }, [controller, onExit]);

  const finishEarly = useCallback(() => {
    if (isBusy || controller.counts.remaining === 0) return;
    releaseVideo(videoRef.current);
    controller.finishEarly();
  }, [controller, isBusy]);

  const requestExit = useCallback(() => {
    if (isBusy) return;
    if (controller.counts.reviewed === 0) {
      saveAndExit();
      return;
    }
    setExitDialogOpen(true);
  }, [controller.counts.reviewed, isBusy, saveAndExit]);

  useReviewShortcuts({
    active: Boolean(clip) && !exitDialogOpen && !removeDialogOpen,
    isBusy,
    canUndo: controller.canUndo,
    onDecision: (decision) => void runDecision(decision),
    onUndo: runUndo,
    onTogglePlayback: togglePlayback,
    onToggleFullscreen: () => void toggleFullscreen(),
    onRequestExit: requestExit,
    shouldIgnoreFullscreenEscape,
  });

  const handlePointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (isFullscreen || isBusy || (event.target as Element).closest("button, input, a")) return;
    pointerStartRef.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    if (isFullscreen) return;
    const start = pointerStartRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    setDrag({
      x: Math.max(-180, Math.min(180, event.clientX - start.x)),
      y: Math.max(0, Math.min(120, event.clientY - start.y)),
    });
  };

  const finishPointer = (event: ReactPointerEvent<HTMLElement>) => {
    const start = pointerStartRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    pointerStartRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (isFullscreen) {
      setDrag({ x: 0, y: 0 });
      return;
    }
    const { x, y } = dragRef.current;
    if (y >= SWIPE_DECISION_THRESHOLD && y > Math.abs(x) * 0.75) {
      void runDecision("pending");
    } else if (x <= -SWIPE_DECISION_THRESHOLD) {
      void runDecision("skipped");
    } else if (x >= SWIPE_DECISION_THRESHOLD) {
      void runDecision("selected");
    } else {
      setDrag({ x: 0, y: 0 });
    }
  };

  const confirmIndexRemoval = async () => {
    if (!clip || isRemoving) return;
    setIsRemoving(true);
    try {
      const removed = await onRemoveFromIndex(clip.id);
      if (removed) {
        controller.removeCurrent();
        setRemoveDialogOpen(false);
      } else {
        setMediaError("无法从素材索引移除此文件，请稍后重试");
      }
    } catch (requestError) {
      setMediaError(`无法从素材索引移除：${commandErrorMessage(requestError)}`);
    } finally {
      setIsRemoving(false);
    }
  };

  if (!clip) {
    return (
      <section aria-live="polite" className="review-workspace review-session review-session--loading">
        <h1>{controller.phase === "completed" ? "正在整理本轮结果…" : "正在准备下一条素材…"}</h1>
      </section>
    );
  }

  const agentArtworkUrl = clip.agentName ? valorantAgentDisplayIconUrl(clip.agentName) : "";
  const mapArtworkUrl = clip.mapName ? valorantMapListViewIconUrl(clip.mapName) : "";
  const transform = exitDecision
    ? reviewExitTransform(exitDecision)
    : `translate3d(${drag.x}px, ${drag.y}px, 0) rotate(${drag.x / 52}deg)`;

  return (
    <section aria-label="快速挑片会话" className="review-workspace review-session">
      <header className="review-session-header">
        <button className="cinematic-button cinematic-button--secondary cinematic-button--small" type="button" onClick={requestExit}>
          <ArrowLeft weight="bold" />退出挑片
        </button>
        <div className="review-session-progress">
          <span>{controller.counts.reviewed} / {controller.counts.total}</span>
          <i aria-label={`挑片进度 ${controller.counts.reviewed} / ${controller.counts.total}`}><b style={{ transform: `scaleX(${progress / 100})` }} /></i>
          <small>已入选 {controller.counts.selected} · 待定 {controller.counts.pending}</small>
        </div>
        <button
          aria-keyshortcuts="Z"
          className="cinematic-button cinematic-button--secondary cinematic-button--small"
          disabled={!controller.canUndo || isBusy}
          title="撤销上一步（Z）"
          type="button"
          onClick={runUndo}
        >
          <ArrowCounterClockwise weight="bold" />撤销 <kbd>Z</kbd>
        </button>
      </header>

      <article
        aria-busy={isBusy}
        aria-label="当前挑片素材"
        className={`review-card${exitDecision ? ` review-card--exit review-card--${exitDecision}` : ""}`}
        style={{ transform }}
        onPointerCancel={finishPointer}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={finishPointer}
      >
        <div className="review-card-verdict review-card-verdict--skipped">跳过</div>
        <div className="review-card-verdict review-card-verdict--pending">待定</div>
        <div className="review-card-verdict review-card-verdict--selected">入选</div>
        <div
          className={isFullscreen
            ? "review-card-media review-card-media--fullscreen"
            : "review-card-media"}
          ref={mediaShellRef}
          onDoubleClick={(event) => {
            if (event.target instanceof Element && event.target.closest("button")) return;
            void toggleFullscreen();
          }}
        >
          {activeMedia?.playable && activeMedia.mediaUrl ? (
            <video
              key={clip.id}
              playsInline
              poster={clip.thumbnailUrl ?? undefined}
              preload="metadata"
              ref={videoRef}
              src={activeMedia.mediaUrl}
              onError={() => {
                setMediaError("当前系统无法内嵌播放此视频");
                void exitFullscreen();
              }}
              onVolumeChange={(event) => {
                if (!isApplyingAudioPreferenceRef.current) {
                  syncAudioPreference(event.currentTarget);
                }
              }}
            />
          ) : clip.thumbnailUrl ? (
            <img alt="" src={clip.thumbnailUrl} />
          ) : (
            <div className={`review-card-placeholder clip-thumb--${clip.thumbnailTone}`}><Play weight="fill" /></div>
          )}
          {isMediaLoading ? <span className="review-media-state">正在准备预览…</span> : null}
          {fullscreenError ? (
            <span className="review-media-state review-media-state--fullscreen" role="status">
              {fullscreenError}
            </span>
          ) : null}
          {activeMedia?.playable && activeMedia.mediaUrl && autoplayBlocked ? (
            <button aria-keyshortcuts="Space" className="review-media-play" type="button" onClick={togglePlayback}>
              <Play weight="fill" />点击播放
            </button>
          ) : null}
          {mediaUnavailable ? (
            <div className="review-media-error" role="status">
              <WarningCircle weight="duotone" />
              <div><strong>无法加载此素材</strong><span>{mediaError || activeMedia?.message}</span></div>
              <div className="review-media-error-actions">
                <button type="button" onClick={() => void runDecision("skipped")}>跳过</button>
                <button type="button" onClick={() => onOpenOriginal(clip.id)}><FolderOpen weight="bold" />定位</button>
                <button type="button" onClick={() => setRemoveDialogOpen(true)}><Database weight="bold" />移除索引</button>
              </div>
            </div>
          ) : null}
          {activeMedia?.playable && activeMedia.mediaUrl && !mediaUnavailable ? (
            <>
              <button
                aria-label={isMuted || volume === 0 ? "恢复声音" : "静音"}
                aria-pressed={isMuted || volume === 0}
                className="review-media-audio"
                type="button"
                onClick={toggleAudioMute}
              >
                {isMuted || volume === 0 ? <SpeakerSlash weight="fill" /> : <SpeakerHigh weight="fill" />}
              </button>
              <button aria-label="播放或暂停预览" aria-keyshortcuts="Space" className="review-media-toggle" type="button" onClick={togglePlayback}>
                <Play weight="fill" /><Pause weight="fill" />
              </button>
              <button
                aria-keyshortcuts="F"
                aria-label={isFullscreen ? "退出全屏" : "进入全屏"}
                aria-pressed={isFullscreen}
                className="review-media-fullscreen"
                disabled={!isFullscreen && !fullscreenEnabled}
                title={isFullscreen ? "退出全屏（F 或 Esc）" : "进入全屏（F 或双击视频）"}
                type="button"
                onClick={() => void toggleFullscreen()}
              >
                {isFullscreen ? <CornersIn weight="bold" /> : <CornersOut weight="bold" />}
              </button>
            </>
          ) : null}
        </div>

        <aside aria-label="当前素材详细信息" className="review-card-copy">
          <header className="review-card-identity">
            {mapArtworkUrl ? <img alt="" className="review-card-map-art" draggable={false} src={mapArtworkUrl} /> : null}
            <div className="review-card-agent-art" aria-hidden="true">
              {agentArtworkUrl ? <img alt="" draggable={false} src={agentArtworkUrl} /> : <Crosshair weight="duotone" />}
            </div>
            <div className="review-card-identity-copy">
              <h1>{clip.agentName || "英雄未识别"}</h1>
              <span className="review-card-account"><UserCircle weight="duotone" />{clip.accountDisplayName || "账号未识别"}</span>
              <div className="review-card-context">
                <span><MapTrifold weight="duotone" />{clip.mapName || "地图未识别"}</span>
                <span><GameController weight="duotone" />{clip.gameMode || "模式未识别"}</span>
              </div>
            </div>
          </header>
          <div className="review-card-details">
            <dl className="review-performance" aria-label="对局表现">
              <div className="review-performance-kda"><dt>K / D / A</dt><dd>{clip.kda || "—"}</dd></div>
              <div><dt>击杀数</dt><dd>{clip.killCount ?? "—"}</dd></div>
              <div><dt>战斗评分</dt><dd>{clip.combatScore ?? "—"}</dd></div>
            </dl>
            <dl className="review-card-facts" aria-label="素材信息">
              <div><dt>录制时间</dt><dd>{formatDateTime(clip.createdAt)}</dd></div>
              <div><dt>视频时长</dt><dd>{formatDuration(clip.durationMs)}</dd></div>
              <div><dt>文件大小</dt><dd>{formatBytes(clip.sizeBytes)}</dd></div>
            </dl>
          </div>
        </aside>
      </article>

      {controller.error ? (
        <div className="review-session-error" role="alert">
          <span>{controller.error}</span>
          <button aria-label="关闭错误" type="button" onClick={controller.clearError}>关闭</button>
        </div>
      ) : null}

      <footer className="review-actions" aria-label="挑片操作">
        <ReviewAction decision="skipped" disabled={isBusy} icon={<ArrowLeft weight="bold" />} keyHint="A / ←" label="跳过" onClick={() => void runDecision("skipped")} />
        <ReviewAction decision="pending" disabled={isBusy} icon={<ArrowDown weight="bold" />} keyHint="S / ↓" label="待定" onClick={() => void runDecision("pending")} />
        <ReviewAction decision="selected" disabled={isBusy} icon={<ArrowRight weight="bold" />} keyHint="D / →" label="入选" onClick={() => void runDecision("selected")} />
      </footer>

      <UiAlertDialog open={exitDialogOpen} onOpenChange={setExitDialogOpen}>
        <UiAlertDialogContent>
          <UiAlertDialogTitle>退出或结束本轮挑片？</UiAlertDialogTitle>
          <UiAlertDialogDescription>
            当前进度：{controller.counts.reviewed} / {controller.counts.total}。保存进度后可稍后继续；提前结束会保留已做决定，剩余 {controller.counts.remaining} 条素材保持未处理，并不再作为可继续进度。
          </UiAlertDialogDescription>
          <div className="ui-alert-dialog-actions review-exit-actions">
            <UiAlertDialogCancel>继续挑片</UiAlertDialogCancel>
            <UiAlertDialogAction className="review-exit-save" onClick={saveAndExit}>保存进度并退出</UiAlertDialogAction>
            {controller.counts.remaining > 0 ? (
              <UiAlertDialogAction className="review-exit-finish" onClick={finishEarly}>
                提前结束并查看结果
              </UiAlertDialogAction>
            ) : null}
          </div>
        </UiAlertDialogContent>
      </UiAlertDialog>

      <UiAlertDialog open={removeDialogOpen} onOpenChange={setRemoveDialogOpen}>
        <UiAlertDialogContent>
          <UiAlertDialogTitle>从索引移除此素材？</UiAlertDialogTitle>
          <UiAlertDialogDescription>这不会删除本地视频，但会让它不再出现在素材库和本轮挑片中。</UiAlertDialogDescription>
          <div className="ui-alert-dialog-actions">
            <UiAlertDialogCancel disabled={isRemoving}>取消</UiAlertDialogCancel>
            <UiAlertDialogAction disabled={isRemoving} onClick={() => void confirmIndexRemoval()}>
              {isRemoving ? "正在移除…" : "移除索引"}
            </UiAlertDialogAction>
          </div>
        </UiAlertDialogContent>
      </UiAlertDialog>
    </section>
  );
}

function ReviewAction({
  decision,
  disabled,
  icon,
  keyHint,
  label,
  onClick,
}: {
  decision: Exclude<ReviewItemDecision, "unreviewed">;
  disabled: boolean;
  icon: ReactNode;
  keyHint: string;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-keyshortcuts={keyHint}
      className={`review-action review-action--${decision}`}
      disabled={disabled}
      type="button"
      onClick={onClick}
    >
      <span className="review-action-icon">{icon}</span>
      <strong>{label}</strong>
      <small>{keyHint}</small>
    </button>
  );
}

function reviewExitTransform(decision: Exclude<ReviewItemDecision, "unreviewed">): string {
  if (decision === "skipped") return `translate3d(-${REVIEW_EXIT_DISTANCE}px, 0, 0) rotate(-7deg)`;
  if (decision === "selected") return `translate3d(${REVIEW_EXIT_DISTANCE}px, 0, 0) rotate(7deg)`;
  return "translate3d(0, 128px, 0) scale(0.98)";
}

function formatDuration(durationMs: number | null): string {
  if (durationMs === null || durationMs <= 0) return "未知";
  const seconds = Math.round(durationMs / 1_000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function normalizeVolumePercent(volumePercent: number): number {
  if (!Number.isFinite(volumePercent)) return 100;
  return Math.round(Math.max(0, Math.min(100, volumePercent)));
}

function releaseVideo(video: HTMLVideoElement | null): void {
  if (!video) return;
  try {
    video.pause();
    video.removeAttribute("src");
    video.load();
  } catch {
    // Detached media elements can reject cleanup while the current card changes.
  }
}

function waitForReviewExit(): Promise<void> {
  if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return Promise.resolve();
  return new Promise((resolve) => window.setTimeout(resolve, REVIEW_EXIT_MS));
}
