import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, forwardRef, type ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { MatchLibrary } from "../../src/components/MatchLibrary";
import { mockClips } from "../../src/data/mockData";
import type { Clip, ClipMatchGroup } from "../../src/types";

vi.mock("motion/react", () => {
  const Article = forwardRef<HTMLElement, Record<string, unknown>>((props, ref) => {
    const { animate, initial, transition, whileHover, ...articleProps } = props;
    void animate;
    void initial;
    void transition;
    void whileHover;
    return createElement("article", { ...articleProps, ref });
  });

  return {
    m: { article: Article },
    useReducedMotion: () => false,
  };
});

type MatchLibraryProps = ComponentProps<typeof MatchLibrary>;

function matchGroupFor(clip: Clip): ClipMatchGroup {
  return {
    id: clip.matchId || "match-test",
    accountId: clip.accountId,
    accountDisplayName: clip.accountDisplayName,
    title: "测试对局",
    subtitle: "测试素材",
    clips: [clip],
    latestModifiedAt: clip.modifiedAt,
    totalSizeBytes: clip.sizeBytes,
    resultLabel: "胜利",
    scoreline: clip.scoreline,
    kda: clip.kda,
    mapName: clip.mapName,
    gameMode: clip.gameMode,
    agentName: clip.agentName,
    agentAvatarUrl: "",
  };
}

function createLibraryProps(
  clip: Clip,
  overrides: Partial<MatchLibraryProps> = {},
): MatchLibraryProps {
  return {
    matchGroups: [matchGroupFor(clip)],
    activeFilterLabels: [],
    selectedClipId: "",
    selectedClipIds: new Set(),
    tags: [],
    totalClipCount: 1,
    viewMode: "list",
    isLoading: false,
    errorMessage: null,
    onClearFilters: vi.fn(),
    onRetryLoad: vi.fn(),
    onOpenScan: vi.fn(),
    onSelectClip: vi.fn(),
    onToggleFavorite: vi.fn(),
    onCopyPath: vi.fn(),
    onOpenOriginal: vi.fn(),
    isTrashMode: false,
    removableFromIndexIds: new Set(),
    onSelectionGesture: vi.fn(),
    onRequestTrash: vi.fn(),
    onRequestPermanentDelete: vi.fn(),
    onRequestPermanentRemove: vi.fn(),
    onRestoreClip: vi.fn(),
    ...overrides,
  };
}

describe("MatchLibrary card behavior", () => {
  it("shows a dedicated empty state when the recycle bin has no videos", () => {
    const onOpenScan = vi.fn();
    const { container } = render(
      <MatchLibrary
        {...createLibraryProps(mockClips[0], {
          isTrashMode: true,
          matchGroups: [],
          onOpenScan,
          totalClipCount: 0,
        })}
      />,
    );

    expect(screen.getByRole("heading", { name: "回收站里没有视频" })).toBeInTheDocument();
    expect(screen.getByText("移入回收站的视频会显示在这里。")).toBeInTheDocument();
    expect(container.querySelector(".match-library-state-icon svg")).toBeInTheDocument();
    expect(screen.queryByText("还没有本地高光")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "前往扫描" })).not.toBeInTheDocument();
    expect(onOpenScan).not.toHaveBeenCalled();
  });

  it("shows index-only cleanup on an eligible ordinary-library card", async () => {
    const user = userEvent.setup();
    const clip = { ...mockClips[0], fileStatus: "missing" };
    const onRequestPermanentRemove = vi.fn();
    const { container } = render(
      <MatchLibrary
        {...createLibraryProps(clip, {
          onRequestPermanentRemove,
          removableFromIndexIds: new Set([clip.id]),
        })}
      />,
    );

    fireEvent.contextMenu(container.querySelector(".match-clip-card") as HTMLElement);
    await user.click(await screen.findByText("仅移除失联索引"));
    expect(onRequestPermanentRemove).toHaveBeenCalledWith(clip.id);
  });

  it("does not expose index-only cleanup for an available ordinary-library card", async () => {
    const clip = { ...mockClips[0], fileStatus: "available" };
    const { container } = render(<MatchLibrary {...createLibraryProps(clip)} />);

    fireEvent.contextMenu(container.querySelector(".match-clip-card") as HTMLElement);
    expect(await screen.findByText("移入回收站")).toBeVisible();
    expect(screen.queryByText("仅移除失联索引")).not.toBeInTheDocument();
  });

  it("renders bundled agent and map artwork with resilient local fallbacks", () => {
    const clip = {
      ...mockClips[0],
      agentName: "尚勃勒",
      mapName: "源工重镇",
      thumbnailUrl: null,
    };
    const { container } = render(
      <MatchLibrary {...createLibraryProps(clip)} />,
    );

    const agentImage = container.querySelector<HTMLImageElement>(".match-board-agent img");
    const mapImage = container.querySelector<HTMLImageElement>(".match-board-map-art img");
    expect(agentImage?.getAttribute("src")).toMatch(/^\/valorant-assets\/agents\/.+\.png$/);
    expect(mapImage?.getAttribute("src")).toMatch(/^\/valorant-assets\/maps\/.+\.png$/);

    fireEvent.error(agentImage!);
    fireEvent.error(mapImage!);
    expect(container.querySelector(".match-board-agent img")).not.toBeInTheDocument();
    expect(container.querySelector(".match-board-agent--fallback")).toHaveTextContent("尚勃");
    expect(container.querySelector(".match-board-map-art img")).not.toBeInTheDocument();
    expect(container.querySelector(".match-board-map-art")).toBeInTheDocument();
  });

  it("keeps checkbox and favorite keyboard activation from opening preview", async () => {
    const user = userEvent.setup();
    const clip = { ...mockClips[0], id: "keyboard-clip", thumbnailUrl: null };
    const onSelectClip = vi.fn();
    const onToggleFavorite = vi.fn();
    const onSelectionGesture = vi.fn();
    render(
      <MatchLibrary
        {...createLibraryProps(clip, {
          onSelectClip,
          onToggleFavorite,
          onSelectionGesture,
        })}
      />,
    );

    const checkbox = screen.getByRole("checkbox", { name: /^选择/ });
    checkbox.focus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");

    const favorite = screen.getByRole("button", { name: /收藏$/ });
    favorite.focus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");

    expect(onSelectionGesture).toHaveBeenCalledTimes(1);
    expect(onToggleFavorite).toHaveBeenCalledTimes(2);
    expect(onSelectClip).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /^预览/ }));
    expect(onSelectClip).toHaveBeenCalledTimes(1);
  });

  it("shows distinct official video scores and never falls back to the match combat score", () => {
    const scoredClip = {
      ...mockClips[0],
      id: "scored-clip",
      combatScore: 987,
      roundScore: 1230,
      thumbnailUrl: null,
    };
    const secondScoredClip = {
      ...scoredClip,
      id: "second-scored-clip",
      officialVideoName: "三杀时刻",
      roundScore: 635,
    };
    const scoredGroup = {
      ...matchGroupFor(scoredClip),
      clips: [scoredClip, secondScoredClip],
    };
    const { rerender } = render(
      <MatchLibrary
        {...createLibraryProps(scoredClip, {
          matchGroups: [scoredGroup],
          totalClipCount: 2,
          viewMode: "grid",
        })}
      />,
    );

    expect(screen.getByText("1230 评分")).toHaveClass("match-clip-score");
    expect(screen.getByText("635 评分")).toHaveClass("match-clip-score");
    expect(screen.queryByText("987 评分")).not.toBeInTheDocument();

    const unscoredClip = { ...scoredClip, roundScore: null };
    rerender(
      <MatchLibrary {...createLibraryProps(unscoredClip, { viewMode: "grid" })} />,
    );

    expect(screen.queryByText("1230 评分")).not.toBeInTheDocument();
    expect(screen.queryByText("987 评分")).not.toBeInTheDocument();
    expect(screen.getByText("官方未同步")).toHaveClass(
      "match-clip-score",
      "match-clip-score--unavailable",
    );
  });

  it("keeps source directory names out of cards and shows favorite independently of legacy review data", () => {
    const clip = {
      ...mockClips[0],
      sourceDirName: "wonderfulVideos94985665477093",
      sourceRelativeDir: "wonderfulVideos94985665477093",
      reviewDecision: "liked" as const,
      isFavorite: true,
      thumbnailUrl: null,
    };
    const { container } = render(
      <MatchLibrary {...createLibraryProps(clip, { viewMode: "grid" })} />,
    );

    expect(screen.queryByText(/wonderfulVideos94985665477093/)).not.toBeInTheDocument();
    expect(screen.queryByText("喜欢")).not.toBeInTheDocument();
    expect(container.querySelector(".match-clip-review")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "取消收藏" })).toHaveClass(
      "match-clip-favorite",
      "match-clip-favorite--active",
    );
    expect(container.querySelector(".match-clip-source-context")).not.toBeInTheDocument();
  });

  it("omits score placeholders for official non-scoring videos but keeps real scores", () => {
    const baseClip = {
      ...mockClips[0],
      roundScore: null,
      thumbnailUrl: null,
    };
    const nonScoringClips = [
      {
        ...baseClip,
        id: "kill-compilation",
        officialVideoName: "击杀集锦",
        officialVideoType: "击杀集锦",
        highlightType: 2,
      },
      {
        ...baseClip,
        id: "death-compilation",
        officialVideoName: "死亡时刻",
        officialVideoType: "死亡集锦",
        highlightType: 3,
      },
      {
        ...baseClip,
        id: "night-market",
        officialVideoName: "夜市翻牌",
        officialVideoType: "夜市翻牌",
        highlightType: null,
        gameMode: "夺还模式",
      },
    ];
    const nonScoringGroup = {
      ...matchGroupFor(nonScoringClips[0]),
      clips: nonScoringClips,
    };
    const { container, rerender } = render(
      <MatchLibrary
        {...createLibraryProps(nonScoringClips[0], {
          matchGroups: [nonScoringGroup],
          totalClipCount: nonScoringClips.length,
          viewMode: "grid",
        })}
      />,
    );

    expect(screen.queryByText("官方未同步")).not.toBeInTheDocument();
    expect(screen.queryByText("暂无")).not.toBeInTheDocument();
    expect(container.querySelector(".match-clip-score")).not.toBeInTheDocument();

    const scoredCompilation = { ...nonScoringClips[0], roundScore: 741 };
    rerender(
      <MatchLibrary
        {...createLibraryProps(scoredCompilation, { viewMode: "grid" })}
      />,
    );

    expect(screen.getByText("741 评分")).toHaveClass("match-clip-score");
  });

  it("falls back after a thumbnail error and retries when the revisioned URL changes", () => {
    const first = {
      ...mockClips[0],
      id: "thumbnail-clip",
      thumbnailUrl: "clip-media://cover/42?v=rev-1",
      thumbnailRevision: "rev-1",
    };
    const { container, rerender } = render(
      <MatchLibrary {...createLibraryProps(first)} />,
    );

    const failedImage = container.querySelector<HTMLImageElement>(".match-clip-thumb img");
    expect(failedImage).toHaveAttribute("src", first.thumbnailUrl);
    fireEvent.error(failedImage!);
    expect(container.querySelector(".match-clip-thumb img")).not.toBeInTheDocument();
    expect(container.querySelector(".clip-thumb-fallback")).toBeInTheDocument();

    rerender(<MatchLibrary {...createLibraryProps({ ...first, thumbnailUrl: null })} />);
    expect(container.querySelector(".clip-thumb-fallback")).toBeInTheDocument();
    rerender(<MatchLibrary {...createLibraryProps(first)} />);
    expect(container.querySelector(".match-clip-thumb img"))
      .toHaveAttribute("src", first.thumbnailUrl);
    fireEvent.error(container.querySelector(".match-clip-thumb img")!);

    const updated = {
      ...first,
      thumbnailUrl: "clip-media://cover/42?v=rev-2",
      thumbnailRevision: "rev-2",
    };
    rerender(<MatchLibrary {...createLibraryProps(updated)} />);
    expect(container.querySelector(".clip-thumb-fallback")).not.toBeInTheDocument();
    expect(container.querySelector(".match-clip-thumb img"))
      .toHaveAttribute("src", updated.thumbnailUrl);
  });
});
