import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useAdController } from "../../src/hooks/useAdController";
import { AD_SLOTS } from "../../src/lib/ads";

const listAdCreatives = vi.fn();
const refreshAdCreatives = vi.fn();
const recordAdClick = vi.fn();
const recordAdImpression = vi.fn();

vi.mock("../../src/api/backend", () => ({
  listAdCreatives: () => listAdCreatives(),
  refreshAdCreatives: (endpoint: string) => refreshAdCreatives(endpoint),
  recordAdClick: (creativeId: string, slot: string, hosts: string[]) =>
    recordAdClick(creativeId, slot, hosts),
  recordAdImpression: (creativeId: string, slot: string) =>
    recordAdImpression(creativeId, slot),
}));

const creative = {
  creativeId: "cr-001",
  title: "标题",
  body: null,
  advertiserName: "广告主",
  weight: 100,
  startAt: null,
  endAt: null,
  imagePath: "ad/cr-001",
};

function options(overrides: Record<string, unknown> = {}) {
  return {
    enabled: true,
    manifestEndpoint: "https://ad.example.com/manifest",
    allowedHosts: "ad.example.com",
    slot: AD_SLOTS.sidebar,
    ...overrides,
  } as Parameters<typeof useAdController>[0];
}

describe("useAdController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listAdCreatives.mockResolvedValue([creative]);
    refreshAdCreatives.mockResolvedValue(1);
    recordAdClick.mockResolvedValue("vf-1-2");
    recordAdImpression.mockResolvedValue(undefined);
  });

  it("does not touch the network when ads are disabled", async () => {
    const { result } = renderHook(() => useAdController(options({ enabled: false })));

    await waitFor(() => expect(result.current.creative).toBeNull());
    expect(refreshAdCreatives).not.toHaveBeenCalled();
    expect(listAdCreatives).not.toHaveBeenCalled();
  });

  it("does not fetch when no endpoint is configured", async () => {
    const { result } = renderHook(() =>
      useAdController(options({ manifestEndpoint: "  " })),
    );

    await waitFor(() => expect(result.current.creative).toBeNull());
    expect(refreshAdCreatives).not.toHaveBeenCalled();
  });

  it("serves the cached creative when the refresh fails offline", async () => {
    refreshAdCreatives.mockRejectedValue(new Error("offline"));

    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
  });

  it("degrades to no ad when the cache read also fails", async () => {
    refreshAdCreatives.mockRejectedValue(new Error("offline"));
    listAdCreatives.mockRejectedValue(new Error("db unavailable"));

    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(refreshAdCreatives).toHaveBeenCalled());
    expect(result.current.creative).toBeNull();
  });

  it("blocks clicks when the landing host allowlist is empty", async () => {
    const { result } = renderHook(() => useAdController(options({ allowedHosts: "" })));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    await result.current.onClick("cr-001");

    expect(recordAdClick).not.toHaveBeenCalled();
  });

  it("sends the parsed host list with a click", async () => {
    const { result } = renderHook(() =>
      useAdController(options({ allowedHosts: "https://ad.example.com/lp, lp.example.com" })),
    );

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    await result.current.onClick("cr-001");

    expect(recordAdClick).toHaveBeenCalledWith("cr-001", AD_SLOTS.sidebar, [
      "ad.example.com",
      "lp.example.com",
    ]);
  });

  it("reports each creative impression only once", async () => {
    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    result.current.onImpression("cr-001");
    result.current.onImpression("cr-001");

    expect(recordAdImpression).toHaveBeenCalledTimes(1);
  });

  it("swallows click failures rather than surfacing them", async () => {
    recordAdClick.mockRejectedValue(new Error("host not allowed"));
    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    await expect(result.current.onClick("cr-001")).resolves.toBeUndefined();
  });
});
