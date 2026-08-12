import { useCallback, useEffect, useRef, type RefObject } from "react";

export const PLAYBACK_KEY_SHORTCUTS = {
  togglePlayback: "Space K",
  seek: "ArrowLeft ArrowRight Shift+ArrowLeft Shift+ArrowRight J L",
  toggleMute: "M",
} as const;

type PlaybackShortcutAction =
  | { type: "toggle-playback"; preventDefault: boolean }
  | { type: "seek"; offsetSeconds: number; preventDefault: boolean }
  | { type: "toggle-mute"; preventDefault: boolean };

export type PlaybackShortcutFocusGuard = (event: KeyboardEvent) => boolean;

type UsePlaybackShortcutsOptions = {
  videoRef: RefObject<HTMLVideoElement | null>;
  active: boolean;
  isFocusProtected?: PlaybackShortcutFocusGuard;
  /** Review cards reserve the arrow keys for decisions instead of seeking. */
  enableArrowSeek?: boolean;
};

type PlaybackShortcutControls = {
  togglePlayback: () => void;
  toggleMute: () => void;
};

const FOCUS_PROTECTED_SELECTOR = [
  "input",
  "textarea",
  "select",
  "button",
  "[contenteditable]:not([contenteditable='false'])",
  "[role='textbox']",
  "[role='combobox']",
  "[role='listbox']",
  "[role='option']",
  "[role='dialog']",
  "[role='alertdialog']",
  "[aria-modal='true']",
  "[cmdk-root]",
  "[cmdk-input]",
  ".ui-command",
].join(",");

const OPEN_MODAL_SELECTOR = [
  "dialog[open]",
  "[aria-modal='true']",
  "[data-state='open'][role='dialog']",
  "[data-state='open'][role='alertdialog']",
  ".ui-dialog-content",
  ".ui-alert-dialog-content",
].join(",");

export function isPlaybackShortcutFocusProtected(event: KeyboardEvent): boolean {
  const ownerDocument = event.view?.document ?? document;
  const target = event.target instanceof Element ? event.target : null;
  const activeElement = ownerDocument.activeElement;

  if (isProtectedElement(target) || isProtectedElement(activeElement)) return true;
  return ownerDocument.querySelector(OPEN_MODAL_SELECTOR) !== null;
}

export function usePlaybackShortcuts({
  videoRef,
  active,
  isFocusProtected = isPlaybackShortcutFocusProtected,
  enableArrowSeek = true,
}: UsePlaybackShortcutsOptions): PlaybackShortcutControls {
  const lastAudibleVolumeRef = useRef(1);

  const togglePlayback = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;

    try {
      if (video.paused) {
        const playResult = video.play();
        void playResult?.catch(() => undefined);
      } else {
        video.pause();
      }
    } catch {
      // A media element can reject commands while its source is changing.
    }
  }, [videoRef]);

  const toggleMute = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;

    const currentVolume = normalizedVolume(video.volume);
    if (video.muted || currentVolume === 0) {
      if (currentVolume === 0) {
        video.volume = normalizedAudibleVolume(lastAudibleVolumeRef.current);
      }
      video.muted = false;
      return;
    }

    lastAudibleVolumeRef.current = currentVolume;
    video.muted = true;
  }, [videoRef]);

  const seekBy = useCallback((offsetSeconds: number) => {
    const video = videoRef.current;
    if (!video || !Number.isFinite(video.duration) || video.duration < 0) return;

    const currentTime = Number.isFinite(video.currentTime) ? video.currentTime : 0;
    const targetTime = Math.max(0, Math.min(video.duration, currentTime + offsetSeconds));
    try {
      video.currentTime = targetTime;
    } catch {
      // Metadata can become unavailable between reading duration and seeking.
    }
  }, [videoRef]);

  useEffect(() => {
    if (!active) return;

    const rememberAudibleVolume = (event: Event) => {
      const video = videoRef.current;
      if (
        event.target !== video ||
        !video ||
        video.muted ||
        !Number.isFinite(video.volume) ||
        video.volume <= 0
      ) {
        return;
      }
      lastAudibleVolumeRef.current = video.volume;
    };

    const video = videoRef.current;
    if (video && !video.muted && video.volume > 0) {
      lastAudibleVolumeRef.current = normalizedVolume(video.volume);
    }

    document.addEventListener("volumechange", rememberAudibleVolume, true);
    return () => document.removeEventListener("volumechange", rememberAudibleVolume, true);
  }, [active, videoRef]);

  useEffect(() => {
    if (!active) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.isComposing ||
        event.keyCode === 229 ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey
      ) {
        return;
      }

      const action = playbackShortcutAction(event, enableArrowSeek);
      if (!action || isFocusProtected(event) || !videoRef.current) return;

      if (action.preventDefault) event.preventDefault();
      if (event.repeat && (action.type === "toggle-playback" || action.type === "toggle-mute")) {
        return;
      }

      if (action.type === "toggle-playback") {
        togglePlayback();
      } else if (action.type === "toggle-mute") {
        toggleMute();
      } else {
        seekBy(action.offsetSeconds);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [active, enableArrowSeek, isFocusProtected, seekBy, toggleMute, togglePlayback, videoRef]);

  return { togglePlayback, toggleMute };
}

function isProtectedElement(element: Element | null): boolean {
  if (!element) return false;
  if (element instanceof HTMLElement && element.isContentEditable) return true;
  return element.closest(FOCUS_PROTECTED_SELECTOR) !== null;
}

function playbackShortcutAction(
  event: KeyboardEvent,
  enableArrowSeek: boolean,
): PlaybackShortcutAction | null {
  if (enableArrowSeek && event.key === "ArrowLeft") {
    return {
      type: "seek",
      offsetSeconds: event.shiftKey ? -10 : -5,
      preventDefault: true,
    };
  }
  if (enableArrowSeek && event.key === "ArrowRight") {
    return {
      type: "seek",
      offsetSeconds: event.shiftKey ? 10 : 5,
      preventDefault: true,
    };
  }
  if (event.shiftKey) return null;

  const key = event.key.toLowerCase();
  if (key === " " || key === "space" || key === "spacebar") {
    return { type: "toggle-playback", preventDefault: true };
  }
  if (key === "k") {
    return { type: "toggle-playback", preventDefault: false };
  }
  if (key === "j") {
    return { type: "seek", offsetSeconds: -10, preventDefault: false };
  }
  if (key === "l") {
    return { type: "seek", offsetSeconds: 10, preventDefault: false };
  }
  if (key === "m") {
    return { type: "toggle-mute", preventDefault: false };
  }
  return null;
}

function normalizedVolume(volume: number): number {
  if (!Number.isFinite(volume)) return 0;
  return Math.max(0, Math.min(1, volume));
}

function normalizedAudibleVolume(volume: number): number {
  const normalized = normalizedVolume(volume);
  return normalized > 0 ? normalized : 1;
}
