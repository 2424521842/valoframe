import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CinematicSidebar } from "../../src/components/CinematicSidebar";

describe("CinematicSidebar", () => {
  it("keeps the recent badge and count in one metadata cell without a smart-filter entry", () => {
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
        onOpenScan={vi.fn()}
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
  });
});
