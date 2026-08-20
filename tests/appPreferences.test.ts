import assert from "node:assert/strict";
import test from "node:test";
import {
  APP_PREFERENCES_STORAGE_ERRORS,
  APP_PREFERENCES_STORAGE_KEY,
  DEFAULT_APP_PREFERENCES,
  createDefaultAppPreferences,
  loadAppPreferences,
  normalizeAppPreferences,
  parseAppPreferences,
  removeAppPreferences,
  saveAppPreferences,
  type AppPreferencesV1,
} from "../src/lib/appPreferences.ts";

const VALID_NON_DEFAULT_PREFERENCES: AppPreferencesV1 = {
  schemaVersion: 1,
  startupDestination: "scan",
  libraryViewMode: "list",
  librarySort: "name-asc",
  previewVolumePercent: 36,
  previewMuted: true,
  reviewAutoplay: false,
  motionMode: "reduced",
  scanOnStartup: true,
  automaticUpdateCheck: false,
  feedbackEndpoint: "https://feedback.example.com/api",
};

test("app preferences expose the documented v1 defaults", () => {
  assert.deepEqual(DEFAULT_APP_PREFERENCES, {
    schemaVersion: 1,
    startupDestination: "library-all",
    libraryViewMode: "grid",
    librarySort: "modified-desc",
    previewVolumePercent: 100,
    previewMuted: false,
    reviewAutoplay: true,
    motionMode: "system",
    scanOnStartup: false,
    automaticUpdateCheck: true,
    feedbackEndpoint: "",
  });

  const first = createDefaultAppPreferences();
  const second = createDefaultAppPreferences();
  assert.notEqual(first, second);
});

test("valid v1 preferences survive parsing and unknown fields are ignored", () => {
  const parsed = normalizeAppPreferences({
    ...VALID_NON_DEFAULT_PREFERENCES,
    futureField: "ignored",
  });

  assert.deepEqual(parsed, VALID_NON_DEFAULT_PREFERENCES);
  assert.equal("futureField" in parsed, false);
});

test("older v1 payloads default only the newer fields", () => {
  const {
    scanOnStartup: _missingScan,
    feedbackEndpoint: _missingEndpoint,
    ...olderPayload
  } = VALID_NON_DEFAULT_PREFERENCES;

  assert.deepEqual(normalizeAppPreferences(olderPayload), {
    ...VALID_NON_DEFAULT_PREFERENCES,
    scanOnStartup: false,
    feedbackEndpoint: "",
  });
});

test("each invalid v1 field falls back independently", () => {
  const invalidCases: Array<[keyof AppPreferencesV1, unknown]> = [
    ["startupDestination", "dashboard"],
    ["libraryViewMode", "tiles"],
    ["librarySort", "random"],
    ["previewVolumePercent", -1],
    ["previewVolumePercent", 101],
    ["previewVolumePercent", 2.5],
    ["previewMuted", 1],
    ["reviewAutoplay", "true"],
    ["motionMode", "always"],
    ["scanOnStartup", "true"],
    ["automaticUpdateCheck", null],
    ["feedbackEndpoint", 123],
    ["feedbackEndpoint", "x".repeat(301)],
  ];

  for (const [field, invalidValue] of invalidCases) {
    const parsed = normalizeAppPreferences({
      ...VALID_NON_DEFAULT_PREFERENCES,
      [field]: invalidValue,
    });
    const expected = {
      ...VALID_NON_DEFAULT_PREFERENCES,
      [field]: DEFAULT_APP_PREFERENCES[field],
    };
    assert.deepEqual(parsed, expected, `invalid ${field} should use its default`);
  }
});

test("an unknown or missing schema and malformed JSON use all defaults", () => {
  assert.deepEqual(
    normalizeAppPreferences({ ...VALID_NON_DEFAULT_PREFERENCES, schemaVersion: 2 }),
    DEFAULT_APP_PREFERENCES,
  );
  assert.deepEqual(
    normalizeAppPreferences({ ...VALID_NON_DEFAULT_PREFERENCES, schemaVersion: "1" }),
    DEFAULT_APP_PREFERENCES,
  );
  assert.deepEqual(normalizeAppPreferences(null), DEFAULT_APP_PREFERENCES);
  assert.deepEqual(parseAppPreferences("{broken"), DEFAULT_APP_PREFERENCES);
  assert.deepEqual(parseAppPreferences(null), DEFAULT_APP_PREFERENCES);
});

test("storage helpers use only the versioned preference key", () => {
  const values = new Map<string, string>();
  const touched: string[] = [];
  const storage = {
    getItem(key: string) {
      touched.push(`get:${key}`);
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      touched.push(`set:${key}`);
      values.set(key, value);
    },
    removeItem(key: string) {
      touched.push(`remove:${key}`);
      values.delete(key);
    },
  };

  assert.equal(saveAppPreferences(storage, VALID_NON_DEFAULT_PREFERENCES), null);
  assert.deepEqual(loadAppPreferences(storage), {
    preferences: VALID_NON_DEFAULT_PREFERENCES,
    storageError: null,
  });
  assert.equal(removeAppPreferences(storage), null);
  assert.deepEqual(touched, [
    `set:${APP_PREFERENCES_STORAGE_KEY}`,
    `get:${APP_PREFERENCES_STORAGE_KEY}`,
    `remove:${APP_PREFERENCES_STORAGE_KEY}`,
  ]);
});

test("storage helper failures return actionable errors instead of throwing", () => {
  assert.deepEqual(
    loadAppPreferences({
      getItem() {
        throw new Error("blocked");
      },
    }),
    {
      preferences: DEFAULT_APP_PREFERENCES,
      storageError: APP_PREFERENCES_STORAGE_ERRORS.read,
    },
  );

  assert.equal(
    saveAppPreferences(
      {
        setItem() {
          throw new Error("quota exceeded");
        },
      },
      VALID_NON_DEFAULT_PREFERENCES,
    ),
    APP_PREFERENCES_STORAGE_ERRORS.write,
  );
  assert.equal(
    removeAppPreferences({
      removeItem() {
        throw new Error("blocked");
      },
    }),
    APP_PREFERENCES_STORAGE_ERRORS.remove,
  );
});
