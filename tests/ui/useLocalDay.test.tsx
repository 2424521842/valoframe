import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useLocalDay } from "../../src/hooks/useLocalDay";
import { dateRangeForPreset } from "../../src/lib/libraryFlow";

describe("useLocalDay", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("advances date preset boundaries when a long-running app crosses midnight", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 20, 23, 59, 59, 950));

    const { result } = renderHook(() => useLocalDay());
    expect(dateRangeForPreset("today", result.current)).toEqual({
      modifiedFrom: "2026-07-20",
      modifiedTo: "2026-07-20",
    });

    act(() => {
      vi.advanceTimersByTime(50);
    });

    expect(dateRangeForPreset("week", result.current)).toEqual({
      modifiedFrom: "2026-07-15",
      modifiedTo: "2026-07-21",
    });
  });

  it("catches up and reschedules when a suspended app regains focus", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 20, 12));

    const { result } = renderHook(() => useLocalDay());
    vi.setSystemTime(new Date(2026, 6, 22, 8));
    expect(dateRangeForPreset("today", result.current)).toEqual({
      modifiedFrom: "2026-07-20",
      modifiedTo: "2026-07-20",
    });

    act(() => window.dispatchEvent(new Event("focus")));

    expect(dateRangeForPreset("today", result.current)).toEqual({
      modifiedFrom: "2026-07-22",
      modifiedTo: "2026-07-22",
    });
  });
});
