import type { ClipSort, LibraryViewMode } from "../types";

export const APP_PREFERENCES_STORAGE_KEY = "valoframe.preferences.v1";
export const APP_PREFERENCES_SCHEMA_VERSION = 1 as const;

export type StartupDestination =
  | "library-all"
  | "library-today"
  | "library-favorites"
  | "review"
  | "scan";

export type MotionMode = "system" | "reduced";

export type AppPreferencesV1 = {
  schemaVersion: typeof APP_PREFERENCES_SCHEMA_VERSION;
  startupDestination: StartupDestination;
  libraryViewMode: LibraryViewMode;
  librarySort: ClipSort;
  previewVolumePercent: number;
  previewMuted: boolean;
  reviewAutoplay: boolean;
  motionMode: MotionMode;
  automaticUpdateCheck: boolean;
};

export type AppPreferencesPatch = Partial<
  Omit<AppPreferencesV1, "schemaVersion">
>;

export type AppPreferencesStorage = Pick<
  Storage,
  "getItem" | "setItem" | "removeItem"
>;

export const APP_PREFERENCES_STORAGE_ERRORS = Object.freeze({
  read: "无法读取已保存的设置，已改用默认设置。",
  write: "无法保存设置；更改仅在本次会话中生效。",
  remove: "无法清除已保存的设置；默认设置仅在本次会话中生效。",
});

export const DEFAULT_APP_PREFERENCES: Readonly<AppPreferencesV1> = Object.freeze({
  schemaVersion: APP_PREFERENCES_SCHEMA_VERSION,
  startupDestination: "library-all",
  libraryViewMode: "grid",
  librarySort: "modified-desc",
  previewVolumePercent: 100,
  previewMuted: false,
  reviewAutoplay: true,
  motionMode: "system",
  automaticUpdateCheck: true,
});

const STARTUP_DESTINATIONS: readonly StartupDestination[] = [
  "library-all",
  "library-today",
  "library-favorites",
  "review",
  "scan",
];

const LIBRARY_VIEW_MODES: readonly LibraryViewMode[] = ["grid", "list"];

const LIBRARY_SORTS: readonly ClipSort[] = [
  "modified-desc",
  "modified-asc",
  "size-desc",
  "size-asc",
  "name-asc",
];

const MOTION_MODES: readonly MotionMode[] = ["system", "reduced"];

export function createDefaultAppPreferences(): AppPreferencesV1 {
  return { ...DEFAULT_APP_PREFERENCES };
}

/**
 * Parses the persisted v1 payload. A mismatched schema invalidates the whole
 * payload; otherwise each field is validated independently.
 */
export function parseAppPreferences(serialized: string | null): AppPreferencesV1 {
  if (serialized === null) return createDefaultAppPreferences();

  try {
    return normalizeAppPreferences(JSON.parse(serialized));
  } catch {
    return createDefaultAppPreferences();
  }
}

export function normalizeAppPreferences(value: unknown): AppPreferencesV1 {
  if (!isRecord(value) || value.schemaVersion !== APP_PREFERENCES_SCHEMA_VERSION) {
    return createDefaultAppPreferences();
  }

  return {
    schemaVersion: APP_PREFERENCES_SCHEMA_VERSION,
    startupDestination: isOneOf(value.startupDestination, STARTUP_DESTINATIONS)
      ? value.startupDestination
      : DEFAULT_APP_PREFERENCES.startupDestination,
    libraryViewMode: isOneOf(value.libraryViewMode, LIBRARY_VIEW_MODES)
      ? value.libraryViewMode
      : DEFAULT_APP_PREFERENCES.libraryViewMode,
    librarySort: isOneOf(value.librarySort, LIBRARY_SORTS)
      ? value.librarySort
      : DEFAULT_APP_PREFERENCES.librarySort,
    previewVolumePercent: isVolumePercent(value.previewVolumePercent)
      ? value.previewVolumePercent
      : DEFAULT_APP_PREFERENCES.previewVolumePercent,
    previewMuted: isBoolean(value.previewMuted)
      ? value.previewMuted
      : DEFAULT_APP_PREFERENCES.previewMuted,
    reviewAutoplay: isBoolean(value.reviewAutoplay)
      ? value.reviewAutoplay
      : DEFAULT_APP_PREFERENCES.reviewAutoplay,
    motionMode: isOneOf(value.motionMode, MOTION_MODES)
      ? value.motionMode
      : DEFAULT_APP_PREFERENCES.motionMode,
    automaticUpdateCheck: isBoolean(value.automaticUpdateCheck)
      ? value.automaticUpdateCheck
      : DEFAULT_APP_PREFERENCES.automaticUpdateCheck,
  };
}

/** Keeps the current value when an untyped caller supplies an invalid patch. */
export function applyAppPreferencesPatch(
  current: AppPreferencesV1,
  patch: AppPreferencesPatch,
): AppPreferencesV1 {
  return {
    schemaVersion: APP_PREFERENCES_SCHEMA_VERSION,
    startupDestination: isOneOf(patch.startupDestination, STARTUP_DESTINATIONS)
      ? patch.startupDestination
      : current.startupDestination,
    libraryViewMode: isOneOf(patch.libraryViewMode, LIBRARY_VIEW_MODES)
      ? patch.libraryViewMode
      : current.libraryViewMode,
    librarySort: isOneOf(patch.librarySort, LIBRARY_SORTS)
      ? patch.librarySort
      : current.librarySort,
    previewVolumePercent: isVolumePercent(patch.previewVolumePercent)
      ? patch.previewVolumePercent
      : current.previewVolumePercent,
    previewMuted: isBoolean(patch.previewMuted)
      ? patch.previewMuted
      : current.previewMuted,
    reviewAutoplay: isBoolean(patch.reviewAutoplay)
      ? patch.reviewAutoplay
      : current.reviewAutoplay,
    motionMode: isOneOf(patch.motionMode, MOTION_MODES)
      ? patch.motionMode
      : current.motionMode,
    automaticUpdateCheck: isBoolean(patch.automaticUpdateCheck)
      ? patch.automaticUpdateCheck
      : current.automaticUpdateCheck,
  };
}

export type AppPreferencesLoadResult = {
  preferences: AppPreferencesV1;
  storageError: string | null;
};

export function loadAppPreferences(
  storage: Pick<AppPreferencesStorage, "getItem"> | null,
): AppPreferencesLoadResult {
  if (storage === null) {
    return { preferences: createDefaultAppPreferences(), storageError: null };
  }

  try {
    return {
      preferences: parseAppPreferences(storage.getItem(APP_PREFERENCES_STORAGE_KEY)),
      storageError: null,
    };
  } catch {
    return {
      preferences: createDefaultAppPreferences(),
      storageError: APP_PREFERENCES_STORAGE_ERRORS.read,
    };
  }
}

export function saveAppPreferences(
  storage: Pick<AppPreferencesStorage, "setItem"> | null,
  preferences: AppPreferencesV1,
): string | null {
  if (storage === null) return null;

  try {
    storage.setItem(APP_PREFERENCES_STORAGE_KEY, JSON.stringify(preferences));
    return null;
  } catch {
    return APP_PREFERENCES_STORAGE_ERRORS.write;
  }
}

export function removeAppPreferences(
  storage: Pick<AppPreferencesStorage, "removeItem"> | null,
): string | null {
  if (storage === null) return null;

  try {
    storage.removeItem(APP_PREFERENCES_STORAGE_KEY);
    return null;
  } catch {
    return APP_PREFERENCES_STORAGE_ERRORS.remove;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

function isVolumePercent(value: unknown): value is number {
  return Number.isInteger(value) && typeof value === "number" && value >= 0 && value <= 100;
}

function isOneOf<T extends string>(
  value: unknown,
  allowedValues: readonly T[],
): value is T {
  return typeof value === "string" && allowedValues.includes(value as T);
}
