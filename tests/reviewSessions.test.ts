import assert from "node:assert/strict";
import test from "node:test";
import {
  createReviewSession,
  reviewSessionCounts,
  reviewSessionQueryKey,
} from "../src/lib/reviewSessions.ts";
import type { ReviewSessionFilters } from "../src/types.ts";

const filters: ReviewSessionFilters = {
  query: {
    accountId: "winter",
    agentName: "尚勃勒",
    mapName: "隐世修所",
    gameMode: "竞技模式",
    sortBy: "modified-desc",
  },
  labels: ["账号：Winter", "英雄：尚勃勒", "地图：隐世修所", "模式：竞技模式"],
  sort: "latest",
  candidateScope: "all",
};

test("creates an independent quick-pick session with only item decisions", () => {
  const session = createReviewSession(filters, ["clip-1", "clip-2"]);

  assert.equal(session.status, "active");
  assert.equal(session.currentIndex, 0);
  assert.equal(session.totalCount, 2);
  assert.deepEqual(session.items, [
    { videoId: "clip-1", decision: "unreviewed" },
    { videoId: "clip-2", decision: "unreviewed" },
  ]);
  assert.deepEqual(session.filters, filters);
  assert.notEqual(session.filters, filters);
  assert.notEqual(session.filters.labels, filters.labels);
});

test("derives review counts from decisions without favorite, tag, or trash state", () => {
  const counts = reviewSessionCounts({
    totalCount: 5,
    items: [
      { videoId: "1", decision: "selected" },
      { videoId: "2", decision: "selected" },
      { videoId: "3", decision: "pending" },
      { videoId: "4", decision: "skipped" },
      { videoId: "5", decision: "unreviewed" },
    ],
  });

  assert.deepEqual(counts, {
    total: 5,
    reviewed: 4,
    selected: 2,
    pending: 1,
    skipped: 1,
    remaining: 1,
  });
});

test("uses a stable inherited library query as its resume key", () => {
  const query = {
    sortBy: "modified-desc" as const,
    gameMode: "竞技模式",
    mapName: "隐世修所",
    agentName: "尚勃勒",
    accountId: "winter",
  };
  assert.equal(
    reviewSessionQueryKey(filters.query),
    reviewSessionQueryKey(query),
  );
  assert.doesNotMatch(reviewSessionQueryKey(query), /selected|pending|skipped/);
});
