import assert from "node:assert/strict";
import test from "node:test";
import { agentChipDisplay } from "../src/lib/agentChip.ts";

test("agent chip uses avatar url until the image fails", () => {
  assert.deepEqual(
    agentChipDisplay("芮娜", "https://assets.example/reyna.png", false),
    {
      kind: "image",
      label: "芮娜",
      url: "https://assets.example/reyna.png",
    },
  );

  assert.deepEqual(
    agentChipDisplay("芮娜", "https://assets.example/reyna.png", true),
    {
      kind: "text",
      label: "芮娜",
      url: "",
    },
  );
});

test("agent chip falls back to placeholder initials without an avatar", () => {
  assert.deepEqual(agentChipDisplay("", "", false), {
    kind: "text",
    label: "??",
    url: "",
  });
});
