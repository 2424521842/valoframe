import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("Tauri window permits the compact desktop layout", () => {
  const config = JSON.parse(
    readFileSync(
      new URL("../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  );

  assert.equal(config.app.windows[0].minWidth, 760);
  assert.equal(config.app.windows[0].minHeight, 560);
});
