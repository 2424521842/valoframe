import {
  ClockCounterClockwise,
  FolderOpen,
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
  isOpen: boolean;
  isOverlay: boolean;
  onClose: () => void;
  onModeChange: (mode: LibraryMode) => void;
  onOpenTagManager: () => void;
  onOpenScan: () => void;
};

export function CinematicSidebar({
  activeScreen,
  activeMode,
  totalCount,
  recentCount,
  favoriteCount,
  trashCount,
  tagCount,
  isOpen,
  isOverlay,
  onClose,
  onModeChange,
  onOpenTagManager,
  onOpenScan,
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
      </nav>

    </aside>
  );
}

type SidebarActionProps = {
  active?: boolean;
  badge?: string;
  count?: number;
  icon: ReactNode;
  label: string;
  onClick: () => void;
};

function SidebarAction({
  active = false,
  badge,
  count,
  icon,
  label,
  onClick,
}: SidebarActionProps) {
  return (
    <button
      aria-current={active ? "page" : undefined}
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
