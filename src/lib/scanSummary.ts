import type { FullDriveScanResult, ScanJobStatus, ScanSummary } from "../types";
import { formatDateTime } from "./formatters";

export const DEFAULT_ACLOS_MISSING_MESSAGE =
  "未找到默认无畏时刻目录，你可以手动添加目录。";

export function isDefaultAclosDirMissing(summary: ScanSummary): boolean {
  return summary.message?.toLowerCase().includes("scan root not found") ?? false;
}

export function scanActivityMessage(summary: ScanSummary): string {
  if (isDefaultAclosDirMissing(summary)) return DEFAULT_ACLOS_MISSING_MESSAGE;

  const baseMessage = `扫描完成：${formatDateTime(new Date().toISOString())}`;
  if (!hasMetadataCounts(summary)) return baseMessage;
  return `${baseMessage} · 元数据 ${summary.metadataMatchCount ?? 0} 场 / ${
    summary.metadataEnrichedClipCount ?? 0
  } 个片段`;
}

export function scanTerminalActivityMessage(
  status: ScanJobStatus,
  summary: ScanSummary | null | undefined,
): string {
  const count = summary ? summary.newClipCount.toLocaleString("zh-CN") : null;
  const pendingCount = summary?.pendingClipCount ?? 0;
  const pendingSuffix = pendingCount > 0
    ? `，另有 ${pendingCount.toLocaleString("zh-CN")} 个 NVIDIA 视频待手动录入`
    : "";
  switch (status) {
    case "completed":
      return count === null
        ? "扫描完成：新增数量不可用"
        : `扫描完成：新增 ${count} 个视频${pendingSuffix}`;
    case "partial":
      return count === null
        ? "扫描部分完成：新增数量不可用"
        : `扫描部分完成：已安全新增 ${count} 个视频${pendingSuffix}`;
    case "cancelled":
      return count === null
        ? "扫描已取消：新增数量不可用"
        : `扫描已取消：已安全新增 ${count} 个视频${pendingSuffix}`;
    case "failed":
      return count === null
        ? "扫描失败：新增数量不可用"
        : `扫描失败：已安全新增 ${count} 个视频${pendingSuffix}`;
    default:
      return "扫描新增数量不可用";
  }
}

export function fullDriveDiscoveryActivityMessage(
  result: FullDriveScanResult,
): string {
  if (result.validatedSourceDirCount === 0) {
    return "未发现标准无畏时刻素材";
  }

  return `全电脑发现完成：${result.validatedSourceDirCount} 个素材目录，扫描 ${result.scannedClipCount} 个视频`;
}

function hasMetadataCounts(summary: ScanSummary): boolean {
  return [
    summary.metadataMatchCount,
    summary.metadataEnrichedClipCount,
    summary.metadataWarningCount,
  ].some((value) => typeof value === "number");
}
