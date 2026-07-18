import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  createElement,
  forwardRef,
  useCallback,
  useLayoutEffect,
  useRef,
  type ComponentProps,
} from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MatchLibrary } from "../../src/components/MatchLibrary";
import { mockClips } from "../../src/data/mockData";
import type { ClipMatchGroup, ClipSummary } from "../../src/types";

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

type Props = ComponentProps<typeof MatchLibrary>;

afterEach(() => vi.restoreAllMocks());

describe("match-level virtualized production library", () => {
  it("keeps rendered match rows and clip cards bounded for both 1k and 10k inputs", () => {
    const { rerender } = render(<MatchLibrary {...propsFor(groups(1_000))} />);
    expect(renderedRows()).toBeGreaterThan(0);
    expect(renderedRows()).toBeLessThan(20);
    expect(renderedCards()).toBeLessThan(20);

    rerender(<MatchLibrary {...propsFor(groups(10_000))} />);
    expect(renderedRows()).toBeGreaterThan(0);
    expect(renderedRows()).toBeLessThan(20);
    expect(renderedCards()).toBeLessThan(20);
  });

  it.each([1_000, 10_000])(
    "keeps a single match with %i clips bounded instead of mounting every card",
    (clipCount) => {
      render(<MatchLibrary {...propsFor([singleGroup(clipCount)])} />);

      expect(renderedRows()).toBe(1);
      expect(renderedCards()).toBeGreaterThan(0);
      expect(renderedCards()).toBeLessThan(40);
    },
  );

  it("moves the bounded card window when the shared scroller moves deep into one match", async () => {
    mockVirtualContentGeometry();
    render(
      <ScrollHarness
        libraryProps={propsFor([singleGroup(10_000)])}
        width={1_200}
      />,
    );

    const scrollHost = screen.getByTestId("virtual-scroll-host");
    const initialIndices = renderedClipIndices();
    expect(initialIndices.length).toBeGreaterThan(0);
    expect(Math.max(...initialIndices)).toBeLessThan(40);

    scrollHost.scrollTop = 80_000;
    fireEvent.scroll(scrollHost);

    await waitFor(() => {
      const nextIndices = renderedClipIndices();
      expect(nextIndices.length).toBeGreaterThan(0);
      expect(nextIndices.length).toBeLessThan(40);
      expect(Math.min(...nextIndices)).toBeGreaterThan(100);
    });
  });

  it("reflows virtual rows for the real width and grid/list mode", async () => {
    mockVirtualContentGeometry();
    const matchGroups = [singleGroup(1_000)];
    const { rerender } = render(
      <ScrollHarness
        libraryProps={propsFor(matchGroups, { viewMode: "grid" })}
        width={1_200}
      />,
    );

    await expectColumnCount(4);
    expect(firstRenderedClipRowCardCount()).toBe(4);

    rerender(
      <ScrollHarness
        libraryProps={propsFor(matchGroups, { viewMode: "list" })}
        width={1_200}
      />,
    );
    await expectColumnCount(1);
    expect(firstRenderedClipRowCardCount()).toBe(1);

    rerender(
      <ScrollHarness
        libraryProps={propsFor(matchGroups, { viewMode: "grid" })}
        width={700}
      />,
    );
    await expectColumnCount(2);
    expect(firstRenderedClipRowCardCount()).toBe(2);
  });

  it("unmounts the large clip window when collapsed and keeps keyboard-loadable pagination", async () => {
    const onLoadMore = vi.fn();
    const { container } = render(
      <MatchLibrary
        {...propsFor([singleGroup(1_000)], {
          hasMore: true,
          onLoadMore,
        })}
      />,
    );
    const header = container.querySelector<HTMLButtonElement>(".match-board-header");
    expect(header).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(header!);
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("match-clip-virtualizer")).not.toBeInTheDocument();
    expect(renderedCards()).toBe(0);

    fireEvent.click(header!);
    await waitFor(() => {
      expect(screen.getByTestId("match-clip-virtualizer")).toBeInTheDocument();
      expect(renderedCards()).toBeGreaterThan(0);
      expect(renderedCards()).toBeLessThan(40);
    });

    fireEvent.click(screen.getByRole("button", { name: /加载更多/ }));
    expect(onLoadMore).toHaveBeenCalledTimes(1);
  });
});

function propsFor(matchGroups: ClipMatchGroup[], overrides: Partial<Props> = {}): Props {
  return {
    matchGroups,
    activeFilterLabels: [],
    selectedClipId: "",
    selectedClipIds: new Set(),
    tags: [],
    totalClipCount: matchGroups.reduce((total, group) => total + group.clips.length, 0),
    viewMode: "grid",
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
    onSelectionGesture: vi.fn(),
    onRequestTrash: vi.fn(),
    onRequestPermanentDelete: vi.fn(),
    onRequestPermanentRemove: vi.fn(),
    onRestoreClip: vi.fn(),
    ...overrides,
  };
}

function ScrollHarness({
  libraryProps,
  width,
}: {
  libraryProps: Props;
  width: number;
}) {
  const scrollElementRef = useRef<HTMLDivElement>(null);
  const setScrollElement = useCallback((element: HTMLDivElement | null) => {
    scrollElementRef.current = element;
    if (element) installScrollGeometry(element);
  }, []);

  useLayoutEffect(() => {
    window.dispatchEvent(new Event("resize"));
  }, [width]);

  return (
    <div
      data-testid="virtual-scroll-host"
      data-viewport-height="800"
      data-viewport-width={width}
      ref={setScrollElement}
    >
      <MatchLibrary {...libraryProps} scrollElementRef={scrollElementRef} />
    </div>
  );
}

function groups(count: number): ClipMatchGroup[] {
  return Array.from({ length: count }, (_, index) => {
    const clip = summary(String(index));
    return {
      id: `match-${index}`,
      accountId: clip.accountId,
      accountDisplayName: clip.accountDisplayName,
      title: `对局 ${index}`,
      subtitle: "虚拟化测试",
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
  });
}

function singleGroup(clipCount: number): ClipMatchGroup {
  const [group] = groups(1);
  const clips = Array.from({ length: clipCount }, (_, index) => summary(String(index)));
  return {
    ...group,
    clips,
    totalSizeBytes: clips.reduce((total, clip) => total + clip.sizeBytes, 0),
  };
}

function summary(id: string): ClipSummary {
  const clip = {
    ...mockClips[0],
    id,
    fileName: `virtual-${id}.mp4`,
    matchId: `match-${id}`,
    clipGroupId: `group-${id}`,
    tags: [...mockClips[0].tags],
  };
  const partial = clip as Partial<typeof clip>;
  delete partial.note;
  delete partial.extractedText;
  delete partial.clipEvents;
  delete partial.eventCount;
  delete partial.roundLabel;
  delete partial.weaponName;
  return clip;
}

function renderedRows(): number {
  return document.querySelectorAll('[data-virtual-row="true"]').length;
}

function renderedCards(): number {
  return document.querySelectorAll("[data-clip-id]").length;
}

function renderedClipIndices(): number[] {
  return [...document.querySelectorAll<HTMLElement>("[data-clip-index]")]
    .map((element) => Number(element.dataset.clipIndex));
}

function firstRenderedClipRowCardCount(): number {
  return document.querySelector('[data-clip-virtual-row="true"]')
    ?.querySelectorAll("[data-clip-id]").length ?? 0;
}

async function expectColumnCount(expected: number): Promise<void> {
  await waitFor(() => {
    expect(screen.getByTestId("match-clip-virtualizer"))
      .toHaveAttribute("data-column-count", String(expected));
  });
}

function installScrollGeometry(element: HTMLDivElement): void {
  if (element.dataset.geometryReady === "true") return;
  element.dataset.geometryReady = "true";
  Object.defineProperties(element, {
    clientHeight: { configurable: true, get: () => viewportHeight(element) },
    clientWidth: { configurable: true, get: () => viewportWidth(element) },
    offsetHeight: { configurable: true, get: () => viewportHeight(element) },
    offsetWidth: { configurable: true, get: () => viewportWidth(element) },
    scrollHeight: { configurable: true, get: () => 4_000_000 },
    scrollTop: { configurable: true, value: 0, writable: true },
  });
  element.getBoundingClientRect = () => rect(
    viewportWidth(element),
    viewportHeight(element),
  );
}

function mockVirtualContentGeometry(): void {
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function () {
    if (this.matches('[data-testid="match-clip-virtual-content"]')) {
      const scrollHost = this.closest<HTMLElement>('[data-testid="virtual-scroll-host"]');
      const width = Math.max(1, viewportWidth(scrollHost) - 54);
      const top = 52 - (scrollHost?.scrollTop ?? 0);
      return rect(width, Number.parseFloat(this.style.height) || 0, top);
    }
    return original.call(this);
  });
}

function viewportWidth(element: HTMLElement | null): number {
  return Number(element?.dataset.viewportWidth) || 1_200;
}

function viewportHeight(element: HTMLElement | null): number {
  return Number(element?.dataset.viewportHeight) || 800;
}

function rect(width: number, height: number, top = 0, left = 0): DOMRect {
  return {
    bottom: top + height,
    height,
    left,
    right: left + width,
    top,
    width,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}
