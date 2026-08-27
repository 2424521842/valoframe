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
  refreshAdCreatives: () => refreshAdCreatives(),
  recordAdClick: (creativeId: string, slot: string) =>
    recordAdClick(creativeId, slot),
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

  it("refreshes through the maintainer-owned backend configuration before reading cache", async () => {
    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    expect(refreshAdCreatives).toHaveBeenCalledWith();
    expect(listAdCreatives).toHaveBeenCalledTimes(1);
    expect(refreshAdCreatives.mock.invocationCallOrder[0]).toBeLessThan(
      listAdCreatives.mock.invocationCallOrder[0],
    );
  });

  it("fails closed instead of serving a stale cached creative when refresh fails", async () => {
    refreshAdCreatives.mockRejectedValue(new Error("offline"));

    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(refreshAdCreatives).toHaveBeenCalled());
    expect(result.current.creative).toBeNull();
    expect(listAdCreatives).not.toHaveBeenCalled();
  });

  it("degrades to no ad when the refreshed cache cannot be read", async () => {
    listAdCreatives.mockRejectedValue(new Error("db unavailable"));

    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(refreshAdCreatives).toHaveBeenCalled());
    expect(result.current.creative).toBeNull();
  });

  it("shows no card when the backend disables the campaign with an empty list", async () => {
    listAdCreatives.mockResolvedValue([]);
    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(listAdCreatives).toHaveBeenCalled());
    expect(result.current.creative).toBeNull();
  });

  it("delegates click trust policy to the backend", async () => {
    const { result } = renderHook(() => useAdController(options()));

    await waitFor(() => expect(result.current.creative?.creativeId).toBe("cr-001"));
    await result.current.onClick("cr-001");

    expect(recordAdClick).toHaveBeenCalledWith("cr-001", AD_SLOTS.sidebar);
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
