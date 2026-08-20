import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppPreferencesController } from "../../src/hooks/useAppPreferences";
import type { AppUpdaterController } from "../../src/hooks/useAppUpdaterController";
import {
  DEFAULT_APP_PREFERENCES,
  type AppPreferencesV1,
} from "../../src/lib/appPreferences";
import { SettingsWorkspace } from "../../src/screens/SettingsWorkspace";
import type { SourceDir } from "../../src/types";

describe("SettingsWorkspace", () => {
  it("renders six keyboard-accessible categories and focuses the selected heading", async () => {
    const user = userEvent.setup();
    renderSettings();

    const navigation = screen.getByRole("navigation", { name: "设置分类" });
    expect(within(navigation).getAllByRole("button")).toHaveLength(6);
    expect(screen.getByRole("button", { name: /常规/ })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("heading", { name: "常规", level: 2 })).toBeVisible();

    const privacyButton = screen.getByRole("button", { name: /数据与隐私/ });
    privacyButton.focus();
    await user.keyboard("{Enter}");

    expect(privacyButton).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("heading", { name: "数据与隐私", level: 2 })).toHaveFocus();
    expect(screen.queryByRole("heading", { name: "常规", level: 2 })).not.toBeInTheDocument();
  });

  it("updates startup, automatic scan, and motion preferences from the general section", async () => {
    const user = userEvent.setup();
    const preferences = preferenceController();
    renderSettings({ preferences });

    await user.click(screen.getByRole("combobox", { name: "启动后打开" }));
    await user.click(screen.getByRole("option", { name: "快速挑片" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ startupDestination: "review" });

    const startupScan = screen.getByRole("switch", { name: "启动时自动扫描" });
    expect(startupScan).not.toBeChecked();
    await user.click(startupScan);
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ scanOnStartup: true });

    await user.click(screen.getByRole("combobox", { name: "动态效果" }));
    await user.click(screen.getByRole("option", { name: "始终减少动效" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ motionMode: "reduced" });
  });

  it("keeps library view and sort controls connected to preferences", async () => {
    const user = userEvent.setup();
    const preferences = preferenceController();
    renderSettings({ preferences });
    await user.click(screen.getByRole("button", { name: /素材库/ }));

    const viewMode = screen.getByRole("radiogroup", { name: "默认视图" });
    expect(within(viewMode).getByRole("radio", { name: "网格" })).toBeChecked();
    await user.click(within(viewMode).getByRole("radio", { name: "列表" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ libraryViewMode: "list" });

    await user.click(screen.getByRole("combobox", { name: "默认排序" }));
    await user.click(screen.getByRole("option", { name: "文件从大到小" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ librarySort: "size-desc" });
  });

  it("updates volume, mute, and quick-review autoplay preferences", async () => {
    const user = userEvent.setup();
    const preferences = preferenceController();
    renderSettings({ preferences });
    await user.click(screen.getByRole("button", { name: /播放/ }));

    fireEvent.change(screen.getByRole("slider", { name: "预览音量" }), {
      target: { value: "42" },
    });
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ previewVolumePercent: 42 });

    await user.click(screen.getByRole("switch", { name: "记住静音状态" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ previewMuted: true });

    await user.click(screen.getByRole("switch", { name: "快速挑片自动播放" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ reviewAutoplay: false });
  });

  it("requires confirmation before resetting only the preference contract", async () => {
    const user = userEvent.setup();
    const preferences = preferenceController();
    renderSettings({ preferences });

    await user.click(screen.getByRole("button", { name: "恢复默认设置" }));
    expect(preferences.resetPreferences).not.toHaveBeenCalled();
    expect(screen.getByText("恢复默认设置？")).toBeVisible();
    expect(screen.getByText(/启动扫描、素材库视图与排序/)).toBeVisible();
    expect(screen.getByText(/来源、标签、索引、缓存、视频和更新检查记录不会改变/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "确认恢复" }));
    expect(preferences.resetPreferences).toHaveBeenCalledTimes(1);
  });

  it("announces preference persistence failures without hiding the active controls", () => {
    renderSettings({
      preferences: preferenceController({ storageError: "本地存储当前不可用。" }),
    });

    expect(screen.getByRole("alert")).toHaveTextContent("无法保存设置");
    expect(screen.getByRole("alert")).toHaveTextContent("本地存储当前不可用");
    expect(screen.getByRole("combobox", { name: "启动后打开" })).toBeEnabled();
  });

  it("summarizes source health and opens source management", async () => {
    const user = userEvent.setup();
    const onOpenScan = vi.fn();
    renderSettings({
      onOpenScan,
      sourceDirs: [
        source("正常来源", { enabled: true }),
        source("离线来源", { accessibility: false, status: "unavailable" }),
        source("扫描失败", { enabled: true, lastError: "读取失败" }),
      ],
    });
    await user.click(screen.getByRole("button", { name: /数据与隐私/ }));

    const summary = screen.getByLabelText("扫描来源摘要");
    expect(summary).toHaveTextContent(/来源总数\s*3/);
    expect(summary).toHaveTextContent(/已启用\s*2/);
    expect(summary).toHaveTextContent(/异常来源\s*2/);
    expect(screen.getByText(/本机 SQLite 数据库/)).toBeVisible();
    expect(screen.getByText(/不提供云同步或遥测/)).toBeVisible();
    expect(screen.getByText(/只有从应用回收站再次确认“永久删除视频”后/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: /管理扫描来源/ }));
    expect(onOpenScan).toHaveBeenCalledTimes(1);
  });

  it("shows truthful about and licensing information without links or obsolete migration copy", async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(screen.getByRole("button", { name: /关于瓦刻/ }));

    expect(screen.getByText("瓦刻 · VALOFRAME")).toBeVisible();
    expect(screen.getByText("Windows x64")).toBeVisible();
    expect(screen.getByText("Stable")).toBeVisible();
    expect(screen.getByText(/项目自有源代码和随附文档采用 MIT License/)).toBeVisible();
    expect(screen.getByText(/与 Riot Games、腾讯及其关联公司不存在隶属/)).toBeVisible();
    expect(screen.queryByText(/v0\.1\.0-beta\.1 不含稳定更新器/)).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("opens updates by default for an actionable update and confirms downloading", async () => {
    const user = userEvent.setup();
    const updater = controller({ phase: "available" });
    renderSettings({ updater });

    expect(screen.getByRole("heading", { name: "更新", level: 2 })).toBeVisible();
    expect(screen.getByText("v0.2.2")).toBeVisible();
    expect(screen.getByText("安全与稳定性改进")).toBeVisible();
    expect(screen.getByText("高级技术信息")).toBeVisible();
    expect(screen.queryByText(/v0\.1\.0-beta\.1 不含稳定更新器/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "下载更新" }));
    expect(updater.download).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认下载" }));
    expect(updater.download).toHaveBeenCalledTimes(1);
  });

  it("updates automatic checking independently from the manual check action", async () => {
    const user = userEvent.setup();
    const preferences = preferenceController();
    const updater = controller({ update: null });
    renderSettings({ preferences, updater });
    await user.click(screen.getByRole("button", { name: /更新/ }));

    await user.click(screen.getByRole("switch", { name: "自动检查更新" }));
    expect(preferences.updatePreferences).toHaveBeenCalledWith({ automaticUpdateCheck: false });
    expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
  });

  it("exposes progress and a real cancel action while downloading", async () => {
    const user = userEvent.setup();
    const updater = controller({
      phase: "downloading",
      progress: { downloadedBytes: 5 * 1_024 * 1_024, totalBytes: 10 * 1_024 * 1_024 },
    });
    renderSettings({ updater });

    expect(screen.getByRole("progressbar", { name: "更新下载进度" })).toHaveValue(5 * 1_024 * 1_024);
    expect(screen.getByText("50%")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "取消下载" }));
    expect(updater.cancelDownload).toHaveBeenCalledTimes(1);
  });

  it("blocks installation while a critical task is active", () => {
    const updater = controller({ phase: "downloaded" });
    renderSettings({
      updater,
      criticalTaskMessage: "扫描任务正在运行，请等待扫描结束后再安装",
    });

    expect(screen.getByRole("button", { name: "安装并重启" })).toBeDisabled();
    expect(screen.getByText(/扫描任务正在运行/)).toBeVisible();
  });

  it("requires a second explicit confirmation before installation and restart", async () => {
    const user = userEvent.setup();
    const updater = controller({ phase: "downloaded" });
    renderSettings({ updater });

    await user.click(screen.getByRole("button", { name: "安装并重启" }));
    expect(updater.installAndRestart).not.toHaveBeenCalled();
    expect(screen.getByText("安装更新并重启？")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "安装并重启" }));
    expect(updater.installAndRestart).toHaveBeenCalledTimes(1);
  });

  it("disables an open install confirmation if a critical task starts", async () => {
    const user = userEvent.setup();
    const updater = controller({ phase: "downloaded" });
    const preferences = preferenceController();
    const view = render(
      <SettingsWorkspace
        criticalTaskMessage={null}
        onOpenScan={vi.fn()}
        preferences={preferences}
        sourceDirs={[]}
        updater={updater}
      />,
    );

    await user.click(screen.getByRole("button", { name: "安装并重启" }));
    view.rerender(
      <SettingsWorkspace
        criticalTaskMessage="视频导出任务正在运行"
        onOpenScan={vi.fn()}
        preferences={preferences}
        sourceDirs={[]}
        updater={updater}
      />,
    );

    expect(screen.getByRole("button", { name: "安装并重启" })).toBeDisabled();
    expect(screen.getByText(/请等待任务结束后再安装更新/)).toBeVisible();
    expect(updater.installAndRestart).not.toHaveBeenCalled();
  });

  it("keeps check disabled after a verified package has downloaded", () => {
    const updater = controller({ phase: "downloaded" });
    renderSettings({ updater });

    expect(screen.getByRole("button", { name: "检查更新" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "安装并重启" })).toBeEnabled();
  });

  it("can reread runtime configuration after a transient IPC failure", async () => {
    const user = userEvent.setup();
    const updater = controller({
      update: null,
      runtimeInfo: null,
      runtimeStatus: "error",
      runtimeError: {
        code: "updater-runtime-info-failed",
        message: "读取更新配置失败",
        retryable: true,
      },
    });
    renderSettings({ updater });
    await user.click(screen.getByRole("button", { name: /更新/ }));

    expect(screen.getByText("更新配置读取失败")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "重新读取配置" }));
    expect(updater.refreshRuntimeInfo).toHaveBeenCalledTimes(1);
  });

  it("offers a direct retry after a retryable download error", () => {
    const updater = controller({
      phase: "available",
      error: {
        code: "update-download-failed",
        message: "下载中断",
        retryable: true,
      },
      failedAction: "download",
    });
    renderSettings({ updater });

    expect(screen.getByRole("button", { name: "重试下载" })).toBeEnabled();
  });

  it("does not label a normal download as a retry after discard fails", () => {
    const updater = controller({
      phase: "available",
      error: {
        code: "update-discard-rejected",
        message: "未能放弃此更新，请重试",
        retryable: true,
      },
      failedAction: "discard",
    });
    renderSettings({ updater });

    expect(screen.getByRole("button", { name: "下载更新" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "重试下载" })).not.toBeInTheDocument();
  });

  it("discards an available update directly", async () => {
    const user = userEvent.setup();
    const updater = controller({ phase: "available" });
    renderSettings({ updater });

    await user.click(screen.getByRole("button", { name: "放弃此更新" }));

    expect(updater.discardUpdate).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("放弃此更新？")).not.toBeInTheDocument();
  });

  it.each(["downloaded", "error"] as const)(
    "requires confirmation before discarding a pending update in the %s phase",
    async (phase) => {
      const user = userEvent.setup();
      const updater = controller({ phase });
      renderSettings({ updater });

      await user.click(screen.getByRole("button", { name: "放弃此更新" }));
      expect(updater.discardUpdate).not.toHaveBeenCalled();
      expect(screen.getByText("放弃此更新？")).toBeVisible();
      expect(screen.getByText(/当前版本不受影响/)).toBeVisible();

      await user.click(screen.getByRole("button", { name: "确认放弃" }));
      expect(updater.discardUpdate).toHaveBeenCalledTimes(1);
    },
  );

  it.each(["downloading", "installing"] as const)(
    "does not allow discard while the updater is %s",
    (phase) => {
      const updater = controller({ phase });
      renderSettings({ updater });

      expect(screen.queryByRole("button", { name: "放弃此更新" })).not.toBeInTheDocument();
    },
  );
});

type RenderSettingsOptions = {
  preferences?: AppPreferencesController;
  updater?: AppUpdaterController;
  criticalTaskMessage?: string | null;
  sourceDirs?: SourceDir[];
  onOpenScan?: () => void;
};

function renderSettings(options: RenderSettingsOptions = {}) {
  return render(
    <SettingsWorkspace
      criticalTaskMessage={options.criticalTaskMessage ?? null}
      onOpenScan={options.onOpenScan ?? vi.fn()}
      preferences={options.preferences ?? preferenceController()}
      sourceDirs={options.sourceDirs ?? []}
      updater={options.updater ?? controller({ update: null })}
    />,
  );
}

function preferenceController(
  overrides: Partial<Omit<AppPreferencesController, "preferences">> & {
    preferences?: Partial<AppPreferencesV1>;
  } = {},
): AppPreferencesController {
  const { preferences: preferenceOverrides, ...controllerOverrides } = overrides;
  return {
    preferences: {
      ...DEFAULT_APP_PREFERENCES,
      ...preferenceOverrides,
    },
    storageError: null,
    updatePreferences: vi.fn(),
    resetPreferences: vi.fn(),
    ...controllerOverrides,
  };
}

function controller(overrides: Partial<AppUpdaterController>): AppUpdaterController {
  const phase = overrides.phase ?? "idle";
  const updatePhases: AppUpdaterController["phase"][] = [
    "available",
    "downloading",
    "cancelling",
    "downloaded",
    "discarding",
    "installing",
    "restarting",
    "error",
  ];
  return {
    runtimeInfo: {
      currentVersion: "0.2.1",
      channel: "stable",
      endpoint: "https://github.com/example/app/releases/latest/download/latest.json",
      configured: true,
    },
    runtimeStatus: "ready",
    runtimeError: null,
    phase,
    update: updatePhases.includes(phase) ? {
      currentVersion: "0.2.1",
      version: "0.2.2",
      notes: "安全与稳定性改进",
      publishedAt: "2026-08-08T00:00:00Z",
    } : null,
    progress: { downloadedBytes: 0, totalBytes: null },
    message: "更新状态",
    error: null,
    failedAction: null,
    canCheck: ![
      "available",
      "checking",
      "downloading",
      "cancelling",
      "downloaded",
      "discarding",
      "installing",
      "restarting",
    ].includes(phase),
    canDownload: phase === "available",
    canCancelDownload: phase === "downloading",
    canDiscard: ["available", "downloaded", "error"].includes(phase),
    canInstall: phase === "downloaded",
    refreshRuntimeInfo: vi.fn(async () => undefined),
    checkManually: vi.fn(async () => undefined),
    download: vi.fn(async () => undefined),
    cancelDownload: vi.fn(async () => undefined),
    discardUpdate: vi.fn(async () => undefined),
    installAndRestart: vi.fn(async () => undefined),
    ...overrides,
  };
}

function source(name: string, overrides: Partial<SourceDir> = {}): SourceDir {
  return {
    id: name,
    name,
    displayName: name,
    path: `D:\\Clips\\${name}`,
    sourceKind: "generic",
    scanMode: "recursive-mp4",
    scanRootPath: `D:\\Clips\\${name}`,
    enabled: false,
    status: "available",
    accessibility: true,
    lastError: null,
    clipCount: 0,
    lastScanAt: null,
    ...overrides,
  };
}
