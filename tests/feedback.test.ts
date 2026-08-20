import assert from "node:assert/strict";
import test from "node:test";
import {
  FEEDBACK_CATEGORY_OPTIONS,
  isValidFeedbackEndpoint,
  normalizeFeedbackEndpoint,
} from "../src/lib/feedback.ts";

test("feedback categories cover every wire value with distinct labels", () => {
  const values = FEEDBACK_CATEGORY_OPTIONS.map((option) => option.value);
  assert.deepEqual(values, ["mismatch", "playback", "metadata", "other"]);
  assert.equal(new Set(values).size, values.length);
  for (const option of FEEDBACK_CATEGORY_OPTIONS) {
    assert.ok(option.label.trim().length > 0);
    assert.ok(option.hint.trim().length > 0);
  }
});

test("endpoint normalization trims and caps length", () => {
  assert.equal(normalizeFeedbackEndpoint(""), "");
  assert.equal(normalizeFeedbackEndpoint("  https://a.b/c  "), "https://a.b/c");
  assert.equal(normalizeFeedbackEndpoint("https://a.b/" + "x".repeat(400)).length, 300);
});

test("endpoint validation allows empty, https, and localhost http only", () => {
  assert.equal(isValidFeedbackEndpoint(""), true);
  assert.equal(isValidFeedbackEndpoint("   "), true);
  assert.equal(isValidFeedbackEndpoint("https://example.com/api"), true);
  assert.equal(isValidFeedbackEndpoint("http://localhost:8080/feedback"), true);
  assert.equal(isValidFeedbackEndpoint("http://127.0.0.1/feedback"), true);
  assert.equal(isValidFeedbackEndpoint("http://[::1]/feedback"), true);
  assert.equal(isValidFeedbackEndpoint("http://example.com/feedback"), false);
  assert.equal(isValidFeedbackEndpoint("ftp://example.com"), false);
  assert.equal(isValidFeedbackEndpoint("https://"), false);
  assert.equal(isValidFeedbackEndpoint("https://a.b"), true);
});
