export function normalizeCustomScanPath(value: string | null): string | null {
  const path = value?.trim();
  return path ? path : null;
}
