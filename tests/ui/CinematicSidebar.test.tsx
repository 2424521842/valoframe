import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CinematicSidebar } from "../../src/components/CinematicSidebar";

describe("CinematicSidebar", () => {
  it("keeps the recent badge and count in one metadata cell without a smart-filter entry", () => {
    const onOpenReview = vi.fn();
    render(
      <CinematicSidebar
        activeMode="all"
        activeScreen="library"
        favoriteCount={12}
        isOpen
        isOverlay={false}
        recentCount={0}
        tagCount={12}
        totalCount={774}
        trashCount={0}
        onClose={vi.fn()}
        onModeChange={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenScan={vi.fn()}
        onOpenReview={onOpenReview}
        onOpenTagManager={vi.fn()}
      />,
    );

    const recentButton = screen.getByText("最近添加").closest("button");
    expect(recentButton).not.toBeNull();

    const metadata = recentButton?.querySelector(".cinematic-sidebar-item-meta");
    expect(metadata).not.toBeNull();
    expect(within(metadata as HTMLElement).getByText("NEW")).toBeVisible();
    expect(within(metadata as HTMLElement).getByText("0")).toBeVisible();
    expect(screen.queryByText("智能筛选")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "设置" })).toBeVisible();
    screen.getByRole("button", { name: "快速挑片" }).click();
    expect(onOpenReview).toHaveBeenCalledTimes(1);
  });

  it("surfaces a background update with a versioned accessible label", () => {
    render(
      <CinematicSidebar
        activeMode="all"
        activeScreen="library"
        favoriteCount={0}
        isOpen
        isOverlay={false}
        recentCount={0}
        tagCount={0}
        totalCount={0}
        trashCount={0}
        updateBadge="待安装"
        updateVersion="0.2.1"
        onClose={vi.fn()}
        onModeChange={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenScan={vi.fn()}
        onOpenTagManager={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", {
      name: "设置，更新已下载 v0.2.1",
    })).toHaveTextContent("待安装");
  });
});
