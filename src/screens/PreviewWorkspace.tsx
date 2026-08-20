import {
  ArrowLeft,
  ArrowSquareOut,
  Check,
  Copy,
  CornersIn,
  CornersOut,
  Crosshair,
  FlagBanner,
  FolderOpen,
  Heart,
  Pause,
  PencilSimple,
  Play,
  Plus,
  SpeakerHigh,
  SpeakerSlash,
  Skull,
  Tag as TagIcon,
  Timer,
  UserCircle,
  X,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { commandErrorMessage, displayHighlightTitle, getClipMedia } from "../api/backend";
import { FeedbackDialog } from "../components/FeedbackDialog";
import { ThumbnailImage } from "../components/ThumbnailImage";
import {
  PLAYBACK_KEY_SHORTCUTS,
  isPlaybackShortcutFocusProtected,
  usePlaybackShortcuts,
} from "../hooks/usePlaybackShortcuts";
import { useElementFullscreen } from "../hooks/useElementFullscreen";
import { formatBytes, formatDateTime } from "../lib/formatters";
import { previewTimelineMarkerMode } from "../lib/videoTypes";
import type { Clip, ClipEvent, ClipMedia, ClipSummary, Tag } from "../types";

type PreviewWorkspaceProps = {
  clip: Clip | null;
  clips: ClipSummary[];
  detailStatus?: "idle" | "loading" | "ready" | "not-found" | "error";
  detailError?: string | null;
  initialVolumePercent?: number;
  initialMuted?: boolean;
  tags: Tag[];
  activityMessage: string;
  feedbackEndpoint?: string;
  onAudioPreferenceChange?: (preference: {
    volumePercent: number;
    muted: boolean;
  }) => void;
  onBack: () => void;
  onCopyPath: (clipId: string) => void;
  onCreateTag: (name: string) => Promise<Tag | null>;
  onManageTags: () => void;
  onOpenOriginal: (clipId: string) => void;
  onOpenExternal: (clipId: string) => void;
  onRetryDetail?: () => void;
  onSelectClip: (clipId: string) => void;
  onToggleFavorite: (clipId: string) => void;
  onToggleTag: (clipId: string, tagId: string, shouldAttach: boolean) => Promise<void>;
  onUpdateNote: (clipId: string, note: string) => Promise<void>;
};

const PREVIEW_ESCAPE_BLOCKED_LAYER_SELECTOR = [
  ".app-backdrop--sidebar",
  "dialog[open]",
  "[aria-modal='true']",
  "[data-state='open'][role='dialog']",
  "[data-state='open'][role='alertdialog']",
  "[data-state='open'][role='menu']",
  "[data-state='open'][role='listbox']",
  ".ui-dialog-content",
  ".ui-alert-dialog-content",
].join(",");

const PREVIEW_ESCAPE_BLOCKED_CONTROL_SELECTOR = [
  "select[aria-expanded='true']",
  "[role='combobox'][aria-expanded='true']",
  "[role='listbox']",
  "[role='menu']",
  "[cmdk-root]",
  "[cmdk-input]",
  ".ui-command",
].join(",");

export function PreviewWorkspace({
  clip,
  clips,
  detailStatus = clip ? "ready" : "idle",
  detailError = null,
  initialVolumePercent = 100,
  initialMuted = false,
  tags,
  activityMessage,
  feedbackEndpoint = "",
  onAudioPreferenceChange,
  onBack,
  onCopyPath,
  onCreateTag,
  onManageTags,
  onOpenOriginal,
  onOpenExternal,
  onRetryDetail = () => undefined,
  onSelectClip,
  onToggleFavorite,
  onToggleTag,
  onUpdateNote,
}: PreviewWorkspaceProps) {
  const [media, setMedia] = useState<ClipMedia | null>(null);
  const [mediaError, setMediaError] = useState("");
  const [isMediaLoading, setIsMediaLoading] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [mediaDuration, setMediaDuration] = useState<number | null>(null);
  const initialVolume = normalizeVolumePercent(initialVolumePercent) / 100;
  const [volume, setVolume] = useState(initialVolume);
  const [isMuted, setIsMuted] = useState(initialMuted || initialVolume === 0);
  const [embeddedPlaybackFailed, setEmbeddedPlaybackFailed] = useState(false);
  const [noteDraft, setNoteDraft] = useState(clip?.note ?? "");
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [newTagName, setNewTagName] = useState("");
  const [selectedTagId, setSelectedTagId] = useState("");
  const [isFeedbackOpen, setIsFeedbackOpen] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const activeClipIdRef = useRef<string | null>(clip?.id ?? null);
  const mediaRequestTokenRef = useRef(0);
  const noteSaveRequestTokenRef = useRef(0);
  const isApplyingAudioPreferenceRef = useRef(false);
  const lastReportedAudioPreferenceRef = useRef({
    volumePercent: Math.round(initialVolume * 100),
    muted: initialMuted || initialVolume === 0,
  });
  activeClipIdRef.current = clip?.id ?? null;

  const relatedClips = useMemo(() => {
    if (!clip) return [];
    const selectedMatchKey = clip.matchId?.trim() || clip.clipGroupId || `clip:${clip.id}`;
    const sameMatch = clips
      .filter((candidate) => {
        const candidateMatchKey = candidate.matchId?.trim() || candidate.clipGroupId || `clip:${candidate.id}`;
        return candidate.accountId === clip.accountId && candidateMatchKey === selectedMatchKey;
      })
      .sort(
        (left, right) =>
          new Date(left.modifiedAt).getTime() - new Date(right.modifiedAt).getTime(),
      );
    return sameMatch.slice(0, 18);
  }, [clip, clips]);

  const isDetailPending = detailStatus === "idle" || detailStatus === "loading";
  const attachedTags = clip ? tags.filter((tag) => clip.tags.includes(tag.id)) : [];
  const availableTags = clip ? tags.filter((tag) => !clip.tags.includes(tag.id)) : [];
  const activeMedia = media?.clipId === clip?.id ? media : null;
  const activeCurrentTime = activeMedia ? currentTime : 0;
  const activeIsPlaying = activeMedia ? isPlaying : false;
  const activeMediaError = isDetailPending ? "" : mediaError;
  const displayedNoteDraft = isDetailPending ? "" : noteDraft;
  const clipDurationSeconds = clip?.durationMs != null &&
    Number.isFinite(clip.durationMs) &&
    clip.durationMs >= 0
    ? clip.durationMs / 1000
    : null;
  const loadedMediaDurationSeconds = activeMedia &&
    mediaDuration !== null &&
    Number.isFinite(mediaDuration) &&
    mediaDuration >= 0
    ? mediaDuration
    : null;
  const knownDurationSeconds = loadedMediaDurationSeconds ?? clipDurationSeconds;
  const durationSeconds = knownDurationSeconds ?? 0;
  const markerMode = clip ? previewTimelineMarkerMode(clip) : null;
  const timelineEvents = (clip?.clipEvents ?? []).filter(
    (event): event is ClipEvent & { videoTimeMs: number } => {
      if (event.videoTimeMs === null || !Number.isFinite(event.videoTimeMs)) return false;
      const eventSeconds = event.videoTimeMs / 1000;
      if (eventSeconds < 0) return false;
      if (knownDurationSeconds !== null && eventSeconds > knownDurationSeconds) return false;
      const eventType = event.eventType.trim().toLocaleLowerCase("en-US");
      if (markerMode === "kill") return eventType === "kill" && event.killerIsMe;
      if (markerMode === "death") return eventType === "death" && event.killedIsMe;
      return false;
    },
  );
  const progressPercent = durationSeconds > 0
    ? Math.min(100, (activeCurrentTime / durationSeconds) * 100)
    : 0;
  const shortcutsActive = Boolean(
    clip &&
    !isDetailPending &&
    activeMedia?.playable &&
    activeMedia.mediaUrl,
  );
  const { togglePlayback, toggleMute } = usePlaybackShortcuts({
    videoRef,
    active: shortcutsActive,
  });
  const fullscreenEligible = shortcutsActive && !embeddedPlaybackFailed;
  const {
    clearFullscreenError,
    elementRef: playerShellRef,
    exitFullscreen,
    fullscreenError,
    isFullscreen,
    shouldIgnoreEscape: shouldIgnoreFullscreenEscape,
    toggleFullscreen,
  } = useElementFullscreen<HTMLDivElement>({ enabled: fullscreenEligible });

  useEffect(() => {
    if (!clip) return;

    const handlePreviewNavigation = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        event.keyCode === 229 ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        event.shiftKey
      ) {
        return;
      }

      if (event.key === "Escape") {
        if (isPreviewEscapeNavigationBlocked(event)) return;
        if (shouldIgnoreFullscreenEscape()) return;
        event.preventDefault();
        onBack();
        return;
      }

      if (
        event.key.toLocaleLowerCase("en-US") === "f" &&
        shortcutsActive &&
        !isFullscreenShortcutFocusProtected(event)
      ) {
        event.preventDefault();
        void toggleFullscreen();
        return;
      }

      if (!/^[1-9]$/.test(event.key) || isPlaybackShortcutFocusProtected(event)) return;
      const targetClip = relatedClips[Number(event.key) - 1];
      if (targetClip) onSelectClip(targetClip.id);
    };

    window.addEventListener("keydown", handlePreviewNavigation);
    return () => window.removeEventListener("keydown", handlePreviewNavigation);
  }, [clip, onBack, onSelectClip, relatedClips, shortcutsActive, shouldIgnoreFullscreenEscape, toggleFullscreen]);

  useEffect(() => {
    const clipId = clip?.id ?? null;
    const requestToken = ++mediaRequestTokenRef.current;
    setMedia(null);
    setMediaError("");
    setIsMediaLoading(false);
    setIsPlaying(false);
    setCurrentTime(0);
    setMediaDuration(null);
    setEmbeddedPlaybackFailed(false);
    clearFullscreenError();

    if (!clip || !clipId) return;
    setIsMediaLoading(true);
    void getClipMedia(clip.id)
      .then((nextMedia) => {
        if (
          mediaRequestTokenRef.current === requestToken &&
          activeClipIdRef.current === clipId &&
          nextMedia.clipId === clipId
        ) {
          setMedia(nextMedia);
        }
      })
      .catch((error) => {
        if (
          mediaRequestTokenRef.current === requestToken &&
          activeClipIdRef.current === clipId
        ) {
          setMediaError(commandErrorMessage(error));
        }
      })
      .finally(() => {
        if (
          mediaRequestTokenRef.current === requestToken &&
          activeClipIdRef.current === clipId
        ) {
          setIsMediaLoading(false);
        }
      });

    return () => {
      if (mediaRequestTokenRef.current === requestToken) {
        mediaRequestTokenRef.current += 1;
      }
    };
  }, [clip?.id]);

  useEffect(() => {
    setNoteDraft(clip?.note ?? "");
  }, [clip?.id, clip?.note]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    isApplyingAudioPreferenceRef.current = true;
    try {
      video.volume = volume;
      video.muted = isMuted || volume === 0;
    } finally {
      isApplyingAudioPreferenceRef.current = false;
    }
  }, [activeMedia?.mediaUrl, clip?.id, isMuted, volume]);

  useEffect(() => {
    noteSaveRequestTokenRef.current += 1;
    setSaveState("idle");
  }, [clip?.id]);

  if (!clip && (detailStatus === "loading" || detailStatus === "idle")) {
    return (
      <section className="preview-workspace preview-workspace--empty" role="status">
        <Crosshair weight="duotone" />
        <h1>正在加载素材详情</h1>
        <p>只读取当前选中素材的备注、事件与完整标签。</p>
        <button className="cinematic-button cinematic-button--secondary" type="button" onClick={onBack}>返回素材库</button>
      </section>
    );
  }

  if (!clip && (detailStatus === "error" || detailStatus === "not-found")) {
    return (
      <section className="preview-workspace preview-workspace--empty" role="alert">
        <Crosshair weight="duotone" />
        <h1>{detailStatus === "not-found" ? "素材已不存在" : "素材详情加载失败"}</h1>
        <p>{detailError || "请返回素材库刷新后重试。"}</p>
        <div>
          <button className="cinematic-button cinematic-button--primary" type="button" onClick={onRetryDetail}>重试详情</button>
          <button className="cinematic-button cinematic-button--secondary" type="button" onClick={onBack}>返回素材库</button>
        </div>
      </section>
    );
  }

  if (!clip) {
    return (
      <section className="preview-workspace preview-workspace--empty">
        <Crosshair weight="duotone" />
        <h1>请选择一个高光片段</h1>
        <button className="cinematic-button cinematic-button--primary" type="button" onClick={onBack}>返回素材库</button>
      </section>
    );
  }

  const seekTo = (seconds: number) => {
    const bounded = Math.max(0, Math.min(durationSeconds || seconds, seconds));
    if (videoRef.current) videoRef.current.currentTime = bounded;
    setCurrentTime(bounded);
  };

  const updateVolume = (nextVolume: number) => {
    const boundedVolume = Math.max(0, Math.min(1, nextVolume));
    const video = videoRef.current;
    if (!video) {
      setVolume(boundedVolume);
      setIsMuted(boundedVolume === 0);
      return;
    }

    isApplyingAudioPreferenceRef.current = true;
    try {
      video.volume = boundedVolume;
      video.muted = boundedVolume === 0;
    } finally {
      isApplyingAudioPreferenceRef.current = false;
    }
    syncAudioPreference(video);
  };

  const syncAudioPreference = (video: HTMLVideoElement) => {
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
      previousPreference.volumePercent === nextPreference.volumePercent &&
      previousPreference.muted === nextPreference.muted
    ) {
      return;
    }
    lastReportedAudioPreferenceRef.current = nextPreference;
    onAudioPreferenceChange?.(nextPreference);
  };

  const toggleAudioMute = () => {
    isApplyingAudioPreferenceRef.current = true;
    try {
      toggleMute();
    } finally {
      isApplyingAudioPreferenceRef.current = false;
    }
    const video = videoRef.current;
    if (video) syncAudioPreference(video);
  };

  const saveNote = async () => {
    if (noteDraft === clip.note || saveState === "saving") return;
    const clipId = clip.id;
    const requestToken = ++noteSaveRequestTokenRef.current;
    setSaveState("saving");
    try {
      await onUpdateNote(clip.id, noteDraft);
      if (
        noteSaveRequestTokenRef.current === requestToken &&
        activeClipIdRef.current === clipId
      ) {
        setSaveState("saved");
      }
    } catch {
      if (
        noteSaveRequestTokenRef.current === requestToken &&
        activeClipIdRef.current === clipId
      ) {
        setSaveState("error");
      }
    }
  };

  const createTag = async (event: FormEvent) => {
    event.preventDefault();
    const name = newTagName.trim();
    if (!name) return;
    const created = await onCreateTag(name);
    if (created) {
      await onToggleTag(clip.id, created.id, true);
      setNewTagName("");
    }
  };

  const addSelectedTag = async () => {
    if (!selectedTagId) return;
    await onToggleTag(clip.id, selectedTagId, true);
    setSelectedTagId("");
  };

  return (
    <section
      aria-busy={isDetailPending}
      aria-label="素材预览"
      className="preview-workspace"
    >
      <aside className="preview-clip-rail">
        <button
          aria-keyshortcuts="Escape"
          className="preview-back-button"
          title="返回素材库（Esc）"
          type="button"
          onClick={onBack}
        >
          <ArrowLeft weight="bold" />
          返回素材库
        </button>
        <div className="preview-rail-heading">
          <span>当前对局</span>
          <strong>{clip.accountDisplayName}</strong>
          <small>
            {relatedClips.length} 条片段
            {relatedClips.length > 1
              ? ` · 数字键 1–${Math.min(9, relatedClips.length)} 切换`
              : null}
          </small>
        </div>
        <div className="preview-rail-list">
          {relatedClips.map((candidate, index) => (
            <button
              aria-current={candidate.id === clip.id ? "true" : undefined}
              aria-keyshortcuts={index < 9 ? String(index + 1) : undefined}
              className={candidate.id === clip.id ? "preview-rail-clip preview-rail-clip--active" : "preview-rail-clip"}
              key={candidate.id}
              title={index < 9 ? `选择第 ${index + 1} 条片段（数字键 ${index + 1}）` : undefined}
              type="button"
              onClick={() => onSelectClip(candidate.id)}
            >
              <ClipArtwork clip={candidate} />
              <span>
                <strong>{clipTitle(candidate)}</strong>
                <small>{formatDuration(candidate.durationMs)} · {formatBytes(candidate.sizeBytes)}</small>
              </span>
              <b>{String(index + 1).padStart(2, "0")}</b>
            </button>
          ))}
        </div>
        <footer><Timer weight="duotone" />{activityMessage}</footer>
      </aside>

      <main className="preview-stage-column">
        <header className="preview-breadcrumb">
          <div>
            <strong>{clip.accountDisplayName}</strong>
            <span>/</span>
            <span>{clipTitle(clip)}</span>
            <b>关键时刻</b>
          </div>
          <div className="preview-breadcrumb-actions">
            <button
              className="cinematic-button cinematic-button--secondary cinematic-button--small"
              title="视频内容与信息不符？上报问题给开发者"
              type="button"
              onClick={() => setIsFeedbackOpen(true)}
            >
              <FlagBanner weight="bold" />反馈问题
            </button>
            <button
              className="cinematic-button cinematic-button--secondary cinematic-button--small"
              title="使用系统默认播放器打开源视频"
              type="button"
              onClick={() => onOpenExternal(clip.id)}
            >
              <ArrowSquareOut weight="bold" />打开源文件
            </button>
          </div>
        </header>

        <div
          className={isFullscreen
            ? "preview-player-shell preview-player-shell--fullscreen"
            : "preview-player-shell"}
          ref={playerShellRef}
        >
        <div
          className="preview-video-stage"
          onDoubleClick={(event) => {
            if (event.target instanceof Element && event.target.closest("button")) return;
            void toggleFullscreen();
          }}
        >
          {activeMedia?.playable && activeMedia.mediaUrl ? (
            <video
              key={clip.id}
              poster={clip.thumbnailUrl ?? undefined}
              ref={videoRef}
              preload="metadata"
              src={activeMedia.mediaUrl}
              onDurationChange={(event) => {
                const nextDuration = event.currentTarget.duration;
                setMediaDuration(
                  Number.isFinite(nextDuration) && nextDuration >= 0
                    ? nextDuration
                    : null,
                );
              }}
              onEnded={() => setIsPlaying(false)}
              onError={() => {
                setIsPlaying(false);
                setEmbeddedPlaybackFailed(true);
                setMediaError("当前系统无法内嵌播放此视频");
                void exitFullscreen();
              }}
              onPause={() => setIsPlaying(false)}
              onPlay={() => setIsPlaying(true)}
              onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)}
              onVolumeChange={(event) => {
                if (!isApplyingAudioPreferenceRef.current) {
                  syncAudioPreference(event.currentTarget);
                }
              }}
            />
          ) : (
            <ClipArtwork clip={clip} hero />
          )}
          {(!activeMedia?.playable || !activeIsPlaying) ? (
            <button
              aria-label="播放视频"
              aria-keyshortcuts={PLAYBACK_KEY_SHORTCUTS.togglePlayback}
              className="preview-video-play"
              disabled={isMediaLoading || !activeMedia?.playable}
              title={activeMediaError || activeMedia?.message || "播放 / 暂停（空格或 K）"}
              type="button"
              onClick={() => void togglePlayback()}
            >
              <Play weight="fill" />
            </button>
          ) : null}
          {isMediaLoading ? <span className="preview-media-state">正在检查本地视频…</span> : null}
          {activeMediaError ? <span className="preview-media-state preview-media-state--error">{activeMediaError}</span> : null}
          {fullscreenError ? (
            <span className="preview-media-state preview-media-state--fullscreen" role="status">
              {fullscreenError}
            </span>
          ) : null}
          {embeddedPlaybackFailed && activeMedia?.playable ? (
            <div className="preview-external-fallback" role="alert">
              <span>视频已安全入库，但当前 WebView2 解码链无法播放。</span>
              <button type="button" onClick={() => onOpenExternal(clip.id)}>
                <ArrowSquareOut weight="bold" />使用系统默认播放器打开
              </button>
            </div>
          ) : null}
        </div>

        <div className="preview-player-controls">
          <button
            aria-keyshortcuts={PLAYBACK_KEY_SHORTCUTS.togglePlayback}
            aria-label={activeIsPlaying ? "暂停" : "播放"}
            disabled={!activeMedia?.playable}
            title="播放 / 暂停（空格或 K）"
            type="button"
            onClick={togglePlayback}
          >
            {activeIsPlaying ? <Pause weight="fill" /> : <Play weight="fill" />}
          </button>
          <div className="preview-volume-control">
            <button
              aria-label={isMuted || volume === 0 ? "恢复声音" : "静音"}
              aria-keyshortcuts={PLAYBACK_KEY_SHORTCUTS.toggleMute}
              aria-pressed={isMuted || volume === 0}
              disabled={!activeMedia?.playable}
              title={`${isMuted || volume === 0 ? "恢复声音" : "静音"}（M）`}
              type="button"
              onClick={toggleAudioMute}
            >
              {isMuted || volume === 0
                ? <SpeakerSlash weight="duotone" />
                : <SpeakerHigh weight="duotone" />}
            </button>
            <input
              aria-label="音量"
              aria-valuetext={`${Math.round((isMuted ? 0 : volume) * 100)}%`}
              disabled={!activeMedia?.playable}
              max="1"
              min="0"
              step="0.05"
              type="range"
              value={isMuted ? 0 : volume}
              onChange={(event) => updateVolume(Number(event.currentTarget.value))}
            />
          </div>
          <span><b>{formatSeconds(activeCurrentTime)}</b> / {formatSeconds(durationSeconds)}</span>
          <i />
          <span className="preview-quality">1080P</span>
          <button
            aria-keyshortcuts="F"
            aria-label={isFullscreen ? "退出全屏" : "进入全屏"}
            aria-pressed={isFullscreen}
            className="preview-fullscreen-button"
            disabled={!fullscreenEligible}
            title={isFullscreen ? "退出全屏（F 或 Esc）" : "进入全屏（F 或双击视频）"}
            type="button"
            onClick={() => void toggleFullscreen()}
          >
            {isFullscreen ? <CornersIn weight="bold" /> : <CornersOut weight="bold" />}
          </button>
        </div>

        <div className="preview-timeline" aria-label="视频时间轴">
          <div className="preview-timeline-track">
            <button
              aria-label="调整播放进度"
              aria-keyshortcuts={PLAYBACK_KEY_SHORTCUTS.seek}
              title="点击跳转；← / → 5 秒，Shift + ← / → 或 J / L 10 秒"
              type="button"
              onClick={(event) => {
                const rect = event.currentTarget.getBoundingClientRect();
                seekTo(((event.clientX - rect.left) / rect.width) * durationSeconds);
              }}
            />
            {timelineEvents.map((event, index) => {
              const eventSeconds = event.videoTimeMs / 1000;
              const left = durationSeconds > 0 ? (eventSeconds / durationSeconds) * 100 : 0;
              const markerLabel = markerMode === "death" ? "本人死亡" : "本人击杀";
              const markerTitle = `${markerLabel} · ${formatSeconds(eventSeconds)}`;
              return (
                <button
                  aria-label={markerTitle}
                  className={`preview-timeline-flag preview-timeline-flag--${markerMode}`}
                  key={event.id || `${event.videoTimeMs}-${index}`}
                  style={{ left: `${left}%` }}
                  title={markerTitle}
                  type="button"
                  onClick={() => seekTo(eventSeconds)}
                >
                  {markerMode === "death"
                    ? <Skull aria-hidden="true" className="preview-timeline-icon--death" weight="fill" />
                    : <Crosshair aria-hidden="true" className="preview-timeline-icon--kill" weight="bold" />}
                </button>
              );
            })}
            <span className="preview-timeline-progress" style={{ width: `${progressPercent}%` }} />
            <span className="preview-timeline-playhead" style={{ left: `${progressPercent}%` }} />
          </div>
          <div className="preview-timeline-scale"><span>00:00</span><span>{formatSeconds(durationSeconds / 3)}</span><span>{formatSeconds(durationSeconds * 2 / 3)}</span><span>{formatSeconds(durationSeconds)}</span></div>
          <div aria-label="时间轴标记图例" className="preview-timeline-legend">
            <span className="preview-timeline-legend--kill"><Crosshair aria-hidden="true" weight="bold" />本人击杀</span>
            <span className="preview-timeline-legend--death"><Skull aria-hidden="true" weight="fill" />本人死亡</span>
          </div>
        </div>
        </div>
      </main>

      <aside className="preview-intel-panel">
        <header><span>素材整理</span><small>{isDetailPending ? "SYNCING" : "ORGANIZE"}</small></header>

        <button
          aria-label={clip.isFavorite ? "取消收藏" : "收藏"}
          aria-pressed={clip.isFavorite}
          className={clip.isFavorite ? "preview-favorite-card preview-favorite-card--active" : "preview-favorite-card"}
          type="button"
          onClick={() => onToggleFavorite(clip.id)}
        >
          <span className="preview-favorite-card__icon">
            <Heart weight={clip.isFavorite ? "fill" : "regular"} />
          </span>
          <span className="preview-favorite-card__copy">
            <strong>{clip.isFavorite ? "已收藏" : "收藏此片段"}</strong>
            <small>{clip.isFavorite ? "已加入收藏，可在素材库快速找到" : "标记重要片段，方便之后快速回看"}</small>
          </span>
          <span className="preview-favorite-card__action">
            {clip.isFavorite ? "移除" : "添加"}
          </span>
        </button>

        <section className="preview-detail-section preview-tag-section">
          <div className="preview-detail-heading">
            <span><TagIcon weight="duotone" />自定义标签</span>
            <button className="preview-manage-tags" type="button" onClick={onManageTags}>
              <PencilSimple weight="bold" />管理
            </button>
          </div>
          <p className="preview-detail-description">添加你创建的标签，用于整理和筛选高光片段</p>
          <div className="preview-tag-list">
            {attachedTags.map((tag) => (
              <button key={tag.id} type="button" onClick={() => void onToggleTag(clip.id, tag.id, false)}>
                {tag.label}<X weight="bold" />
              </button>
            ))}
            {attachedTags.length === 0 ? <small>暂无标签</small> : null}
          </div>
          <div className="preview-tag-add">
            <select aria-label="选择已有标签" value={selectedTagId} onChange={(event) => setSelectedTagId(event.currentTarget.value)}>
              <option value="">选择已有标签</option>
              {availableTags.map((tag) => <option key={tag.id} value={tag.id}>{tag.label}</option>)}
            </select>
            <button aria-label="添加选中标签" disabled={!selectedTagId} type="button" onClick={() => void addSelectedTag()}><Plus weight="bold" /></button>
          </div>
          <form className="preview-tag-create" onSubmit={createTag}>
            <input maxLength={24} placeholder="新标签名称" value={newTagName} onChange={(event) => setNewTagName(event.currentTarget.value)} />
            <button disabled={!newTagName.trim()} type="submit">创建</button>
          </form>
        </section>

        <section className="preview-detail-section preview-info-section">
          <div className="preview-detail-heading"><span>视频信息</span></div>
          <dl className="preview-intel-list">
            <div><dt>账号</dt><dd><UserCircle weight="duotone" />{clip.accountDisplayName}</dd></div>
            <div><dt>来源</dt><dd>{sourceKindLabel(clip.sourceKind)} · {clip.sourceDirName}</dd></div>
            <div><dt>相对目录</dt><dd>{clip.sourceRelativeDir || "根目录"}</dd></div>
            <div><dt>英雄</dt><dd>{clip.agentName || "未知"}</dd></div>
            <div><dt>地图</dt><dd>{clip.mapName || "未知"}</dd></div>
            <div><dt>模式</dt><dd>{clip.gameMode || "未知"}</dd></div>
            <div><dt>{clip.matchStartedAt ? "比赛时间" : "有效录制日期"}</dt><dd>{formatDateTime(clip.matchStartedAt ?? clip.createdAt)}</dd></div>
            <div><dt>文件大小</dt><dd>{formatBytes(clip.sizeBytes)}</dd></div>
            <div><dt>时长</dt><dd>{formatDuration(clip.durationMs)}</dd></div>
          </dl>
        </section>

        <section className="preview-detail-section preview-note-section">
          <div className="preview-detail-heading"><span>备注</span><small>{displayedNoteDraft.length}/200</small></div>
          <textarea
            disabled={isDetailPending}
            maxLength={200}
            placeholder={isDetailPending ? "正在加载备注…" : "记录复盘重点或剪辑思路"}
            value={displayedNoteDraft}
            onBlur={() => void saveNote()}
            onChange={(event) => {
              setNoteDraft(event.currentTarget.value);
              setSaveState("idle");
            }}
          />
          <button disabled={isDetailPending || noteDraft === clip.note || saveState === "saving"} type="button" onClick={() => void saveNote()}>
            {isDetailPending ? "正在同步…" : saveState === "saving" ? "保存中…" : saveState === "saved" ? <><Check weight="bold" />已保存</> : saveState === "error" ? "重试保存" : "保存备注"}
          </button>
        </section>

        <footer className="preview-file-actions">
          <button type="button" onClick={() => onOpenOriginal(clip.id)}><FolderOpen weight="bold" />打开位置</button>
          <button type="button" onClick={() => onCopyPath(clip.id)}><Copy weight="bold" />复制路径</button>
        </footer>
      </aside>
      <FeedbackDialog
        clip={clip}
        endpoint={feedbackEndpoint}
        open={isFeedbackOpen}
        onOpenChange={setIsFeedbackOpen}
      />
    </section>
  );
}

function ClipArtwork({ clip, hero = false }: { clip: ClipSummary; hero?: boolean }) {
  return (
    <div className={hero ? "cinematic-artwork cinematic-artwork--hero" : "cinematic-artwork"}>
      <ThumbnailImage
        alt=""
        decoding="async"
        fallback={<span aria-hidden="true" className="cinematic-artwork-fallback" />}
        src={clip.thumbnailUrl}
      />
      <span aria-hidden="true" className="cinematic-artwork-shade" />
    </div>
  );
}

function sourceKindLabel(kind: Clip["sourceKind"]): string {
  switch (kind) {
    case "nvidia": return "NVIDIA";
    case "tracker": return "Tracker";
    case "generic": return "普通目录";
    case "aclos": return "ACLOS";
  }
}

function clipTitle(clip: ClipSummary): string {
  return displayHighlightTitle(clip);
}

function isPreviewEscapeNavigationBlocked(event: KeyboardEvent): boolean {
  const ownerDocument = event.view?.document ?? document;
  if (ownerDocument.querySelector(PREVIEW_ESCAPE_BLOCKED_LAYER_SELECTOR)) return true;

  const target = event.target instanceof Element ? event.target : ownerDocument.activeElement;
  return Boolean(target?.closest(PREVIEW_ESCAPE_BLOCKED_CONTROL_SELECTOR));
}

function isFullscreenShortcutFocusProtected(event: KeyboardEvent): boolean {
  const ownerDocument = event.view?.document ?? document;
  if (
    ownerDocument.querySelector(PREVIEW_ESCAPE_BLOCKED_LAYER_SELECTOR)
    || ownerDocument.querySelector(PREVIEW_ESCAPE_BLOCKED_CONTROL_SELECTOR)
  ) {
    return true;
  }
  const target = event.target instanceof Element ? event.target : ownerDocument.activeElement;
  if (target?.closest(".preview-fullscreen-button")) return false;
  return isPlaybackShortcutFocusProtected(event);
}

function formatDuration(durationMs: number | null): string {
  return formatSeconds((durationMs ?? 0) / 1000);
}

function formatSeconds(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "00:00";
  const total = Math.round(value);
  return `${Math.floor(total / 60).toString().padStart(2, "0")}:${(total % 60).toString().padStart(2, "0")}`;
}

function normalizeVolumePercent(volumePercent: number): number {
  if (!Number.isFinite(volumePercent)) return 100;
  return Math.round(Math.max(0, Math.min(100, volumePercent)));
}
