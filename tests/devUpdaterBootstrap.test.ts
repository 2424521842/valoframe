import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const script = readFileSync(new URL("../start-dev.bat", import.meta.url), "utf8");

test("local desktop dev startup embeds only the ignored updater public key when available", () => {
  assert.match(
    script,
    /release-secrets\\valoframe-updater\.key\.pub/u,
  );
  assert.match(script, /if defined VALOFRAME_UPDATER_PUBLIC_KEY/u);
  assert.match(
    script,
    /set \/p "VALOFRAME_UPDATER_PUBLIC_KEY="<"!UPDATER_PUBLIC_KEY_FILE!"/u,
  );
  assert.match(script, /updater will remain disabled/u);
  assert.doesNotMatch(
    script,
    /TAURI_SIGNING_PRIVATE_KEY(?:_PASSWORD)?|valoframe-updater\.key(?!\.pub)/u,
  );
  assert.doesNotMatch(script, /echo .*VALOFRAME_UPDATER_PUBLIC_KEY/u);
});
