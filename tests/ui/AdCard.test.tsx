import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AdCard } from "../../src/components/AdCard";
import type { AdCreative } from "../../src/lib/ads";

vi.mock("../../src/api/backend", () => ({
  adImageUrl: (imagePath: string) => `clip-media://${imagePath}`,
}));

function creative(overrides: Partial<AdCreative> = {}): AdCreative {
  return {
    creativeId: "cr-001",
    title: "示例广告标题",
    body: "示例描述文案",
    advertiserName: "某广告主",
    weight: 100,
    startAt: null,
    endAt: null,
    imagePath: "ad/cr-001",
    ...overrides,
  };
}

describe("AdCard", () => {
  it("renders nothing when there is no creative", () => {
    const { container } = render(
      <AdCard creative={null} onClick={vi.fn()} onImpression={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the mandatory 广告 label and the advertiser name", () => {
    render(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={vi.fn()} />,
    );

    expect(screen.getByText("广告")).toBeTruthy();
    expect(screen.getByText("某广告主")).toBeTruthy();
    expect(screen.getByText("示例广告标题")).toBeTruthy();
    expect(screen.getByText("示例描述文案")).toBeTruthy();
  });

  it("loads the image through the local clip-media protocol, never a vendor URL", () => {
    render(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={vi.fn()} />,
    );

    const image = document.querySelector("img");
    expect(image?.getAttribute("src")).toBe("clip-media://ad/cr-001");
  });

  it("reports an impression once per creative", () => {
    const onImpression = vi.fn();
    const { rerender } = render(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={onImpression} />,
    );

    rerender(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={onImpression} />,
    );

    expect(onImpression).toHaveBeenCalledTimes(1);
    expect(onImpression).toHaveBeenCalledWith("cr-001");
  });

  it("reports a new impression when the creative rotates", () => {
    const onImpression = vi.fn();
    const { rerender } = render(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={onImpression} />,
    );

    rerender(
      <AdCard
        creative={creative({ creativeId: "cr-002" })}
        onClick={vi.fn()}
        onImpression={onImpression}
      />,
    );

    expect(onImpression).toHaveBeenCalledTimes(2);
    expect(onImpression).toHaveBeenLastCalledWith("cr-002");
  });

  it("passes the creative id to the click handler", async () => {
    const onClick = vi.fn();
    render(
      <AdCard creative={creative()} onClick={onClick} onImpression={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button"));

    expect(onClick).toHaveBeenCalledWith("cr-001");
  });

  it("labels the click target as an ad for screen readers", () => {
    render(
      <AdCard creative={creative()} onClick={vi.fn()} onImpression={vi.fn()} />,
    );

    expect(
      screen.getByRole("button", { name: "广告：示例广告标题（某广告主）" }),
    ).toBeTruthy();
  });
});
