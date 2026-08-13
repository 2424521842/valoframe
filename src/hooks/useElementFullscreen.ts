import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

const FULLSCREEN_EXIT_KEY_GUARD_MS = 350;

type UseElementFullscreenOptions = {
  enabled: boolean;
};

type ElementFullscreenControls<T extends HTMLElement> = {
  clearFullscreenError: () => void;
  elementRef: RefObject<T | null>;
  exitFullscreen: () => Promise<void>;
  fullscreenError: string;
  isFullscreen: boolean;
  shouldIgnoreEscape: () => boolean;
  toggleFullscreen: () => Promise<void>;
};

/** Keeps app controls in sync with the browser-owned Fullscreen API state. */
export function useElementFullscreen<T extends HTMLElement>({
  enabled,
}: UseElementFullscreenOptions): ElementFullscreenControls<T> {
  const elementRef = useRef<T>(null);
  const fullscreenStateRef = useRef(false);
  const lastFullscreenExitAtRef = useRef(Number.NEGATIVE_INFINITY);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [fullscreenError, setFullscreenError] = useState("");

  const clearFullscreenError = useCallback(() => setFullscreenError(""), []);

  const exitFullscreen = useCallback(async () => {
    const element = elementRef.current;
    if (!element || document.fullscreenElement !== element) return;
    if (typeof document.exitFullscreen !== "function") {
      setFullscreenError("无法退出全屏，请按 Esc 重试");
      return;
    }

    try {
      await document.exitFullscreen();
    } catch {
      setFullscreenError("无法退出全屏，请按 Esc 重试");
    }
  }, []);

  const toggleFullscreen = useCallback(async () => {
    const element = elementRef.current;
    if (!element) return;

    setFullscreenError("");
    if (document.fullscreenElement === element) {
      await exitFullscreen();
      return;
    }
    if (!enabled) return;
    if (typeof element.requestFullscreen !== "function") {
      setFullscreenError("当前环境无法进入全屏");
      return;
    }

    try {
      await element.requestFullscreen();
    } catch {
      setFullscreenError("当前环境无法进入全屏");
    }
  }, [enabled, exitFullscreen]);

  const shouldIgnoreEscape = useCallback(() => {
    const element = elementRef.current;
    return fullscreenStateRef.current
      || (element !== null && document.fullscreenElement === element)
      || Date.now() - lastFullscreenExitAtRef.current < FULLSCREEN_EXIT_KEY_GUARD_MS;
  }, []);

  useEffect(() => {
    const syncFullscreenState = () => {
      const element = elementRef.current;
      const nextIsFullscreen = element !== null && document.fullscreenElement === element;
      if (fullscreenStateRef.current && !nextIsFullscreen) {
        lastFullscreenExitAtRef.current = Date.now();
      }
      fullscreenStateRef.current = nextIsFullscreen;
      setIsFullscreen(nextIsFullscreen);
      if (nextIsFullscreen) setFullscreenError("");
    };
    const reportFullscreenError = () => {
      const element = elementRef.current;
      setFullscreenError(
        element !== null && document.fullscreenElement === element
          ? "无法退出全屏，请按 Esc 重试"
          : "当前环境无法进入全屏",
      );
    };

    document.addEventListener("fullscreenchange", syncFullscreenState);
    document.addEventListener("fullscreenerror", reportFullscreenError);
    return () => {
      document.removeEventListener("fullscreenchange", syncFullscreenState);
      document.removeEventListener("fullscreenerror", reportFullscreenError);
    };
  }, []);

  return {
    clearFullscreenError,
    elementRef,
    exitFullscreen,
    fullscreenError,
    isFullscreen,
    shouldIgnoreEscape,
    toggleFullscreen,
  };
}
