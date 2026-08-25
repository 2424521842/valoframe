import {
  ClockCounterClockwise,
  CheckSquare,
  FolderOpen,
  GearSix,
  Heart,
  SquaresFour,
  Tag,
  Trash,
} from "@phosphor-icons/react";
import type { ReactNode } from "react";
import type { AppScreen, LibraryMode } from "../types";

type CinematicSidebarProps = {
  activeScreen: AppScreen;
  activeMode: LibraryMode;
  totalCount: number;
  recentCount: number;
  favoriteCount: number;
  trashCount: number;
  tagCount: number;
  updateBadge?: "更新" | "待安装";
  updateVersion?: string;
  isOpen: boolean;
  isOverlay: boolean;
  /** Rendered below the nav; null when ads are disabled or unavailable. */
  adSlot?: ReactNode;
  onClose: () => void;
  onModeChange: (mode: LibraryMode) => void;
  onOpenTagManager: () => void;
  onOpenScan: () => void;
  onOpenSettings: () => void;
  onOpenReview?: () => void;
};

export function CinematicSidebar({
  activeScreen,
  activeMode,
  totalCount,
  recentCount,
  favoriteCount,
  trashCount,
  tagCount,
  updateBadge,
  updateVersion,
  isOpen,
  isOverlay,
  adSlot = null,
  onClose,
  onModeChange,
  onOpenTagManager,
  onOpenScan,
  onOpenSettings,
  onOpenReview = () => undefined,
}: CinematicSidebarProps) {
  const hidden = isOverlay && !isOpen;

  return (
    <aside
      aria-hidden={hidden || undefined}
      aria-label="主导航"
      className={`cinematic-sidebar${isOpen ? " cinematic-sidebar--open" : ""}`}
      inert={hidden || undefined}
    >
      <button
        aria-label="关闭主导航"
        className="cinematic-sidebar-close"
        type="button"
        onClick={onClose}
      >
        ×
      </button>

      <nav aria-label="素材工作流">
        <SidebarAction
          active={activeScreen === "library" && activeMode === "all"}
          count={totalCount}
          icon={<SquaresFour weight="duotone" />}
          label="全部素材"
          onClick={() => onModeChange("all")}
        />
        <SidebarAction
          active={activeScreen === "library" && activeMode === "today"}
          badge="NEW"
          count={recentCount}
          icon={<ClockCounterClockwise weight="duotone" />}
          label="最近添加"
          onClick={() => onModeChange("today")}
        />
        <SidebarAction
          active={activeScreen === "library" && activeMode === "favorites"}
          count={favoriteCount}
          icon={<Heart weight="duotone" />}
          label="收藏"
          onClick={() => onModeChange("favorites")}
        />
        <SidebarAction
          active={activeScreen === "tags"}
          count={tagCount}
          icon={<Tag weight="duotone" />}
          label="自定义标签"
          onClick={onOpenTagManager}
        />
        <SidebarAction
          active={activeScreen === "review"}
          icon={<CheckSquare weight="duotone" />}
          label="快速挑片"
          onClick={onOpenReview}
        />
        <SidebarAction
          active={activeScreen === "library" && activeMode === "trash"}
          count={trashCount}
          icon={<Trash weight="duotone" />}
          label="回收站"
          onClick={() => onModeChange("trash")}
        />
        <SidebarAction
          active={activeScreen === "scan"}
          icon={<FolderOpen weight="duotone" />}
          label="扫描目录"
          onClick={onOpenScan}
        />
        <SidebarAction
          active={activeScreen === "settings"}
          ariaLabel={updateBadge
            ? `设置，${updateBadge === "待安装" ? "更新已下载" : "有可用更新"}${updateVersion ? ` v${updateVersion}` : ""}`
            : undefined}
          badge={updateBadge}
          icon={<GearSix weight="duotone" />}
          label="设置"
          onClick={onOpenSettings}
        />
      </nav>

      {adSlot ? <div className="cinematic-sidebar-ad">{adSlot}</div> : null}

    </aside>
  );
}

type SidebarActionProps = {
  active?: boolean;
  ariaLabel?: string;
  badge?: string;
  count?: number;
  icon: ReactNode;
  label: string;
  onClick: () => void;
};

function SidebarAction({
  active = false,
  ariaLabel,
  badge,
  count,
  icon,
  label,
  onClick,
}: SidebarActionProps) {
  return (
    <button
      aria-current={active ? "page" : undefined}
      aria-label={ariaLabel}
      className={active ? "cinematic-sidebar-item cinematic-sidebar-item--active" : "cinematic-sidebar-item"}
      type="button"
      onClick={onClick}
    >
      {icon}
      <span>{label}</span>
      {badge || typeof count === "number" ? (
        <span className="cinematic-sidebar-item-meta">
          {badge ? <em>{badge}</em> : null}
          {typeof count === "number" ? <strong>{count.toLocaleString("zh-CN")}</strong> : null}
        </span>
      ) : null}
    </button>
  );
}
