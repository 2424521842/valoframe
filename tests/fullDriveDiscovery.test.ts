import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const backendSource = readFileSync(
  new URL("../src/api/backend.ts", import.meta.url),
  "utf8",
);
const appSource = readFileSync(
  new URL("../src/App.tsx", import.meta.url),
  "utf8",
);
const scanControllerSource = readFileSync(
  new URL("../src/hooks/useScanController.ts", import.meta.url),
  "utf8",
);

test("full-drive discovery invokes the Tauri command", () => {
  assert.match(backendSource, /invoke<ScanJobResult<FullDriveScanResult>>\("discover_and_scan_fixed_drives"\)/);
});

test("full-drive discovery is wired to the independent sidebar action", () => {
  assert.match(scanControllerSource, /await discoverAndScanFixedDrives\(\)/);
  assert.match(appSource, /discoverAll: handleDiscoverAll/);
  assert.match(appSource, /onDiscoverAll=\{handleDiscoverAll\}/);
});
