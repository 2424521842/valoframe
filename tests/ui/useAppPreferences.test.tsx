import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { UiSwitch } from "../../src/components/ui/switch";
import { useAppPreferences } from "../../src/hooks/useAppPreferences";
import {
  APP_PREFERENCES_STORAGE_ERRORS,
  APP_PREFERENCES_STORAGE_KEY,
  DEFAULT_APP_PREFERENCES,
  type AppPreferencesStorage,
  type AppPreferencesV1,
} from "../../src/lib/appPreferences";

describe("useAppPreferences", () => {
  it("reads persisted preferences synchronously on the first render", () => {
    const persisted: AppPreferencesV1 = {
      ...DEFAULT_APP_PREFERENCES,
      startupDestination: "library-favorites",
      previewVolumePercent: 42,
      previewMuted: true,
    };
    const storage = memoryStorage({
      [APP_PREFERENCES_STORAGE_KEY]: JSON.stringify(persisted),
    });

    const controller = renderHook(() => useAppPreferences({ storage }));

    expect(controller.result.current.preferences).toEqual(persisted);
    expect(controller.result.current.storageError).toBeNull();
  });

  it("keeps updates in memory when a write fails and clears the error after a later success", () => {
    const values = new Map<string, string>();
    let writesFail = true;
    const storage: AppPreferencesStorage = {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        if (writesFail) throw new Error("quota exceeded");
        values.set(key, value);
      },
      removeItem: (key) => {
        values.delete(key);
      },
    };
    const controller = renderHook(() => useAppPreferences({ storage }));

    act(() => {
      controller.result.current.updatePreferences({ libraryViewMode: "list" });
    });
    expect(controller.result.current.preferences.libraryViewMode).toBe("list");
    expect(controller.result.current.storageError).toBe(
      APP_PREFERENCES_STORAGE_ERRORS.write,
    );
    expect(values.has(APP_PREFERENCES_STORAGE_KEY)).toBe(false);

    writesFail = false;
    act(() => {
      controller.result.current.updatePreferences({ librarySort: "name-asc" });
    });
    expect(controller.result.current.storageError).toBeNull();
    expect(JSON.parse(values.get(APP_PREFERENCES_STORAGE_KEY)!)).toMatchObject({
      libraryViewMode: "list",
      librarySort: "name-asc",
    });
  });

  it("recovers from a read failure when the next write succeeds", () => {
    const values = new Map<string, string>();
    const storage: AppPreferencesStorage = {
      getItem() {
        throw new Error("storage blocked");
      },
      setItem: (key, value) => {
        values.set(key, value);
      },
      removeItem: (key) => {
        values.delete(key);
      },
    };
    const controller = renderHook(() => useAppPreferences({ storage }));

    expect(controller.result.current.preferences).toEqual(DEFAULT_APP_PREFERENCES);
    expect(controller.result.current.storageError).toBe(
      APP_PREFERENCES_STORAGE_ERRORS.read,
    );

    act(() => {
      controller.result.current.updatePreferences({ previewMuted: true });
    });
    expect(controller.result.current.preferences.previewMuted).toBe(true);
    expect(controller.result.current.storageError).toBeNull();
    expect(values.has(APP_PREFERENCES_STORAGE_KEY)).toBe(true);
  });

  it("reset removes only the preference key and restores defaults", () => {
    const storage = memoryStorage({
      [APP_PREFERENCES_STORAGE_KEY]: JSON.stringify({
        ...DEFAULT_APP_PREFERENCES,
        motionMode: "reduced",
      }),
      "valoframe.unrelated": "keep-me",
    });
    const controller = renderHook(() => useAppPreferences({ storage }));

    act(() => controller.result.current.resetPreferences());

    expect(controller.result.current.preferences).toEqual(DEFAULT_APP_PREFERENCES);
    expect(controller.result.current.storageError).toBeNull();
    expect(storage.getItem(APP_PREFERENCES_STORAGE_KEY)).toBeNull();
    expect(storage.getItem("valoframe.unrelated")).toBe("keep-me");
  });

  it("keeps reset defaults in memory when removing persisted data fails", () => {
    const storage: AppPreferencesStorage = {
      getItem: () => JSON.stringify({
        ...DEFAULT_APP_PREFERENCES,
        automaticUpdateCheck: false,
      }),
      setItem: () => undefined,
      removeItem() {
        throw new Error("blocked");
      },
    };
    const controller = renderHook(() => useAppPreferences({ storage }));

    act(() => controller.result.current.resetPreferences());

    expect(controller.result.current.preferences).toEqual(DEFAULT_APP_PREFERENCES);
    expect(controller.result.current.storageError).toBe(
      APP_PREFERENCES_STORAGE_ERRORS.remove,
    );
  });

  it("ignores invalid runtime patches instead of corrupting session state", () => {
    const controller = renderHook(() => useAppPreferences({ storage: null }));

    act(() => {
      controller.result.current.updatePreferences({
        previewVolumePercent: 101,
        motionMode: "animated",
      } as never);
    });

    expect(controller.result.current.preferences).toEqual(DEFAULT_APP_PREFERENCES);
    expect(controller.result.current.storageError).toBeNull();
  });
});

describe("UiSwitch", () => {
  it("preserves Radix switch semantics and state attributes", () => {
    render(
      <UiSwitch
        aria-label="自动检查更新"
        className="custom-switch"
        defaultChecked={false}
      />,
    );

    const control = screen.getByRole("switch", { name: "自动检查更新" });
    expect(control).toHaveClass("ui-switch", "custom-switch");
    expect(control).toHaveAttribute("data-state", "unchecked");
    expect(control.querySelector(".ui-switch-thumb")).not.toBeNull();

    fireEvent.click(control);
    expect(control).toHaveAttribute("data-state", "checked");
  });
});

function memoryStorage(initialValues: Record<string, string> = {}): AppPreferencesStorage {
  const values = new Map(Object.entries(initialValues));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      values.delete(key);
    },
  };
}
