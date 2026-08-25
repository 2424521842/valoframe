import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  CheckCircle,
  CloudArrowDown,
  Database,
  FolderOpen,
  Gear,
  Info,
  PlayCircle,
  Power,
  ShieldCheck,
  SpeakerHigh,
  SquaresFour,
  WarningCircle,
  X,
  type Icon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import {
  UiAlertDialog,
  UiAlertDialogAction,
  UiAlertDialogCancel,
  UiAlertDialogContent,
  UiAlertDialogDescription,
  UiAlertDialogTitle,
} from "../components/ui/alert-dialog";
import {
  UiSelect,
  UiSelectContent,
  UiSelectItem,
  UiSelectTrigger,
  UiSelectValue,
} from "../components/ui/select";
import { UiSwitch } from "../components/ui/switch";
import type { AppPreferencesController } from "../hooks/useAppPreferences";
import type { AppUpdaterController } from "../hooks/useAppUpdaterController";
import type { SourceDir } from "../types";

type SettingsWorkspaceProps = {
  preferences: AppPreferencesController;
  updater: AppUpdaterController;
  criticalTaskMessage: string | null;
  sourceDirs: SourceDir[];
  onOpenScan: () => void;
};

type SettingsSection = "general" | "library" | "playback" | "updates" | "privacy" | "about";
type Confirmation = "reset" | "download" | "discard" | "install" | null;

type SettingsSectionDefinition = {
  id: SettingsSection;
  label: string;
  description: string;
  icon: Icon;
};

const SETTINGS_SECTIONS: SettingsSectionDefinition[] = [
  { id: "general", label: "常规", description: "启动与界面体验", icon: Gear },
  { id: "library", label: "素材库", description: "视图与排序方式", icon: SquaresFour },
  { id: "playback", label: "播放", description: "预览与快速挑片", icon: PlayCircle },
  { id: "updates", label: "更新", description: "稳定通道与安装", icon: CloudArrowDown },
  { id: "privacy", label: "数据与隐私", description: "本地数据边界", icon: ShieldCheck },
  { id: "about", label: "关于瓦刻", description: "版本与许可范围", icon: Info },
];

const STARTUP_DESTINATIONS = [
  { value: "library-all", label: "全部素材" },
  { value: "library-today", label: "最近添加" },
  { value: "library-favorites", label: "收藏" },
  { value: "review", label: "快速挑片" },
  { value: "scan", label: "扫描目录" },
] as const;

const LIBRARY_SORT_OPTIONS = [
  { value: "modified-desc", label: "最近修改优先" },
  { value: "modified-asc", label: "最早修改优先" },
  { value: "size-desc", label: "文件从大到小" },
  { value: "size-asc", label: "文件从小到大" },
  { value: "name-asc", label: "文件名 A–Z" },
] as const;

export function SettingsWorkspace({
  preferences,
  updater,
  criticalTaskMessage,
  sourceDirs,
  onOpenScan,
}: SettingsWorkspaceProps) {
  const [activeSection, setActiveSection] = useState<SettingsSection>(() => (
    hasActionableUpdate(updater) ? "updates" : "general"
  ));
  const [confirmation, setConfirmation] = useState<Confirmation>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const sectionHeadingRef = useRef<HTMLHeadingElement>(null);
  const previousSectionRef = useRef(activeSection);
  const values = preferences.preferences;
  const abnormalSourceCount = useMemo(
    () => sourceDirs.filter((source) => (
      !source.accessibility
      || Boolean(source.lastError)
      || source.status === "unavailable"
      || source.status === "error"
    )).length,
    [sourceDirs],
  );

  useEffect(() => {
    if (previousSectionRef.current === activeSection) {
      return;
    }
    previousSectionRef.current = activeSection;
    const workspace = workspaceRef.current;
    if (workspace) {
      if (typeof workspace.scrollTo === "function") {
        workspace.scrollTo({ top: 0, behavior: "auto" });
      } else {
        workspace.scrollTop = 0;
      }
    }
    sectionHeadingRef.current?.focus();
  }, [activeSection]);

  const changeSection = (section: SettingsSection) => {
    setActiveSection(section);
  };

  const installConfirmationBlocked = confirmation === "install" && Boolean(criticalTaskMessage);
  const discardConfirmationBlocked = confirmation === "discard" && !updater.canDiscard;
  const confirmationBlocked = installConfirmationBlocked || discardConfirmationBlocked;

  const confirmAction = () => {
    const action = confirmation;
    setConfirmation(null);
    if (action === "reset") {
      preferences.resetPreferences();
    } else if (action === "download") {
      void updater.download();
    } else if (action === "discard") {
      void updater.discardUpdate();
    } else if (action === "install") {
      void updater.installAndRestart();
    }
  };

  return (
    <section className="settings-workspace" aria-labelledby="settings-heading">
      <header className="settings-header">
        <div>
          <h1 id="settings-heading">设置</h1>
          <p>管理瓦刻在这台电脑上的启动、浏览、播放与更新方式。</p>
        </div>
        <div
          className="settings-version-badge"
          aria-label={`当前应用版本 v${updater.runtimeInfo?.currentVersion ?? "未知"}，Stable 稳定通道`}
        >
          <span>当前版本</span>
          <strong>v{updater.runtimeInfo?.currentVersion ?? "—"}</strong>
          <small>Windows x64 · Stable</small>
        </div>
      </header>

      <div className="settings-layout">
        <nav className="settings-nav" aria-label="设置分类">
          {SETTINGS_SECTIONS.map((section) => {
            const SectionIcon = section.icon;
            const isActive = activeSection === section.id;
            return (
              <button
                key={section.id}
                aria-current={isActive ? "page" : undefined}
                className="settings-nav-button"
                data-active={isActive || undefined}
                type="button"
                onClick={() => changeSection(section.id)}
              >
                <SectionIcon aria-hidden="true" weight={isActive ? "fill" : "regular"} />
                <span>
                  <strong>{section.label}</strong>
                  <small>{section.description}</small>
                </span>
                {section.id === "updates" && hasActionableUpdate(updater) ? (
                  <em aria-label="有待处理更新" />
                ) : null}
              </button>
            );
          })}
        </nav>

        <div className="settings-category-picker">
          <span id="settings-category-label">设置分类</span>
          <UiSelect value={activeSection} onValueChange={(value) => changeSection(value as SettingsSection)}>
            <UiSelectTrigger aria-labelledby="settings-category-label">
              <UiSelectValue />
            </UiSelectTrigger>
            <UiSelectContent>
              {SETTINGS_SECTIONS.map((section) => (
                <UiSelectItem key={section.id} value={section.id}>{section.label}</UiSelectItem>
              ))}
            </UiSelectContent>
          </UiSelect>
        </div>

        <main ref={workspaceRef} className="settings-content">
          {preferences.storageError ? (
            <div className="settings-save-alert" role="alert">
              <WarningCircle aria-hidden="true" />
              <div>
                <strong>无法保存设置</strong>
                <span>{preferences.storageError} 更改不会保留到下次启动。</span>
              </div>
            </div>
          ) : null}

          <section
            className="settings-section"
            aria-labelledby={`settings-section-${activeSection}`}
          >
            <SettingsSectionHeading
              activeSection={activeSection}
              headingRef={sectionHeadingRef}
            />

            {activeSection === "general" ? (
              <div className="settings-group">
                <SettingRow
                  description="下次启动瓦刻时直接进入所选工作区。"
                  title="启动后打开"
                  titleId="startup-destination-label"
                >
                  <UiSelect
                    value={values.startupDestination}
                    onValueChange={(value) => preferences.updatePreferences({
                      startupDestination: value as typeof values.startupDestination,
                    })}
                  >
                    <UiSelectTrigger aria-labelledby="startup-destination-label">
                      <UiSelectValue />
                    </UiSelectTrigger>
                    <UiSelectContent>
                      {STARTUP_DESTINATIONS.map((option) => (
                        <UiSelectItem key={option.value} value={option.value}>{option.label}</UiSelectItem>
                      ))}
                    </UiSelectContent>
                  </UiSelect>
                </SettingRow>

                <SettingRow
                  description="开启后，下次启动瓦刻会同步所有已加入自动同步的来源；短时间内重新启动不会重复扫描，仍可随时手动同步。"
                  title="启动时自动扫描"
                  titleId="scan-on-startup-label"
                >
                  <UiSwitch
                    aria-labelledby="scan-on-startup-label"
                    checked={values.scanOnStartup}
                    onCheckedChange={(checked) => preferences.updatePreferences({ scanOnStartup: checked })}
                  />
                </SettingRow>

                <SettingRow
                  description="“跟随系统”会尊重 Windows 的减少动态效果偏好。"
                  title="动态效果"
                  titleId="motion-mode-label"
                >
                  <UiSelect
                    value={values.motionMode}
                    onValueChange={(value) => preferences.updatePreferences({
                      motionMode: value as typeof values.motionMode,
                    })}
                  >
                    <UiSelectTrigger aria-labelledby="motion-mode-label">
                      <UiSelectValue />
                    </UiSelectTrigger>
                    <UiSelectContent>
                      <UiSelectItem value="system">跟随系统</UiSelectItem>
                      <UiSelectItem value="reduced">始终减少动效</UiSelectItem>
                    </UiSelectContent>
                  </UiSelect>
                </SettingRow>

                <SettingRow
                  description="开启后会在侧栏底部显示带「广告」标识的静态图文卡片。素材由瓦刻后端下载到本机后再显示，界面本身不会连接任何外部服务器；关闭后不再发起素材请求。"
                  title="显示广告"
                  titleId="ads-enabled-label"
                >
                  <UiSwitch
                    aria-labelledby="ads-enabled-label"
                    checked={values.adsEnabled}
                    onCheckedChange={(checked) => preferences.updatePreferences({ adsEnabled: checked })}
                  />
                </SettingRow>

                {values.adsEnabled ? (
                  <>
                    <SettingRow
                      description="广告方提供的素材清单接口，必须以 https:// 开头（本机联调可用 http://localhost）。留空则不显示广告。"
                      title="广告素材接口"
                      titleId="ad-endpoint-label"
                    >
                      <input
                        aria-labelledby="ad-endpoint-label"
                        className="settings-input"
                        placeholder="https://ad.example.com/manifest"
                        type="url"
                        value={values.adManifestEndpoint}
                        onChange={(event) => preferences.updatePreferences({
                          adManifestEndpoint: event.target.value,
                        })}
                      />
                    </SettingRow>

                    <SettingRow
                      description="广告方落地页域名，多个用逗号或空格分隔。只有列表内的域名（含其子域名）才允许打开；留空将阻止所有广告点击。"
                      title="落地页域名允许列表"
                      titleId="ad-hosts-label"
                    >
                      <input
                        aria-labelledby="ad-hosts-label"
                        className="settings-input"
                        placeholder="ad.example.com, lp.example.com"
                        type="text"
                        value={values.adAllowedHosts}
                        onChange={(event) => preferences.updatePreferences({
                          adAllowedHosts: event.target.value,
                        })}
                      />
                    </SettingRow>
                  </>
                ) : null}

                <SettingRow
                  description="只重置本页中的偏好，不会修改来源、标签、索引、缓存、视频或更新检查记录。"
                  title="恢复默认设置"
                >
                  <button
                    className="settings-button settings-button--secondary"
                    type="button"
                    onClick={() => setConfirmation("reset")}
                  >
                    <ArrowCounterClockwise aria-hidden="true" />
                    恢复默认设置
                  </button>
                </SettingRow>
              </div>
            ) : null}

            {activeSection === "library" ? (
              <div className="settings-group">
                <SettingRow
                  description="此设置与素材库工具栏保持同步。"
                  title="默认视图"
                  titleId="library-view-mode-label"
                >
                  <div className="settings-segmented" role="radiogroup" aria-labelledby="library-view-mode-label">
                    <label data-active={values.libraryViewMode === "grid" || undefined}>
                      <input
                        checked={values.libraryViewMode === "grid"}
                        name="library-view-mode"
                        type="radio"
                        value="grid"
                        onChange={() => preferences.updatePreferences({ libraryViewMode: "grid" })}
                      />
                      <SquaresFour aria-hidden="true" />
                      网格
                    </label>
                    <label data-active={values.libraryViewMode === "list" || undefined}>
                      <input
                        checked={values.libraryViewMode === "list"}
                        name="library-view-mode"
                        type="radio"
                        value="list"
                        onChange={() => preferences.updatePreferences({ libraryViewMode: "list" })}
                      />
                      <Database aria-hidden="true" />
                      列表
                    </label>
                  </div>
                </SettingRow>

                <SettingRow
                  description="此设置会成为素材库查询的默认排序。"
                  title="默认排序"
                  titleId="library-sort-label"
                >
                  <UiSelect
                    value={values.librarySort}
                    onValueChange={(value) => preferences.updatePreferences({
                      librarySort: value as typeof values.librarySort,
                    })}
                  >
                    <UiSelectTrigger aria-labelledby="library-sort-label">
                      <UiSelectValue />
                    </UiSelectTrigger>
                    <UiSelectContent>
                      {LIBRARY_SORT_OPTIONS.map((option) => (
                        <UiSelectItem key={option.value} value={option.value}>{option.label}</UiSelectItem>
                      ))}
                    </UiSelectContent>
                  </UiSelect>
                </SettingRow>
              </div>
            ) : null}

            {activeSection === "playback" ? (
              <div className="settings-group">
                <SettingRow
                  description="完整预览会记住此音量，并与播放器中的音量控件保持同步。"
                  title="预览音量"
                  titleId="preview-volume-label"
                >
                  <div className="settings-volume-control">
                    <SpeakerHigh aria-hidden="true" />
                    <input
                      aria-labelledby="preview-volume-label"
                      max="100"
                      min="0"
                      step="1"
                      type="range"
                      value={values.previewVolumePercent}
                      onChange={(event) => preferences.updatePreferences({
                        previewVolumePercent: Number(event.currentTarget.value),
                      })}
                    />
                    <output>{values.previewVolumePercent}%</output>
                  </div>
                </SettingRow>

                <SettingRow
                  description="开启后，完整预览默认静音；仍可随时在播放器中取消静音。"
                  title="记住静音状态"
                  titleId="preview-muted-label"
                >
                  <UiSwitch
                    aria-labelledby="preview-muted-label"
                    checked={values.previewMuted}
                    onCheckedChange={(checked) => preferences.updatePreferences({ previewMuted: checked })}
                  />
                </SettingRow>

                <SettingRow
                  description="快速挑片沿用预览音量和静音设置；若系统拦截有声自动播放，可点击播放后继续观看。"
                  title="快速挑片自动播放"
                  titleId="review-autoplay-label"
                >
                  <UiSwitch
                    aria-labelledby="review-autoplay-label"
                    checked={values.reviewAutoplay}
                    onCheckedChange={(checked) => preferences.updatePreferences({ reviewAutoplay: checked })}
                  />
                </SettingRow>
              </div>
            ) : null}

            {activeSection === "updates" ? (
              <UpdatesSection
                criticalTaskMessage={criticalTaskMessage}
                updater={updater}
                automaticUpdateCheck={values.automaticUpdateCheck}
                onAutomaticUpdateCheckChange={(checked) => preferences.updatePreferences({
                  automaticUpdateCheck: checked,
                })}
                onRequestConfirmation={setConfirmation}
              />
            ) : null}

            {activeSection === "privacy" ? (
              <div className="settings-privacy">
                <div className="settings-source-summary" aria-label="扫描来源摘要">
                  <div>
                    <span>来源总数</span>
                    <strong>{sourceDirs.length}</strong>
                  </div>
                  <div>
                    <span>已启用</span>
                    <strong>{sourceDirs.filter((source) => source.enabled).length}</strong>
                  </div>
                  <div data-tone={abnormalSourceCount > 0 ? "warning" : "normal"}>
                    <span>异常来源</span>
                    <strong>{abnormalSourceCount}</strong>
                  </div>
                </div>

                <button className="settings-manage-sources" type="button" onClick={onOpenScan}>
                  <FolderOpen aria-hidden="true" />
                  <span>
                    <strong>管理扫描来源</strong>
                    <small>查看目录状态、重新扫描或添加来源</small>
                  </span>
                </button>

                <div className="settings-privacy-copy">
                  <section>
                    <Database aria-hidden="true" />
                    <div>
                      <h3>数据保存在本机</h3>
                      <p>素材索引、收藏、标签和备注保存在本机 SQLite 数据库中；快速挑片进度单独保存在本机应用存储。瓦刻不提供云同步或遥测，也不会自动上传；只有在你主动提交“问题反馈”并按选择附带数据后，诊断数据才会离开本机。</p>
                    </div>
                  </section>
                  <section>
                    <ShieldCheck aria-hidden="true" />
                    <div>
                      <h3>原视频默认只读</h3>
                      <p>扫描、预览、整理和移入应用回收站不会修改原视频。只有从应用回收站再次确认“永久删除视频”后，瓦刻才会尝试删除经过身份校验的本地文件；仅移除索引不会触碰视频。</p>
                    </div>
                  </section>
                </div>
              </div>
            ) : null}

            {activeSection === "about" ? (
              <div className="settings-about">
                <div className="settings-about-brand">
                  <span className="settings-about-mark" aria-hidden="true">
                    <img alt="" src="/valoframe-mark.png" />
                  </span>
                  <div>
                    <h3>瓦刻 · VALOFRAME</h3>
                    <p>在本机整理、筛选和预览《无畏契约》高光素材。</p>
                  </div>
                </div>

                <dl className="settings-about-meta">
                  <div><dt>版本</dt><dd>v{updater.runtimeInfo?.currentVersion ?? "—"}</dd></div>
                  <div><dt>平台</dt><dd>Windows x64</dd></div>
                  <div><dt>更新通道</dt><dd>Stable</dd></div>
                </dl>

                <div className="settings-license-copy">
                  <section>
                    <h3>许可范围</h3>
                    <p>项目自有源代码和随附文档采用 MIT License。第三方依赖、FFmpeg、游戏内容、名称、商标、品牌图标及其他非项目自有素材遵循各自的许可与权利范围，不包含在本项目的 MIT 授权中。</p>
                  </section>
                  <section>
                    <h3>非官方社区项目</h3>
                    <p>瓦刻与 Riot Games、腾讯及其关联公司不存在隶属、赞助或认可关系。VALORANT、《无畏契约》及相关名称、商标和游戏内容归其各自权利人所有。</p>
                  </section>
                </div>
              </div>
            ) : null}
          </section>
        </main>
      </div>

      <UiAlertDialog open={confirmation !== null} onOpenChange={(open) => !open && setConfirmation(null)}>
        <UiAlertDialogContent>
          <UiAlertDialogTitle>{confirmationTitle(confirmation)}</UiAlertDialogTitle>
          <UiAlertDialogDescription>
            {confirmationDescription(confirmation, updater, criticalTaskMessage)}
          </UiAlertDialogDescription>
          <div className="ui-alert-dialog-actions">
            <UiAlertDialogCancel>取消</UiAlertDialogCancel>
            <UiAlertDialogAction disabled={confirmationBlocked} onClick={confirmAction}>
              {confirmationActionLabel(confirmation)}
            </UiAlertDialogAction>
          </div>
        </UiAlertDialogContent>
      </UiAlertDialog>
    </section>
  );
}

function SettingsSectionHeading({
  activeSection,
  headingRef,
}: {
  activeSection: SettingsSection;
  headingRef: RefObject<HTMLHeadingElement | null>;
}) {
  const section = SETTINGS_SECTIONS.find((candidate) => candidate.id === activeSection)
    ?? SETTINGS_SECTIONS[0];
  return (
    <header className="settings-section-heading">
      <h2 ref={headingRef} id={`settings-section-${activeSection}`} tabIndex={-1}>{section.label}</h2>
      <p>{section.description}</p>
    </header>
  );
}

function SettingRow({
  title,
  description,
  titleId,
  children,
}: {
  title: string;
  description: string;
  titleId?: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div className="settings-row-copy">
        <strong id={titleId}>{title}</strong>
        <span>{description}</span>
      </div>
      <div className="settings-row-control">{children}</div>
    </div>
  );
}

function UpdatesSection({
  updater,
  criticalTaskMessage,
  automaticUpdateCheck,
  onAutomaticUpdateCheckChange,
  onRequestConfirmation,
}: {
  updater: AppUpdaterController;
  criticalTaskMessage: string | null;
  automaticUpdateCheck: boolean;
  onAutomaticUpdateCheckChange: (checked: boolean) => void;
  onRequestConfirmation: (confirmation: Confirmation) => void;
}) {
  const isChecking = updater.phase === "checking";
  const isDownloading = updater.phase === "downloading" || updater.phase === "cancelling";
  const isDiscarding = updater.phase === "discarding";
  const isRuntimeLoading = updater.runtimeStatus === "loading";
  const isRuntimeError = updater.runtimeStatus === "error";
  const configured = updater.runtimeStatus === "ready" && updater.runtimeInfo?.configured === true;
  const totalBytes = updater.progress.totalBytes && updater.progress.totalBytes > 0
    ? updater.progress.totalBytes
    : null;
  const progressPercent = totalBytes
    ? Math.min(100, (updater.progress.downloadedBytes / totalBytes) * 100)
    : null;
  const runtimeLabel = updater.runtimeStatus === "loading"
    ? "读取中"
    : updater.runtimeStatus === "error"
      ? "读取失败"
      : configured
        ? "已配置"
        : "未配置";
  const canRefreshRuntime = isRuntimeError
    && !isDownloading
    && !isDiscarding
    && updater.phase !== "installing"
    && updater.phase !== "restarting";
  const statusTone = updater.error || updater.runtimeError ? "error" : updater.phase;

  const requestDiscard = () => {
    if (!updater.canDiscard) return;
    if (updater.phase === "downloaded" || updater.phase === "error") {
      onRequestConfirmation("discard");
      return;
    }
    void updater.discardUpdate();
  };

  return (
    <div className="settings-updates">
      <div className="settings-group">
        <SettingRow
          description="每天最多自动检查一次，并在窗口重新获得焦点时补查；手动检查始终可用。"
          title="自动检查更新"
          titleId="automatic-update-check-label"
        >
          <UiSwitch
            aria-labelledby="automatic-update-check-label"
            checked={automaticUpdateCheck}
            onCheckedChange={onAutomaticUpdateCheckChange}
          />
        </SettingRow>
      </div>

      <div className="about-content settings-update-content">
        <article className="about-card about-card--updater">
          <div className="about-card-title">
            <span><ShieldCheck aria-hidden="true" />应用内更新</span>
            <em className={configured ? "about-channel about-channel--ready" : "about-channel"}>
              {runtimeLabel}
            </em>
          </div>
          <p className="about-card-copy">
            只有经过 Tauri 更新签名验证的更高稳定版本才可安装。手动检查不受每日限频影响。
          </p>

          <div
            aria-live={updater.error || updater.runtimeError ? undefined : "polite"}
            className={`about-update-status about-update-status--${statusTone}`}
            role={updater.error || updater.runtimeError ? "alert" : "status"}
          >
            <StatusIcon
              phase={updater.phase}
              hasError={Boolean(updater.error || updater.runtimeError)}
            />
            <div>
              <strong>{statusTitle(updater)}</strong>
              <span>{updater.message}</span>
            </div>
          </div>

          {updater.update ? (
            <div className="about-release">
              <div>
                <span>可用版本</span>
                <strong>v{updater.update.version}</strong>
                {updater.update.publishedAt ? (
                  <small>{formatPublishedAt(updater.update.publishedAt)}</small>
                ) : null}
              </div>
              <section aria-label="发布说明">
                <h3>发布说明</h3>
                <p>{updater.update.notes || "暂无发布说明"}</p>
              </section>
            </div>
          ) : null}

          {isDownloading ? (
            <div className="about-download-progress">
              <div>
                <span>{updater.phase === "cancelling" ? "正在取消" : "正在下载"}</span>
                <strong>{progressLabel(updater.progress.downloadedBytes, updater.progress.totalBytes)}</strong>
              </div>
              <progress
                aria-label="更新下载进度"
                max={totalBytes ?? 1}
                value={totalBytes ? Math.min(updater.progress.downloadedBytes, totalBytes) : undefined}
              />
              <small>{progressPercent === null ? "正在等待服务器提供文件大小" : `${progressPercent.toFixed(0)}%`}</small>
            </div>
          ) : null}

          {criticalTaskMessage && updater.phase === "downloaded" ? (
            <p className="about-task-blocker" role="status">
              <WarningCircle aria-hidden="true" />
              {criticalTaskMessage}
            </p>
          ) : null}

          <div className="about-actions">
            <button
              className="about-button about-button--secondary"
              disabled={isRuntimeError ? !canRefreshRuntime : !updater.canCheck}
              type="button"
              onClick={() => void (isRuntimeError
                ? updater.refreshRuntimeInfo()
                : updater.checkManually())}
            >
              <ArrowsClockwise aria-hidden="true" />
              {isRuntimeLoading
                ? "正在读取配置"
                : isRuntimeError
                  ? "重新读取配置"
                  : isChecking
                    ? "正在检查"
                    : "检查更新"}
            </button>
            {updater.canDownload ? (
              <button
                className="about-button about-button--primary"
                type="button"
                onClick={() => onRequestConfirmation("download")}
              >
                <CloudArrowDown aria-hidden="true" />
                {updater.failedAction === "download" && updater.error?.retryable
                  ? "重试下载"
                  : "下载更新"}
              </button>
            ) : null}
            {updater.canDiscard ? (
              <button
                className="about-button about-button--secondary"
                type="button"
                onClick={requestDiscard}
              >
                <X aria-hidden="true" />
                放弃此更新
              </button>
            ) : null}
            {isDiscarding ? (
              <button className="about-button about-button--secondary" disabled type="button">
                <X aria-hidden="true" />
                正在放弃
              </button>
            ) : null}
            {updater.canCancelDownload ? (
              <button
                className="about-button about-button--secondary"
                type="button"
                onClick={() => void updater.cancelDownload()}
              >
                <X aria-hidden="true" />
                取消下载
              </button>
            ) : null}
            {updater.phase === "cancelling" ? (
              <button className="about-button about-button--secondary" disabled type="button">
                正在取消
              </button>
            ) : null}
            {updater.canInstall ? (
              <button
                className="about-button about-button--primary"
                disabled={Boolean(criticalTaskMessage)}
                type="button"
                onClick={() => onRequestConfirmation("install")}
              >
                <Power aria-hidden="true" />
                安装并重启
              </button>
            ) : null}
          </div>
        </article>

        <aside className="about-card about-card--trust">
          <div className="about-card-title">
            <span><Info aria-hidden="true" />更新说明</span>
          </div>
          <ul>
            <li>更新只来自 GitHub 的非 prerelease 稳定 Release。</li>
            <li>下载、验签或安装失败不会替换当前版本。</li>
            <li>扫描、永久删除、视频导出或来源重定位进行时，安装会被后端安全阻止。</li>
            <li>不支持忽略签名、安装旧版本或切换测试通道。</li>
          </ul>
          {updater.runtimeInfo?.endpoint ? (
            <details className="about-endpoint">
              <summary>高级技术信息</summary>
              <span>稳定元数据端点</span>
              <code>{updater.runtimeInfo.endpoint}</code>
            </details>
          ) : null}
        </aside>
      </div>
    </div>
  );
}

function StatusIcon({
  phase,
  hasError,
}: {
  phase: AppUpdaterController["phase"];
  hasError: boolean;
}) {
  if (hasError) {
    return <WarningCircle aria-hidden="true" />;
  }
  if (phase === "up-to-date" || phase === "downloaded") {
    return <CheckCircle aria-hidden="true" />;
  }
  if (phase === "error") {
    return <WarningCircle aria-hidden="true" />;
  }
  if (
    phase === "available"
    || phase === "downloading"
    || phase === "cancelling"
    || phase === "discarding"
  ) {
    return <CloudArrowDown aria-hidden="true" />;
  }
  return <ShieldCheck aria-hidden="true" />;
}

function statusTitle(updater: AppUpdaterController): string {
  if (updater.runtimeStatus === "loading") return "正在读取更新配置";
  if (updater.runtimeError) return "更新配置读取失败";
  if (updater.error) return "更新操作未完成";
  if (updater.phase === "checking") return "正在检查";
  if (updater.phase === "up-to-date") return "已是最新版";
  if (updater.phase === "available") return "发现新版本";
  if (updater.phase === "downloading") return "正在下载";
  if (updater.phase === "cancelling") return "正在取消下载";
  if (updater.phase === "downloaded") return "已准备安装";
  if (updater.phase === "discarding") return "正在放弃更新";
  if (updater.phase === "installing" || updater.phase === "restarting") return "正在更新";
  if (updater.phase === "error") return "更新操作失败";
  return updater.runtimeStatus === "ready" && updater.runtimeInfo?.configured
    ? "稳定更新通道"
    : "更新功能尚未启用";
}

function confirmationTitle(confirmation: Confirmation): string {
  if (confirmation === "reset") return "恢复默认设置？";
  if (confirmation === "install") return "安装更新并重启？";
  if (confirmation === "discard") return "放弃此更新？";
  return "下载此稳定更新？";
}

function confirmationDescription(
  confirmation: Confirmation,
  updater: AppUpdaterController,
  criticalTaskMessage: string | null,
): string {
  if (confirmation === "reset") {
    return "启动位置、启动扫描、素材库视图与排序、播放、动态效果和自动更新偏好将恢复默认值。来源、标签、索引、缓存、视频和更新检查记录不会改变。";
  }
  if (confirmation === "install" && criticalTaskMessage) {
    return `${criticalTaskMessage}。请等待任务结束后再安装更新。`;
  }
  if (confirmation === "install") {
    return "安装开始后瓦刻将关闭，并由受签名保护的安装程序完成升级和重启。请先保存正在编辑的内容。";
  }
  if (confirmation === "discard") {
    return `将清除 v${updater.update?.version ?? "—"} 的待处理状态和已下载文件（如有）。当前版本不受影响，之后可以重新检查更新。`;
  }
  return `将下载 v${updater.update?.version ?? "—"}。下载完成并通过签名验证前，不会更改当前安装。`;
}

function confirmationActionLabel(confirmation: Confirmation): string {
  if (confirmation === "reset") return "确认恢复";
  if (confirmation === "install") return "安装并重启";
  if (confirmation === "discard") return "确认放弃";
  return "确认下载";
}

function hasActionableUpdate(updater: AppUpdaterController): boolean {
  return updater.phase === "available"
    || updater.phase === "downloading"
    || updater.phase === "cancelling"
    || updater.phase === "downloaded"
    || updater.phase === "discarding"
    || updater.phase === "installing"
    || updater.phase === "restarting"
    || (updater.phase === "error" && updater.update !== null && updater.canDiscard);
}

function progressLabel(downloaded: number, total: number | null): string {
  const downloadedLabel = formatBytes(downloaded);
  return total ? `${downloadedLabel} / ${formatBytes(total)}` : downloadedLabel;
}

function formatBytes(value: number): string {
  if (value < 1_024) return `${value} B`;
  if (value < 1_024 * 1_024) return `${(value / 1_024).toFixed(1)} KB`;
  return `${(value / (1_024 * 1_024)).toFixed(1)} MB`;
}

function formatPublishedAt(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleDateString("zh-CN", { year: "numeric", month: "short", day: "numeric" });
}
