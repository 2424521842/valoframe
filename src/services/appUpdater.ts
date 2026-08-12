import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import packageJson from "../../package.json";

export type AppUpdateRuntimeInfo = {
  currentVersion: string;
  channel: "stable";
  endpoint: string;
  configured: boolean;
};

export type AppUpdateMetadata = {
  currentVersion: string;
  version: string;
  notes: string;
  publishedAt: string | null;
};

export type AppUpdateDownloadEvent =
  | { event: "Started"; data: { contentLength?: number | null } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Verifying" }
  | { event: "Finished" };

export type AppUpdaterClient = {
  getRuntimeInfo: () => Promise<AppUpdateRuntimeInfo>;
  check: () => Promise<AppUpdateMetadata | null>;
  download: (onEvent: (event: AppUpdateDownloadEvent) => void) => Promise<void>;
  cancelDownload: () => Promise<boolean>;
  discard: () => Promise<boolean>;
  install: () => Promise<void>;
};

export type AppUpdateServiceError = Error & {
  code: string;
  retryable: boolean;
};

export const appUpdaterClient: AppUpdaterClient = {
  async getRuntimeInfo() {
    if (!isTauri()) {
      return {
        currentVersion: packageJson.version,
        channel: "stable",
        endpoint: "",
        configured: false,
      };
    }
    return invoke<AppUpdateRuntimeInfo>("get_app_update_runtime_info");
  },

  async check() {
    assertDesktopUpdater();
    return invoke<AppUpdateMetadata | null>("check_for_app_update");
  },

  async download(onEvent) {
    assertDesktopUpdater();
    const channel = new Channel<AppUpdateDownloadEvent>();
    channel.onmessage = onEvent;
    await invoke("download_app_update", { onEvent: channel });
  },

  async cancelDownload() {
    assertDesktopUpdater();
    return invoke<boolean>("cancel_app_update_download");
  },

  async discard() {
    assertDesktopUpdater();
    return invoke<boolean>("discard_app_update");
  },

  async install() {
    assertDesktopUpdater();
    await invoke("install_app_update");
  },
};

export function normalizeAppUpdateError(error: unknown): AppUpdateServiceError {
  const payload = asErrorRecord(error);
  const message = readableErrorMessage(error, payload);
  const normalized = new Error(message) as AppUpdateServiceError;
  normalized.name = "AppUpdateServiceError";
  normalized.code = typeof payload?.code === "string" && payload.code.trim()
    ? payload.code
    : "update-operation-failed";
  normalized.retryable = typeof payload?.retryable === "boolean"
    ? payload.retryable
    : true;
  return normalized;
}

function assertDesktopUpdater() {
  if (!isTauri()) {
    throw {
      code: "desktop-only",
      message: "应用内更新仅在已安装的桌面版本中可用",
      retryable: false,
    };
  }
}

function asErrorRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object"
    ? value as Record<string, unknown>
    : null;
}

function readableErrorMessage(
  error: unknown,
  payload: Record<string, unknown> | null,
): string {
  if (typeof payload?.message === "string" && payload.message.trim()) {
    return payload.message;
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (typeof error === "number" || typeof error === "boolean" || typeof error === "bigint") {
    return String(error);
  }
  return "未知更新错误";
}
