import type { ScanJobStatus, ScanProgress } from "../types";

export type ScanProgressPresentation = {
  percent: number | null;
  determinate: boolean;
  stageLabel: string;
  ariaLabel: string;
  ariaValueText: string;
};

const SCAN_START_PERCENT = 10;
const SCAN_END_PERCENT = 85;

/**
 * Maps backend work into honest UI stages. Scanner counts describe source/file
 * traversal only, so later database work intentionally remains indeterminate.
 */
export function scanProgressPresentation(
  progress: ScanProgress | null,
  fallbackStatus: ScanJobStatus,
): ScanProgressPresentation {
  const status = progress?.status ?? fallbackStatus;

  if (progress?.terminal && status === "completed") {
    return presentation(100, true, "扫描完成");
  }

  if (progress?.terminal) {
    if (status === "partial") {
      return presentation(99, true, "扫描部分完成");
    }
    if (status === "cancelled") {
      return presentation(null, false, "扫描已取消");
    }
    return presentation(null, false, status === "failed" ? "扫描失败" : progress.message);
  }

  if (status === "cancelling" || progress?.phase === "cancelling") {
    return presentation(null, false, "正在完成安全取消…");
  }

  const phase = progress?.phase;
  if (progress !== null && phase === "scanning") {
    const total = progress.total;
    const hasReliableTotal = total !== null && total > 0;
    const stageLabel = hasReliableTotal
      ? `正在扫描录像 · 已处理 ${Math.min(progress.processed, total)} / ${total}`
      : "正在扫描录像";
    if (!hasReliableTotal) return presentation(null, false, stageLabel);

    const ratio = clamp(progress.processed / total, 0, 1);
    const percent = Math.min(
      SCAN_END_PERCENT,
      Math.round(SCAN_START_PERCENT + ratio * (SCAN_END_PERCENT - SCAN_START_PERCENT)),
    );
    return presentation(percent, true, stageLabel);
  }

  if (phase === "importing") {
    return presentation(null, false, "扫描完成，正在导入数据");
  }
  if (phase === "metadata") {
    return presentation(null, false, "正在整理元数据");
  }
  if (phase === "finalizing" || phase === "completed") {
    return presentation(null, false, "正在完成收尾");
  }
  if (phase === "discovering" || phase === "drive-discovery") {
    return presentation(null, false, "正在发现来源");
  }
  if (phase === "starting") {
    return presentation(null, false, "正在准备扫描");
  }

  if (progress === null) {
    if (status === "running") return presentation(null, false, "正在准备扫描");
    if (status === "completed") return presentation(null, false, "扫描完成");
    if (status === "partial") return presentation(null, false, "扫描部分完成");
    if (status === "cancelled") return presentation(null, false, "扫描已取消");
    if (status === "failed") return presentation(null, false, "扫描失败");
    return presentation(null, false, "尚未开始新的扫描");
  }

  return presentation(null, false, progress.message || "正在处理扫描任务");
}

function presentation(
  percent: number | null,
  determinate: boolean,
  stageLabel: string,
): ScanProgressPresentation {
  const safePercent = percent === null ? null : clamp(percent, 0, 100);
  return {
    percent: safePercent,
    determinate,
    stageLabel,
    ariaLabel: `扫描进度：${stageLabel}`,
    ariaValueText: safePercent === null ? stageLabel : `${stageLabel}，${safePercent}%`,
  };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
