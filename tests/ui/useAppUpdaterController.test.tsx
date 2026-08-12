import { act, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  AUTO_UPDATE_CHECK_INTERVAL_MS,
  AUTO_UPDATE_CHECK_STORAGE_KEY,
  isAutomaticUpdateCheckDue,
  useAppUpdaterController,
} from "../../src/hooks/useAppUpdaterController";
import type {
  AppUpdateDownloadEvent,
  AppUpdaterClient,
} from "../../src/services/appUpdater";
import {
  appUpdaterClient,
  normalizeAppUpdateError,
} from "../../src/services/appUpdater";

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class MockChannel {
    onmessage: ((event: unknown) => void) | null = null;
  },
  invoke: vi.fn(),
  isTauri: () => true,
}));

describe("appUpdaterClient", () => {
  it("invokes the discard command and returns its boolean result", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockReset();
    invokeMock.mockResolvedValueOnce(true);

    await expect(appUpdaterClient.discard()).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("discard_app_update");
  });
});

describe("useAppUpdaterController", () => {
  it("checks automatically at most once per day and lets manual checks bypass the limit", async () => {
    const now = 2_000_000_000_000;
    const firstNow = () => now;
    const storage = memoryStorage();
    const client = fakeClient();
    client.check.mockResolvedValue(null);

    const first = renderHook(() => useAppUpdaterController({
      client,
      storage,
      now: firstNow,
    }));
    await waitFor(() => expect(client.check).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(first.result.current.phase).toBe("up-to-date"));
    expect(storage.getItem(AUTO_UPDATE_CHECK_STORAGE_KEY)).toBe(String(now));
    first.unmount();

    const secondNow = () => now + AUTO_UPDATE_CHECK_INTERVAL_MS - 1;
    const second = renderHook(() => useAppUpdaterController({
      client,
      storage,
      now: secondNow,
    }));
    await waitFor(() => expect(second.result.current.runtimeInfo).not.toBeNull());
    await waitFor(() => expect(second.result.current.message).toBe("今日已尝试自动更新检查"));
    expect(client.check).toHaveBeenCalledTimes(1);

    await act(async () => {
      await second.result.current.checkManually();
    });
    expect(client.check).toHaveBeenCalledTimes(2);
  });

  it("keeps automatic network failures silent but reports manual failures", async () => {
    const client = fakeClient();
    const now = () => 2_100_000_000_000;
    client.check.mockRejectedValue({
      code: "update-network-error",
      message: "offline",
      retryable: true,
    });
    const automatic = renderHook(() => useAppUpdaterController({
      client,
      storage: memoryStorage(),
      now,
    }));

    await waitFor(() => expect(client.check).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(automatic.result.current.phase).toBe("idle"));
    expect(automatic.result.current.error).toBeNull();
    automatic.unmount();

    const manual = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(manual.result.current.runtimeInfo).not.toBeNull());
    await act(async () => {
      await manual.result.current.checkManually();
    });
    expect(manual.result.current.phase).toBe("error");
    expect(manual.result.current.error?.code).toBe("update-network-error");
  });

  it("runs only one effective automatic check under React StrictMode replay", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(null);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      storage: memoryStorage(),
      now: () => 2_200_000_000_000,
    }), { reactStrictMode: true });

    await waitFor(() => expect(controller.result.current.phase).toBe("up-to-date"));
    expect(client.check).toHaveBeenCalledTimes(1);
  });

  it("checks again when the window regains focus after the daily interval", async () => {
    let currentTime = 2_250_000_000_000;
    const storage = memoryStorage();
    const client = fakeClient();
    client.check.mockResolvedValue(null);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      storage,
      now: () => currentTime,
    }));

    await waitFor(() => expect(controller.result.current.phase).toBe("up-to-date"));
    expect(client.check).toHaveBeenCalledTimes(1);

    currentTime += AUTO_UPDATE_CHECK_INTERVAL_MS;
    act(() => window.dispatchEvent(new Event("focus")));

    await waitFor(() => expect(client.check).toHaveBeenCalledTimes(2));
    expect(storage.getItem(AUTO_UPDATE_CHECK_STORAGE_KEY)).toBe(String(currentTime));
  });

  it("falls back to the in-memory limit when browser storage is unavailable", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(null);
    const unavailableStorage: Pick<Storage, "getItem" | "setItem"> = {
      getItem: () => {
        throw new Error("storage disabled");
      },
      setItem: () => {
        throw new Error("storage disabled");
      },
    };
    const controller = renderHook(() => useAppUpdaterController({
      client,
      storage: unavailableStorage,
      now: () => 2_300_000_000_000,
    }));

    await waitFor(() => expect(controller.result.current.phase).toBe("up-to-date"));
    expect(client.check).toHaveBeenCalledTimes(1);
  });

  it("distinguishes runtime failures from unconfigured builds and can refresh", async () => {
    const client = fakeClient();
    client.getRuntimeInfo.mockRejectedValueOnce({
      code: "runtime-info-failed",
      message: "IPC unavailable",
      retryable: true,
    });
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));

    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("error"));
    expect(controller.result.current.runtimeInfo).toBeNull();
    expect(controller.result.current.runtimeError?.code).toBe("runtime-info-failed");
    expect(controller.result.current.message).toContain("无法读取更新配置");
    expect(controller.result.current.canCheck).toBe(false);

    await act(async () => {
      await controller.result.current.refreshRuntimeInfo();
    });

    expect(controller.result.current.runtimeStatus).toBe("ready");
    expect(controller.result.current.runtimeError).toBeNull();
    expect(controller.result.current.runtimeInfo?.configured).toBe(true);
    expect(controller.result.current.canCheck).toBe(true);
  });

  it("does not record an automatic attempt for an unconfigured build", async () => {
    const storage = memoryStorage();
    const client = fakeClient();
    client.getRuntimeInfo.mockResolvedValue({
      currentVersion: "0.2.0",
      channel: "stable",
      endpoint: "",
      configured: false,
    });
    const controller = renderHook(() => useAppUpdaterController({ client, storage }));

    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("unconfigured"));
    expect(client.check).not.toHaveBeenCalled();
    expect(storage.getItem(AUTO_UPDATE_CHECK_STORAGE_KEY)).toBeNull();
  });

  it("tracks download progress and returns safely to available after cancellation", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    const pendingDownload = deferred<void>();
    let onDownloadEvent: ((event: AppUpdateDownloadEvent) => void) | null = null;
    client.download.mockImplementation((listener) => {
      onDownloadEvent = listener;
      return pendingDownload.promise;
    });
    client.cancelDownload.mockResolvedValue(true);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeInfo).not.toBeNull());
    await act(async () => {
      await controller.result.current.checkManually();
    });

    let downloadPromise!: Promise<void>;
    act(() => {
      downloadPromise = controller.result.current.download();
    });
    await waitFor(() => expect(client.download).toHaveBeenCalledTimes(1));
    act(() => {
      onDownloadEvent?.({ event: "Started", data: { contentLength: 1_000 } });
      onDownloadEvent?.({ event: "Progress", data: { chunkLength: 250 } });
      onDownloadEvent?.({ event: "Verifying" });
    });
    expect(controller.result.current.progress).toEqual({
      downloadedBytes: 250,
      totalBytes: 1_000,
    });
    expect(controller.result.current.message).toBe("更新包下载完成，正在验证签名…");

    await act(async () => {
      await controller.result.current.cancelDownload();
    });
    expect(controller.result.current.phase).toBe("cancelling");
    pendingDownload.reject({
      code: "update-download-cancelled",
      message: "已取消",
      retryable: true,
    });
    await act(async () => {
      await downloadPromise;
    });
    expect(controller.result.current.phase).toBe("available");
    expect(controller.result.current.error).toBeNull();
  });

  it("keeps a completed download when a rejected cancellation arrives late", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    const pendingDownload = deferred<void>();
    const pendingCancellation = deferred<boolean>();
    client.download.mockReturnValue(pendingDownload.promise);
    client.cancelDownload.mockReturnValue(pendingCancellation.promise);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    let downloadPromise!: Promise<void>;
    act(() => {
      downloadPromise = controller.result.current.download();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("downloading"));
    let cancellationPromise!: Promise<void>;
    act(() => {
      cancellationPromise = controller.result.current.cancelDownload();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("cancelling"));

    await act(async () => {
      pendingDownload.resolve();
      await downloadPromise;
    });
    expect(controller.result.current.phase).toBe("downloaded");

    await act(async () => {
      pendingCancellation.resolve(false);
      await cancellationPromise;
    });
    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.error).toBeNull();
    expect(controller.result.current.canInstall).toBe(true);
  });

  it("ignores a late cancellation transport error after the download completes", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    const pendingDownload = deferred<void>();
    const pendingCancellation = deferred<boolean>();
    client.download.mockReturnValue(pendingDownload.promise);
    client.cancelDownload.mockReturnValue(pendingCancellation.promise);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    let downloadPromise!: Promise<void>;
    act(() => {
      downloadPromise = controller.result.current.download();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("downloading"));
    let cancellationPromise!: Promise<void>;
    act(() => {
      cancellationPromise = controller.result.current.cancelDownload();
    });

    await act(async () => {
      pendingDownload.resolve();
      await downloadPromise;
    });
    await act(async () => {
      pendingCancellation.reject(new Error("channel closed"));
      await cancellationPromise;
    });

    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.error).toBeNull();
  });

  it("keeps downloading after a cancellation request fails and clears that error on success", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    const pendingDownload = deferred<void>();
    client.download.mockReturnValue(pendingDownload.promise);
    client.cancelDownload.mockRejectedValue({
      code: "cancel-request-failed",
      message: "IPC failed",
      retryable: true,
    });
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    let downloadPromise!: Promise<void>;
    act(() => {
      downloadPromise = controller.result.current.download();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("downloading"));
    await act(async () => controller.result.current.cancelDownload());

    expect(controller.result.current.phase).toBe("downloading");
    expect(controller.result.current.error?.code).toBe("cancel-request-failed");
    expect(controller.result.current.canCancelDownload).toBe(true);

    await act(async () => {
      pendingDownload.resolve();
      await downloadPromise;
    });
    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.error).toBeNull();
  });

  it("allows retryable download failures to retry without another update check", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.download
      .mockRejectedValueOnce({
        code: "update-network-error",
        message: "offline",
        retryable: true,
      })
      .mockResolvedValueOnce(undefined);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    await act(async () => controller.result.current.download());
    expect(controller.result.current.phase).toBe("available");
    expect(controller.result.current.error?.code).toBe("update-network-error");
    expect(controller.result.current.failedAction).toBe("download");
    expect(controller.result.current.canDownload).toBe(true);

    await act(async () => controller.result.current.download());
    expect(client.download).toHaveBeenCalledTimes(2);
    expect(client.check).toHaveBeenCalledTimes(1);
    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.error).toBeNull();
  });

  it("keeps non-retryable download failures terminal", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.download.mockRejectedValue({
      code: "update-signature-invalid",
      message: "bad signature",
      retryable: false,
    });
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    await act(async () => controller.result.current.download());
    expect(controller.result.current.phase).toBe("error");
    expect(controller.result.current.error?.retryable).toBe(false);
    expect(controller.result.current.canDownload).toBe(false);
    expect(controller.result.current.canCheck).toBe(false);
    expect(controller.result.current.canDiscard).toBe(true);

    client.check.mockClear();
    await act(async () => controller.result.current.checkManually());
    expect(client.check).not.toHaveBeenCalled();
    expect(controller.result.current.phase).toBe("error");
    expect(controller.result.current.update).toEqual(updateMetadata());
  });

  it("discards a downloaded update, clears local state, and allows another check", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.download.mockImplementation(async (listener) => {
      listener({ event: "Started", data: { contentLength: 1_000 } });
      listener({ event: "Progress", data: { chunkLength: 800 } });
    });
    client.discard.mockResolvedValue(true);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());
    await act(async () => controller.result.current.download());

    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.progress.downloadedBytes).toBe(800);
    expect(controller.result.current.canDiscard).toBe(true);

    await act(async () => controller.result.current.discardUpdate());

    expect(client.discard).toHaveBeenCalledTimes(1);
    expect(controller.result.current.phase).toBe("idle");
    expect(controller.result.current.update).toBeNull();
    expect(controller.result.current.progress).toEqual({
      downloadedBytes: 0,
      totalBytes: null,
    });
    expect(controller.result.current.error).toBeNull();
    expect(controller.result.current.canCheck).toBe(true);
    expect(controller.result.current.message).toContain("可以重新检查更新");
  });

  it("blocks concurrent updater actions while discarding", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    const pendingDiscard = deferred<boolean>();
    client.discard.mockReturnValue(pendingDiscard.promise);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());
    client.check.mockClear();

    let discardPromise!: Promise<void>;
    act(() => {
      discardPromise = controller.result.current.discardUpdate();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("discarding"));
    expect(controller.result.current.canDiscard).toBe(false);
    expect(controller.result.current.canCheck).toBe(false);

    await act(async () => {
      await controller.result.current.checkManually();
      await controller.result.current.download();
      await controller.result.current.discardUpdate();
    });
    expect(client.check).not.toHaveBeenCalled();
    expect(client.download).not.toHaveBeenCalled();
    expect(client.discard).toHaveBeenCalledTimes(1);

    await act(async () => {
      pendingDiscard.resolve(true);
      await discardPromise;
    });
    expect(controller.result.current.phase).toBe("idle");
  });

  it("preserves a failed update session and its phase when discard is rejected", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.download.mockRejectedValue({
      code: "update-signature-invalid",
      message: "bad signature",
      retryable: false,
    });
    client.discard.mockResolvedValue(false);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());
    await act(async () => controller.result.current.download());
    const pendingUpdate = controller.result.current.update;

    await act(async () => controller.result.current.discardUpdate());

    expect(controller.result.current.phase).toBe("error");
    expect(controller.result.current.update).toBe(pendingUpdate);
    expect(controller.result.current.error?.code).toBe("update-discard-rejected");
    expect(controller.result.current.failedAction).toBe("discard");
    expect(controller.result.current.message).toContain("放弃此更新失败");
    expect(controller.result.current.canDiscard).toBe(true);
  });

  it("keeps an available session usable when discard is rejected", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.discard.mockResolvedValue(false);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());

    await act(async () => controller.result.current.discardUpdate());

    expect(controller.result.current.phase).toBe("available");
    expect(controller.result.current.update).toEqual(updateMetadata());
    expect(controller.result.current.failedAction).toBe("discard");
    expect(controller.result.current.canDownload).toBe(true);
    expect(controller.result.current.canDiscard).toBe(true);
    expect(controller.result.current.canCheck).toBe(false);
  });

  it("refuses checks while an update is available, downloaded, installing, or restarting", async () => {
    const client = fakeClient();
    client.check.mockResolvedValue(updateMetadata());
    client.download.mockResolvedValue(undefined);
    const pendingInstall = deferred<void>();
    client.install.mockReturnValue(pendingInstall.promise);
    const controller = renderHook(() => useAppUpdaterController({
      client,
      automaticCheck: false,
    }));
    await waitFor(() => expect(controller.result.current.runtimeStatus).toBe("ready"));
    await act(async () => controller.result.current.checkManually());
    client.check.mockClear();

    expect(controller.result.current.phase).toBe("available");
    expect(controller.result.current.canCheck).toBe(false);
    await act(async () => controller.result.current.checkManually());
    expect(controller.result.current.phase).toBe("available");
    expect(client.check).not.toHaveBeenCalled();

    await act(async () => controller.result.current.download());
    await act(async () => controller.result.current.checkManually());
    expect(controller.result.current.phase).toBe("downloaded");
    expect(controller.result.current.canCheck).toBe(false);
    expect(client.check).not.toHaveBeenCalled();

    let installPromise!: Promise<void>;
    act(() => {
      installPromise = controller.result.current.installAndRestart();
    });
    await waitFor(() => expect(controller.result.current.phase).toBe("installing"));
    await act(async () => controller.result.current.checkManually());
    expect(client.check).not.toHaveBeenCalled();

    await act(async () => {
      pendingInstall.resolve();
      await installPromise;
    });
    expect(controller.result.current.phase).toBe("restarting");
    await act(async () => controller.result.current.checkManually());
    expect(client.check).not.toHaveBeenCalled();
  });

  it("rejects future or stale timestamps without suppressing checks indefinitely", () => {
    const now = 10 * AUTO_UPDATE_CHECK_INTERVAL_MS;
    expect(isAutomaticUpdateCheckDue(null, now)).toBe(true);
    expect(isAutomaticUpdateCheckDue(now - 1_000, now)).toBe(false);
    expect(isAutomaticUpdateCheckDue(now - AUTO_UPDATE_CHECK_INTERVAL_MS, now)).toBe(true);
    expect(isAutomaticUpdateCheckDue(now + 10 * 60 * 1_000, now)).toBe(true);
  });
});

describe("normalizeAppUpdateError", () => {
  it("uses a readable fallback for null and opaque objects while preserving partial fields", () => {
    expect(normalizeAppUpdateError(null).message).toBe("未知更新错误");
    expect(normalizeAppUpdateError({}).message).toBe("未知更新错误");

    const partial = normalizeAppUpdateError({
      code: "updater-busy",
      retryable: false,
    });
    expect(partial.message).toBe("未知更新错误");
    expect(partial.code).toBe("updater-busy");
    expect(partial.retryable).toBe(false);

    expect(normalizeAppUpdateError({ message: "offline" }).message).toBe("offline");
  });
});

function fakeClient() {
  return {
    getRuntimeInfo: vi.fn(async () => ({
      currentVersion: "0.2.0",
      channel: "stable" as const,
      endpoint: "https://example.invalid/latest.json",
      configured: true,
    })),
    check: vi.fn<AppUpdaterClient["check"]>(),
    download: vi.fn<AppUpdaterClient["download"]>(),
    cancelDownload: vi.fn<AppUpdaterClient["cancelDownload"]>(),
    discard: vi.fn<AppUpdaterClient["discard"]>(),
    install: vi.fn<AppUpdaterClient["install"]>(),
  };
}

function updateMetadata() {
  return {
    currentVersion: "0.2.0",
    version: "0.2.1",
    notes: "安全更新",
    publishedAt: "2026-08-08T00:00:00Z",
  };
}

function memoryStorage(): Pick<Storage, "getItem" | "setItem"> {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
