import { useEffect, useRef } from "react";
import type { ReviewItemDecision } from "../types";

type ReviewShortcutDecision = Exclude<ReviewItemDecision, "unreviewed">;

type UseReviewShortcutsOptions = {
  active: boolean;
  isBusy: boolean;
  canUndo: boolean;
  onDecision?: (decision: ReviewShortcutDecision) => void;
  onUndo: () => void;
  onTogglePlayback?: () => void;
  onToggleFullscreen?: () => void;
  onRequestExit?: () => void;
  shouldIgnoreFullscreenEscape?: () => boolean;
};

const REVIEW_FOCUS_PROTECTED_SELECTOR = [
  "input",
  "textarea",
  "select",
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

const REVIEW_OPEN_MODAL_SELECTOR = [
  "dialog[open]",
  "[aria-modal='true']",
  "[data-state='open'][role='dialog']",
  "[data-state='open'][role='alertdialog']",
  ".ui-dialog-content",
  ".ui-alert-dialog-content",
].join(",");

/** One keyboard listener for the complete quick-pick command set. */
export function useReviewShortcuts({
  active,
  isBusy,
  canUndo,
  onDecision,
  onUndo,
  onTogglePlayback,
  onToggleFullscreen,
  onRequestExit,
  shouldIgnoreFullscreenEscape,
}: UseReviewShortcutsOptions): void {
  const optionsRef = useRef({
    isBusy,
    canUndo,
    onDecision,
    onUndo,
    onTogglePlayback,
    onToggleFullscreen,
    onRequestExit,
    shouldIgnoreFullscreenEscape,
  });
  optionsRef.current = {
    isBusy,
    canUndo,
    onDecision,
    onUndo,
    onTogglePlayback,
    onToggleFullscreen,
    onRequestExit,
    shouldIgnoreFullscreenEscape,
  };

  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented
        || event.isComposing
        || event.keyCode === 229
        || isReviewShortcutFocusProtected(event)
      ) {
        return;
      }
      const options = optionsRef.current;
      if (event.key === "Escape" && options.onRequestExit) {
        if (options.shouldIgnoreFullscreenEscape?.()) return;
        event.preventDefault();
        options.onRequestExit();
        return;
      }
      if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.repeat) return;

      const decision = reviewDecisionForShortcut(event.key);
      if (decision && options.onDecision) {
        event.preventDefault();
        if (!options.isBusy) options.onDecision(decision);
        return;
      }
      if (event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (!options.isBusy && options.canUndo) options.onUndo();
        return;
      }
      if (isSpaceKey(event.key) && options.onTogglePlayback) {
        event.preventDefault();
        if (!options.isBusy) options.onTogglePlayback();
        return;
      }
      if (event.key.toLowerCase() === "f" && options.onToggleFullscreen) {
        event.preventDefault();
        if (!options.isBusy) options.onToggleFullscreen();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active]);
}

function reviewDecisionForShortcut(key: string): ReviewShortcutDecision | null {
  const normalized = key.toLowerCase();
  if (normalized === "a" || key === "ArrowLeft") return "skipped";
  if (normalized === "s" || key === "ArrowDown") return "pending";
  if (normalized === "d" || key === "ArrowRight") return "selected";
  return null;
}

function isSpaceKey(key: string): boolean {
  return key === " " || key.toLowerCase() === "space" || key.toLowerCase() === "spacebar";
}

function isReviewShortcutFocusProtected(event: KeyboardEvent): boolean {
  const ownerDocument = event.view?.document ?? document;
  const target = event.target instanceof Element ? event.target : null;
  const activeElement = ownerDocument.activeElement;
  return isProtectedReviewElement(target)
    || isProtectedReviewElement(activeElement)
    || ownerDocument.querySelector(REVIEW_OPEN_MODAL_SELECTOR) !== null;
}

function isProtectedReviewElement(element: Element | null): boolean {
  if (!element) return false;
  if (element instanceof HTMLElement && element.isContentEditable) return true;
  return element.closest(REVIEW_FOCUS_PROTECTED_SELECTOR) !== null;
}
