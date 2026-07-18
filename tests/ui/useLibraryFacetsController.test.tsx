import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LibraryFacets } from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  getLibraryFacets: vi.fn(),
}));

vi.mock("../../src/api/backend", () => ({
  commandErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  getLibraryFacets: mocks.getLibraryFacets,
}));

import { useLibraryFacetsController } from "../../src/hooks/useLibraryFacetsController";

describe("useLibraryFacetsController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getLibraryFacets.mockResolvedValue(facets(10));
  });

  it("loads once under StrictMode and exposes the successful whole-library result", async () => {
    const { result } = renderHook(() => useLibraryFacetsController(), {
      reactStrictMode: true,
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(mocks.getLibraryFacets).toHaveBeenCalledTimes(1);
    expect(result.current.facets?.activeCount).toBe(10);
    expect(result.current.error).toBeNull();
  });

  it("lets the newest refresh win over an older response", async () => {
    const oldRefresh = deferred<LibraryFacets>();
    const newRefresh = deferred<LibraryFacets>();
    mocks.getLibraryFacets
      .mockResolvedValueOnce(facets(10))
      .mockReturnValueOnce(oldRefresh.promise)
      .mockReturnValueOnce(newRefresh.promise);
    const { result } = renderHook(() => useLibraryFacetsController());
    await waitFor(() => expect(result.current.facets?.activeCount).toBe(10));

    let oldPromise!: Promise<boolean>;
    let newPromise!: Promise<boolean>;
    act(() => {
      oldPromise = result.current.refresh();
      newPromise = result.current.refresh();
    });
    expect(result.current.isLoading).toBe(true);

    await act(async () => {
      newRefresh.resolve(facets(30));
      await newPromise;
    });
    expect(result.current.facets?.activeCount).toBe(30);
    expect(result.current.isLoading).toBe(false);

    await act(async () => {
      oldRefresh.resolve(facets(20));
      await oldPromise;
    });
    expect(result.current.facets?.activeCount).toBe(30);
    expect(await oldPromise).toBe(false);
  });

  it("reports failure, retains last-known-good facets, and clears the error on retry", async () => {
    mocks.getLibraryFacets
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(facets(40))
      .mockRejectedValueOnce(new Error("refresh failed"))
      .mockResolvedValueOnce(facets(50));
    const { result } = renderHook(() => useLibraryFacetsController());

    await waitFor(() => expect(result.current.error).toContain("offline"));
    expect(result.current.facets).toBeNull();
    expect(result.current.isLoading).toBe(false);

    await act(async () => {
      expect(await result.current.refresh()).toBe(true);
    });
    expect(result.current.facets?.activeCount).toBe(40);
    expect(result.current.error).toBeNull();

    await act(async () => {
      expect(await result.current.refresh()).toBe(false);
    });
    expect(result.current.facets?.activeCount).toBe(40);
    expect(result.current.error).toContain("refresh failed");

    await act(async () => {
      expect(await result.current.refresh()).toBe(true);
    });
    expect(result.current.facets?.activeCount).toBe(50);
    expect(result.current.error).toBeNull();
  });
});

function facets(activeCount: number): LibraryFacets {
  return libraryFacets({ activeCount, totalCount: activeCount });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}
