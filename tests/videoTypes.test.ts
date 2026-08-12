import assert from "node:assert/strict";
import test from "node:test";
import {
  absoluteTimelineCompilationMode,
  expectsOfficialRoundScore,
  matchesVideoType,
  previewTimelineMarkerMode,
  videoTypeLabel,
  VIDEO_TYPE_FILTERS,
} from "../src/lib/videoTypes.ts";

test("video types use the requested product order and labels", () => {
  assert.deepEqual(VIDEO_TYPE_FILTERS, [
    "triple",
    "quad",
    "five",
    "six",
    "kill-compilation",
    "death",
  ]);
  assert.deepEqual(
    VIDEO_TYPE_FILTERS.map(videoTypeLabel),
    ["三杀时刻", "四杀时刻", "五杀时刻", "六杀时刻", "击杀集锦", "死亡时刻"],
  );
});

test("five-kill and six-kill moments are separate video types", () => {
  const five = { highlightType: 10, killCount: 5 };
  const six = { highlightType: 10, killCount: 6 };

  assert.equal(matchesVideoType(five, "five"), true);
  assert.equal(matchesVideoType(five, "six"), false);
  assert.equal(matchesVideoType(six, "five"), false);
  assert.equal(matchesVideoType(six, "six"), true);
});

test("custom tag text does not participate in video type classification", () => {
  const customTagOnly = {
    officialVideoName: "",
    officialVideoType: "",
    highlightType: null,
    killCount: null,
    extractedText: "",
  };

  assert.equal(matchesVideoType(customTagOnly, "triple"), false);
});

test("compilation and death metadata map to independent video types", () => {
  assert.equal(matchesVideoType({ highlightType: 2 }, "kill-compilation"), true);
  assert.equal(matchesVideoType({ officialVideoName: "死亡集锦" }, "death"), true);
  assert.equal(matchesVideoType({ highlightType: 3 }, "kill-compilation"), false);
});

test("absolute compilation classification stays separate from marker eligibility", () => {
  assert.equal(previewTimelineMarkerMode({ highlightType: 2 }), "kill");
  assert.equal(previewTimelineMarkerMode({ highlightType: 3 }), "death");
  assert.equal(previewTimelineMarkerMode({ officialVideoType: "KILL COMPILATION" }), "kill");
  assert.equal(previewTimelineMarkerMode({ officialVideoName: "死亡集锦" }), "death");
  assert.equal(previewTimelineMarkerMode({ officialVideoName: "普通素材" }), null);
  assert.equal(
    previewTimelineMarkerMode({ officialVideoName: "击杀集锦 / 死亡集锦" }),
    null,
  );
  assert.equal(
    previewTimelineMarkerMode({ highlightType: 2, officialVideoName: "死亡集锦" }),
    "kill",
  );

  assert.equal(absoluteTimelineCompilationMode({ highlightType: 2 }), "kill");
  assert.equal(absoluteTimelineCompilationMode({ highlightType: 3 }), "death");
  assert.equal(
    absoluteTimelineCompilationMode({ officialVideoName: "击杀集锦 / 死亡集锦" }),
    null,
  );
});

test("ordinary multi-kills expose kill markers without entering compilation filters", () => {
  const ordinaryMultiKills = [
    { highlightType: 4 },
    { highlightType: 6 },
    { highlightType: 10 },
    ...[3, 4, 5, 6].map((killCount) => ({ killCount })),
    ...["三杀时刻", "四杀时刻", "五杀时刻", "六杀时刻"].map(
      (officialVideoName) => ({ officialVideoName }),
    ),
  ];

  for (const clip of ordinaryMultiKills) {
    assert.equal(previewTimelineMarkerMode(clip), "kill");
    assert.equal(absoluteTimelineCompilationMode(clip), null);
    assert.equal(matchesVideoType(clip, "kill-compilation"), false);
    assert.equal(matchesVideoType(clip, "death"), false);
  }
});

test("official round scores are expected only for supported modes and kill moments", () => {
  for (const gameMode of ["普通模式", "极速模式", "竞技模式"]) {
    assert.equal(
      expectsOfficialRoundScore({ officialVideoType: "三杀时刻", gameMode }),
      true,
    );
  }

  assert.equal(
    expectsOfficialRoundScore({ officialVideoType: "三杀时刻", gameMode: "夺还模式" }),
    false,
  );
  assert.equal(
    expectsOfficialRoundScore({ officialVideoType: "三杀时刻", gameMode: "未评级" }),
    false,
  );
  assert.equal(
    expectsOfficialRoundScore({ officialVideoType: "三杀时刻", gameMode: null }),
    true,
  );
});

test("official non-scoring video types never receive a missing-score placeholder", () => {
  for (const officialVideoType of ["击杀集锦", "死亡集锦", "夜市翻牌"]) {
    assert.equal(
      expectsOfficialRoundScore({
        officialVideoType,
        highlightType: 10,
        gameMode: "竞技模式",
      }),
      false,
    );
  }

  assert.equal(
    expectsOfficialRoundScore({ officialVideoType: "精彩回放", gameMode: "竞技模式" }),
    false,
  );
});

test("legacy numeric official types and highlight types retain score eligibility", () => {
  for (const highlightType of [4, 6, 10]) {
    assert.equal(
      expectsOfficialRoundScore({ highlightType, gameMode: "竞技模式" }),
      true,
    );
  }

  assert.equal(
    expectsOfficialRoundScore({ officialVideoType: "10", gameMode: "竞技模式" }),
    true,
  );
  assert.equal(
    expectsOfficialRoundScore({
      officialVideoType: "高光时刻",
      highlightType: 10,
      gameMode: "竞技模式",
    }),
    true,
  );
});
