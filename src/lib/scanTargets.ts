import type { ScanSummary, ScanTarget, SourceDir } from "../types";

export function scanTargetFromPath(path: string): ScanTarget {
  const normalizedPath = path.trim().replace(/[\\/]+$/, "");
  const parts = normalizedPath.split(/[\\/]+/).filter(Boolean);
  const name = parts[parts.length - 1] || "自定义目录";

  return {
    id: `manual:${normalizedPath.toLowerCase()}`,
    name,
    path: normalizedPath,
    origin: "manual",
  };
}

export function mergeScanTargets(
  sourceDirs: SourceDir[],
  manualTargets: ScanTarget[],
  excludedPaths: ReadonlySet<string> = new Set(),
): ScanTarget[] {
  const byPath = new Map<string, ScanTarget>();

  for (const source of sourceDirs) {
    const scanRoot = scanTargetPathForSource(source);
    const key = pathKey(scanRoot);
    if (!key || excludedPaths.has(key)) continue;
    byPath.set(key, {
      id: `indexed:${key}`,
      name: pathBaseName(scanRoot) || source.name,
      path: scanRoot,
      origin: "indexed",
    });
  }

  for (const target of manualTargets) {
    const key = pathKey(target.path);
    if (!key || excludedPaths.has(key)) continue;
    byPath.set(key, target);
  }

  return [...byPath.values()];
}

export function scanTargetPathForSource(source: SourceDir): string {
  const configuredRoot = source.scanRootPath.trim().replace(/[\\/]+$/, "");
  const sourcePath = source.path.trim().replace(/[\\/]+$/, "");

  // Schema-v14 backfills old ACLOS rows with scan_root_path = path. The
  // legacy scan_roots command still expects the directory containing the
  // wonderfulVideos* folders, while persistent and recursive sources are
  // scanned from their configured root verbatim.
  if (
    source.scanMode === "aclos-structured" &&
    pathKey(configuredRoot) === pathKey(sourcePath)
  ) {
    return scanRootFromSourcePath(sourcePath);
  }

  return configuredRoot || sourcePath;
}

export function scanRootFromSourcePath(sourcePath: string): string {
  const normalized = sourcePath.trim().replace(/[\\/]+$/, "");
  const separator = normalized.includes("\\") ? "\\" : "/";
  const lastSeparator = normalized.lastIndexOf(separator);

  if (lastSeparator <= 0) return normalized;
  return normalized.slice(0, lastSeparator);
}

export function scanPathKey(path: string): string {
  return pathKey(path);
}

export function mergeScanSummaries(summaries: ScanSummary[]): ScanSummary {
  if (summaries.length === 0) {
    return {
      rootPath: "",
      sourceDirCount: 0,
      clipGroupCount: 0,
      newClipCount: 0,
      updatedClipCount: 0,
      missingClipCount: 0,
      coverMissingCount: 0,
      metadataMatchCount: 0,
      metadataEnrichedClipCount: 0,
      metadataEventCount: 0,
      metadataWarningCount: 0,
      errors: [],
      message: "没有可扫描的目录",
    };
  }

  return summaries.reduce<ScanSummary>(
    (total, current) => ({
      rootPath: summaries.length === 1 ? current.rootPath : "多个扫描目录",
      sourceDirCount: total.sourceDirCount + current.sourceDirCount,
      clipGroupCount: total.clipGroupCount + current.clipGroupCount,
      newClipCount: total.newClipCount + current.newClipCount,
      updatedClipCount: total.updatedClipCount + current.updatedClipCount,
      missingClipCount: total.missingClipCount + current.missingClipCount,
      coverMissingCount: total.coverMissingCount + current.coverMissingCount,
      metadataMatchCount:
        (total.metadataMatchCount ?? 0) + (current.metadataMatchCount ?? 0),
      metadataEnrichedClipCount:
        (total.metadataEnrichedClipCount ?? 0) +
        (current.metadataEnrichedClipCount ?? 0),
      metadataEventCount:
        (total.metadataEventCount ?? 0) + (current.metadataEventCount ?? 0),
      metadataWarningCount:
        (total.metadataWarningCount ?? 0) +
        (current.metadataWarningCount ?? 0),
      errors: [...total.errors, ...current.errors],
      message: current.message ?? total.message,
    }),
    {
      rootPath: "",
      sourceDirCount: 0,
      clipGroupCount: 0,
      newClipCount: 0,
      updatedClipCount: 0,
      missingClipCount: 0,
      coverMissingCount: 0,
      metadataMatchCount: 0,
      metadataEnrichedClipCount: 0,
      metadataEventCount: 0,
      metadataWarningCount: 0,
      errors: [],
      message: null,
    },
  );
}

function pathKey(path: string): string {
  return path.trim().replace(/[\\/]+$/, "").toLowerCase();
}

function pathBaseName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? "";
}
