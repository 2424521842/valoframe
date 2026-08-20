import type { FeedbackCategory } from "../types";

export const MAX_FEEDBACK_DESCRIPTION_CHARS = 2000;
export const MAX_FEEDBACK_CONTACT_CHARS = 200;
export const MAX_FEEDBACK_ENDPOINT_CHARS = 300;
export const MAX_FEEDBACK_VIDEO_BYTES = 1024 * 1024 * 1024;

export const FEEDBACK_CATEGORY_OPTIONS: ReadonlyArray<{
  value: FeedbackCategory;
  label: string;
  hint: string;
}> = [
  { value: "mismatch", label: "视频内容与信息不匹配", hint: "画面不是当前账号 / 对局 / 片段" },
  { value: "playback", label: "视频无法正常播放", hint: "黑屏、花屏或无法打开" },
  { value: "metadata", label: "标题 / 击杀 / 对局信息错误", hint: "账号、比分、击杀数等信息有误" },
  { value: "other", label: "其他问题", hint: "不属于以上类别的问题" },
];

/** Trims and length-caps the endpoint while typing in settings; invalid URLs are still saved but rejected at submit time. */
export function normalizeFeedbackEndpoint(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length > MAX_FEEDBACK_ENDPOINT_CHARS) {
    return trimmed.slice(0, MAX_FEEDBACK_ENDPOINT_CHARS);
  }
  return trimmed;
}

/** Empty (save-to-file only) is always valid; otherwise HTTPS or localhost HTTP are required. */
export function isValidFeedbackEndpoint(raw: string): boolean {
  const trimmed = raw.trim();
  if (!trimmed) return true;
  const lower = trimmed.toLowerCase();
  const allowed = lower.startsWith("https://")
    || lower.startsWith("http://localhost")
    || lower.startsWith("http://127.0.0.1")
    || lower.startsWith("http://[::1]");
  return allowed && trimmed.length >= 11;
}
