import {
  ArrowLeft,
  Check,
  Copy,
  Crosshair,
  Flag,
  FolderOpen,
  Heart,
  Pause,
  PencilSimple,
  Play,
  Plus,
  SpeakerHigh,
  SpeakerSlash,
  Tag as TagIcon,
  Timer,
  UserCircle,
  X,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { commandErrorMessage, displayHighlightTitle, getClipMedia } from "../api/backend";
import { ThumbnailImage } from "../components/ThumbnailImage";
import { formatBytes, formatDateTime } from "../lib/formatters";
import type { Clip, ClipEvent, ClipMedia, ClipSummary, Tag } from "../types";

type PreviewWorkspaceProps = {
  clip: Clip | null;
  clips: ClipSummary[];
  detailStatus?: "idle" | "loading" | "ready" | "not-found" | "error";
  detailError?: string | null;
  tags: Tag[];
  activityMessage: string;
  onBack: () => void;
  onCopyPath: (clipId: string) => void;
  onCreateTag: (name: string) => Promise<Tag | null>;
  onManageTags: () => void;
  onOpenOriginal: (clipId: string) => void;
  onRetryDetail?: () => void;
  onSelectClip: (clipId: string) => void;
  onToggleFavorite: (clipId: string) => void;
  onToggleTag: (clipId: string, tagId: string, shouldAttach: boolean) => Promise<void>;
  onUpdateNote: (clipId: string, note: string) => Promise<void>;
};

export function PreviewWorkspace({
  clip,
  clips,
  detailStatus = clip ? "ready" : "idle",
  detailError = null,
  tags,
  activityMessage,
  onBack,
  onCopyPath,
  onCreateTag,
  onManageTags,
  onOpenOriginal,
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
  const [mediaDuration, setMediaDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [isMuted, setIsMuted] = useState(false);
  const [noteDraft, setNoteDraft] = useState(clip?.note ?? "");
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [newTagName, setNewTagName] = useState("");
  const [selectedTagId, setSelectedTagId] = useState("");
  const videoRef = useRef<HTMLVideoElement>(null);
  const activeClipIdRef = useRef<string | null>(clip?.id ?? null);
  const mediaRequestTokenRef = useRef(0);
  const noteSaveRequestTokenRef = useRef(0);
  const lastAudibleVolumeRef = useRef(1);
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
  const durationSeconds = (activeMedia ? mediaDuration : 0) || ((clip?.durationMs ?? 0) / 1000);
  const timelineEvents = (clip?.clipEvents ?? []).filter(
    (event): event is ClipEvent & { videoTimeMs: number } => event.videoTimeMs !== null,
  );
  const progressPercent = durationSeconds > 0
    ? Math.min(100, (activeCurrentTime / durationSeconds) * 100)
    : 0;

  useEffect(() => {
    const clipId = clip?.id ?? null;
    const requestToken = ++mediaRequestTokenRef.current;
    setMedia(null);
    setMediaError("");
    setIsMediaLoading(false);
    setIsPlaying(false);
    setCurrentTime(0);
    setMediaDuration(0);

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
    video.volume = volume;
    video.muted = isMuted || volume === 0;
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

  const togglePlayback = async () => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) {
      await video.play();
    } else {
      video.pause();
    }
  };

  const toggleMute = () => {
    if (isMuted || volume === 0) {
      const restoredVolume = Math.max(0.05, lastAudibleVolumeRef.current);
      setVolume(restoredVolume);
      setIsMuted(false);
      return;
    }

    lastAudibleVolumeRef.current = volume;
    setIsMuted(true);
  };

  const updateVolume = (nextVolume: number) => {
    const boundedVolume = Math.max(0, Math.min(1, nextVolume));
    if (boundedVolume > 0) {
      lastAudibleVolumeRef.current = boundedVolume;
    }
    setVolume(boundedVolume);
    setIsMuted(boundedVolume === 0);
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
        <button className="preview-back-button" type="button" onClick={onBack}>
          <ArrowLeft weight="bold" />
          返回素材库
        </button>
        <div className="preview-rail-heading">
          <span>当前对局</span>
          <strong>{clip.accountDisplayName}</strong>
          <small>{relatedClips.length} 条片段</small>
        </div>
        <div className="preview-rail-list">
          {relatedClips.map((candidate, index) => (
            <button
              aria-current={candidate.id === clip.id ? "true" : undefined}
              className={candidate.id === clip.id ? "preview-rail-clip preview-rail-clip--active" : "preview-rail-clip"}
              key={candidate.id}
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
          <button className="cinematic-button cinematic-button--secondary cinematic-button--small" type="button" onClick={() => onOpenOriginal(clip.id)}>
            <FolderOpen weight="bold" />打开源文件
          </button>
        </header>

        <div className="preview-video-stage">
          {activeMedia?.playable && activeMedia.mediaUrl ? (
            <video
              key={clip.id}
              poster={clip.thumbnailUrl ?? undefined}
              ref={videoRef}
              preload="metadata"
              src={activeMedia.mediaUrl}
              onDurationChange={(event) => setMediaDuration(event.currentTarget.duration || 0)}
              onEnded={() => setIsPlaying(false)}
              onError={() => setMediaError("预览加载失败")}
              onPause={() => setIsPlaying(false)}
              onPlay={() => setIsPlaying(true)}
              onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)}
            />
          ) : (
            <ClipArtwork clip={clip} hero />
          )}
          {(!activeMedia?.playable || !activeIsPlaying) ? (
            <button
              aria-label="播放视频"
              className="preview-video-play"
              disabled={isMediaLoading || !activeMedia?.playable}
              title={activeMediaError || activeMedia?.message || undefined}
              type="button"
              onClick={() => void togglePlayback()}
            >
              <Play weight="fill" />
            </button>
          ) : null}
          {isMediaLoading ? <span className="preview-media-state">正在检查本地视频…</span> : null}
          {activeMediaError ? <span className="preview-media-state preview-media-state--error">{activeMediaError}</span> : null}
        </div>

        <div className="preview-player-controls">
          <button aria-label={activeIsPlaying ? "暂停" : "播放"} disabled={!activeMedia?.playable} type="button" onClick={() => void togglePlayback()}>
            {activeIsPlaying ? <Pause weight="fill" /> : <Play weight="fill" />}
          </button>
          <div className="preview-volume-control">
            <button
              aria-label={isMuted || volume === 0 ? "恢复声音" : "静音"}
              aria-pressed={isMuted || volume === 0}
              disabled={!activeMedia?.playable}
              title={isMuted || volume === 0 ? "恢复声音" : "静音"}
              type="button"
              onClick={toggleMute}
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
          <Crosshair weight="duotone" />
        </div>

        <div className="preview-timeline" aria-label="视频时间轴">
          <div className="preview-timeline-track">
            <button
              aria-label="调整播放进度"
              type="button"
              onClick={(event) => {
                const rect = event.currentTarget.getBoundingClientRect();
                seekTo(((event.clientX - rect.left) / rect.width) * durationSeconds);
              }}
            />
            {timelineEvents.map((event, index) => {
              const left = durationSeconds > 0 ? Math.min(100, (event.videoTimeMs / 1000 / durationSeconds) * 100) : 0;
              return (
                <button
                  aria-label={`${eventLabel(event)} ${formatSeconds(event.videoTimeMs / 1000)}`}
                  className={`preview-timeline-flag preview-timeline-flag--${eventTone(event)}`}
                  key={event.id || `${event.videoTimeMs}-${index}`}
                  style={{ left: `${left}%` }}
                  type="button"
                  onClick={() => seekTo(event.videoTimeMs / 1000)}
                >
                  <Flag weight="fill" />
                </button>
              );
            })}
            <span className="preview-timeline-progress" style={{ width: `${progressPercent}%` }} />
            <span className="preview-timeline-playhead" style={{ left: `${progressPercent}%` }} />
          </div>
          <div><span>00:00</span><span>{formatSeconds(durationSeconds / 3)}</span><span>{formatSeconds(durationSeconds * 2 / 3)}</span><span>{formatSeconds(durationSeconds)}</span></div>
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
            <div><dt>英雄</dt><dd>{clip.agentName || "未知"}</dd></div>
            <div><dt>地图</dt><dd>{clip.mapName || "未知"}</dd></div>
            <div><dt>模式</dt><dd>{clip.gameMode || "未知"}</dd></div>
            <div><dt>比赛时间</dt><dd>{formatDateTime(clip.matchStartedAt ?? clip.createdAt)}</dd></div>
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

function eventTone(event: ClipEvent): "red" | "amber" | "violet" {
  const type = event.eventType.toLowerCase();
  if (type.includes("kill") || type.includes("击杀")) return "red";
  if (type.includes("assist") || type.includes("助攻")) return "amber";
  return "violet";
}

function clipTitle(clip: ClipSummary): string {
  return displayHighlightTitle(clip);
}

function eventLabel(event: ClipEvent): string {
  const type = event.eventType.toLowerCase();
  if (type.includes("kill") || type.includes("击杀")) return "击杀";
  if (type.includes("assist") || type.includes("助攻")) return "助攻";
  return "事件";
}

function formatDuration(durationMs: number | null): string {
  return formatSeconds((durationMs ?? 0) / 1000);
}

function formatSeconds(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "00:00";
  const total = Math.round(value);
  return `${Math.floor(total / 60).toString().padStart(2, "0")}:${(total % 60).toString().padStart(2, "0")}`;
}
