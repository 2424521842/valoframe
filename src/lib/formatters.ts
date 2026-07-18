export function formatBytes(bytes: number): string {
  if (bytes < 1_000_000) {
    return `${Math.round(bytes / 1_000)} KB`;
  }

  if (bytes < 1_000_000_000) {
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
}

export function formatDateTime(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}
