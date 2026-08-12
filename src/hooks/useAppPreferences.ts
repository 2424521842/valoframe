import { useCallback, useRef, useState } from "react";
import {
  APP_PREFERENCES_STORAGE_ERRORS,
  applyAppPreferencesPatch,
  createDefaultAppPreferences,
  loadAppPreferences,
  removeAppPreferences,
  saveAppPreferences,
  type AppPreferencesPatch,
  type AppPreferencesStorage,
  type AppPreferencesV1,
} from "../lib/appPreferences";

export type AppPreferencesController = {
  preferences: AppPreferencesV1;
  storageError: string | null;
  updatePreferences: (patch: AppPreferencesPatch) => void;
  resetPreferences: () => void;
};

export type UseAppPreferencesOptions = {
  /** Primarily useful for tests or non-browser hosts. Null disables persistence. */
  storage?: AppPreferencesStorage | null;
};

type StorageResolver = () => AppPreferencesStorage | null;

export function useAppPreferences(
  options: UseAppPreferencesOptions = {},
): AppPreferencesController {
  const resolverRef = useRef<StorageResolver | null>(null);
  if (resolverRef.current === null) {
    resolverRef.current = createStorageResolver(options.storage);
  }

  const [initialState] = useState(() => loadFromResolver(resolverRef.current!));
  const [preferences, setPreferences] = useState(initialState.preferences);
  const [storageError, setStorageError] = useState<string | null>(
    initialState.storageError,
  );
  const preferencesRef = useRef(preferences);

  const updatePreferences = useCallback((patch: AppPreferencesPatch) => {
    const nextPreferences = applyAppPreferencesPatch(
      preferencesRef.current,
      patch,
    );
    preferencesRef.current = nextPreferences;
    setPreferences(nextPreferences);
    setStorageError(saveToResolver(resolverRef.current!, nextPreferences));
  }, []);

  const resetPreferences = useCallback(() => {
    const nextPreferences = createDefaultAppPreferences();
    preferencesRef.current = nextPreferences;
    setPreferences(nextPreferences);
    setStorageError(removeFromResolver(resolverRef.current!));
  }, []);

  return {
    preferences,
    storageError,
    updatePreferences,
    resetPreferences,
  };
}

function createStorageResolver(
  storageOverride: AppPreferencesStorage | null | undefined,
): StorageResolver {
  if (storageOverride !== undefined) return () => storageOverride;

  return () => {
    if (typeof window === "undefined") return null;
    return window.localStorage;
  };
}

function loadFromResolver(resolver: StorageResolver) {
  try {
    return loadAppPreferences(resolver());
  } catch {
    return {
      preferences: createDefaultAppPreferences(),
      storageError: APP_PREFERENCES_STORAGE_ERRORS.read,
    };
  }
}

function saveToResolver(
  resolver: StorageResolver,
  preferences: AppPreferencesV1,
): string | null {
  try {
    return saveAppPreferences(resolver(), preferences);
  } catch {
    return APP_PREFERENCES_STORAGE_ERRORS.write;
  }
}

function removeFromResolver(resolver: StorageResolver): string | null {
  try {
    return removeAppPreferences(resolver());
  } catch {
    return APP_PREFERENCES_STORAGE_ERRORS.remove;
  }
}
