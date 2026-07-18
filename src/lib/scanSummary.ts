import type { FullDriveScanResult, ScanSummary } from "../types";
import { formatDateTime } from "./formatters";

export const DEFAULT_ACLOS_MISSING_MESSAGE =
  "未找到默认无畏时刻目录，你可以手动添加目录。";

export function isDefaultAclosDirMissing(summary: ScanSummary): boolean {
  return summary.message?.toLowerCase().includes("scan root not found") ?? false;
}

export function scanActivityMessage(summary: ScanSummary): string {
  if (isDefaultAclosDirMissing(summary)) {
    return DEFAULT_ACLOS_MISSING_MESSAGE;
  }

  const baseMessage = `扫描完成：${formatDateTime(new Date().toISOString())}`;

  if (!hasMetadataCounts(summary)) {
    return baseMessage;
  }

  return `${baseMessage} · 元数据 ${summary.metadataMatchCount ?? 0} 场 / ${
    summary.metadataEnrichedClipCount ?? 0
  } 个片段`;
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
