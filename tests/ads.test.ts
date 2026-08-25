import assert from "node:assert/strict";
import { test } from "node:test";

import {
  activeCreatives,
  parseAllowedHosts,
  selectCreative,
  type AdCreative,
} from "../src/lib/ads.ts";

function creative(overrides: Partial<AdCreative> = {}): AdCreative {
  return {
    creativeId: "cr-001",
    title: "标题",
    body: null,
    advertiserName: "广告主",
    weight: 100,
    startAt: null,
    endAt: null,
    imagePath: "ad/cr-001",
    ...overrides,
  };
}

test("activeCreatives keeps creatives with no flight window", () => {
  const creatives = [creative()];
  assert.equal(activeCreatives(creatives, new Date("2026-08-23T00:00:00Z")).length, 1);
});

test("activeCreatives excludes creatives outside their flight window", () => {
  const creatives = [
    creative({ creativeId: "past", endAt: "2026-08-01T00:00:00Z" }),
    creative({ creativeId: "future", startAt: "2026-09-01T00:00:00Z" }),
    creative({
      creativeId: "current",
      startAt: "2026-08-01T00:00:00Z",
      endAt: "2026-08-31T00:00:00Z",
    }),
  ];

  const active = activeCreatives(creatives, new Date("2026-08-23T00:00:00Z"));
  assert.deepEqual(
    active.map((item) => item.creativeId),
    ["current"],
  );
});

test("activeCreatives ignores unparseable timestamps rather than hiding the creative", () => {
  const creatives = [creative({ startAt: "not-a-date", endAt: "also-bad" })];
  assert.equal(activeCreatives(creatives, new Date("2026-08-23T00:00:00Z")).length, 1);
});

test("selectCreative returns null when there is nothing to show", () => {
  assert.equal(selectCreative([], 0), null);
});

test("selectCreative rotates deterministically across the weighted list", () => {
  const creatives = [
    creative({ creativeId: "a", weight: 100 }),
    creative({ creativeId: "b", weight: 100 }),
  ];

  // Equal weights expand to 10 slots each, so the first ten indices stay on "a".
  assert.equal(selectCreative(creatives, 0)?.creativeId, "a");
  assert.equal(selectCreative(creatives, 9)?.creativeId, "a");
  assert.equal(selectCreative(creatives, 10)?.creativeId, "b");
  // The index wraps rather than falling off the end.
  assert.equal(selectCreative(creatives, 20)?.creativeId, "a");
});

test("selectCreative tolerates hostile weights from a vendor manifest", () => {
  const creatives = [
    creative({ creativeId: "a", weight: Number.NaN }),
    creative({ creativeId: "b", weight: -5 }),
    creative({ creativeId: "c", weight: 10_000_000 }),
  ];

  assert.notEqual(selectCreative(creatives, 0), null);
  assert.notEqual(selectCreative(creatives, Number.NaN), null);
  assert.notEqual(selectCreative(creatives, -3), null);
});

test("parseAllowedHosts normalizes separators, schemes, and paths", () => {
  assert.deepEqual(
    parseAllowedHosts("https://ad.example.com/lp, LP.Example.com  other.example.com"),
    ["ad.example.com", "lp.example.com", "other.example.com"],
  );
});

test("parseAllowedHosts drops entries that are not hostnames", () => {
  assert.deepEqual(parseAllowedHosts("localhost, ad.example.com, 中文, a_b.com"), [
    "ad.example.com",
  ]);
});

test("parseAllowedHosts deduplicates and returns empty for blank input", () => {
  assert.deepEqual(parseAllowedHosts("ad.example.com ad.example.com"), [
    "ad.example.com",
  ]);
  assert.deepEqual(parseAllowedHosts("   "), []);
});
