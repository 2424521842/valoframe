import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const scanWorkspaceSource = readFileSync(
  new URL("../src/screens/ScanWorkspace.tsx", import.meta.url),
  "utf8",
);
const activeFiltersModule = await import("../src/lib/activeFilters.ts");

test("mode transitions preserve status except when entering missing", () => {
  const transitionLibraryMode = Reflect.get(
    activeFiltersModule,
    "transitionLibraryMode",
  ) as ((fileStatus: string, mode: string) => unknown) | undefined;

  assert.equal(typeof transitionLibraryMode, "function");
  assert.deepEqual(transitionLibraryMode?.("available", "favorites"), {
    libraryMode: "favorites",
    fileStatus: "available",
  });
  assert.deepEqual(transitionLibraryMode?.("available", "missing"), {
    libraryMode: "missing",
    fileStatus: "all",
  });
});

test("legacy file-status transition remains valid but is no longer a primary controller", () => {
  const transitionFileStatus = Reflect.get(
    activeFiltersModule,
    "transitionFileStatus",
  ) as ((libraryMode: string, fileStatus: string) => unknown) | undefined;

  assert.equal(typeof transitionFileStatus, "function");
  const missingState = (
    activeFiltersModule.transitionLibraryMode("available", "missing")
  );
  assert.deepEqual(
    transitionFileStatus?.(missingState.libraryMode, "available"),
    { libraryMode: "all", fileStatus: "available" },
  );
  assert.deepEqual(transitionFileStatus?.("missing", "all"), {
    libraryMode: "missing",
    fileStatus: "all",
  });
  assert.doesNotMatch(app, /transitionFileStatus\(libraryMode, nextFileStatus\)/);
  assert.match(app, /highlightFilter/);
  assert.match(app, /dateRangeForPreset/);
});

test("account, agent, map, and mode filters remain independently composable", () => {
  const accountHandler = extractFunction(app, "handleAccountChange");
  const agentHandler = extractFunction(app, "handleAgentChange");
  const mapHandler = extractFunction(app, "handleMapChange");
  const gameModeHandler = extractFunction(app, "handleGameModeChange");

  assert.doesNotMatch(accountHandler, /setSelectedAgentName|setSelectedMapName|setSelectedGameMode/);
  assert.doesNotMatch(agentHandler, /setSelectedAccountId|setSelectedMapName|setSelectedGameMode/);
  assert.doesNotMatch(mapHandler, /setSelectedAccountId|setSelectedAgentName|setSelectedGameMode/);
  assert.doesNotMatch(gameModeHandler, /setSelectedAccountId|setSelectedAgentName|setSelectedMapName/);
});

test("App owns only the compact navigation drawer", () => {
  assert.match(app, /const \[isSidebarOpen, setIsSidebarOpen\] = useState\(false\)/);
  assert.match(app, /useMediaQuery\("\(max-width: 919px\)"\)/);
  assert.doesNotMatch(app, /<DetailPanel/);
  assert.doesNotMatch(app, /DrawerState|activeDrawer|open-detail|close-detail/);
  assert.match(app, /menuTriggerRef\.current\?\.focus\(\)/);
  assert.match(app, /const hasSidebarOverlay = isSidebarOverlay && isSidebarOpen/);
});

test("scan page owns a staged directory queue and visible failure state", () => {
  assert.match(app, /scanTargetFromPath\(path\)/);
  assert.match(app, /setManualScanTargets/);
  assert.match(scanWorkspaceSource, /待扫描目录/);
  assert.match(scanWorkspaceSource, /从扫描队列移除/);
  assert.match(scanWorkspaceSource, /role="alert"/);
});

function extractFunction(source: string, name: string): string {
  const start = source.indexOf(`const ${name} =`);
  assert.notEqual(start, -1, `missing function ${name}`);
  const nextFunction = source.indexOf("\n\n  const ", start + 1);
  const blockEnd = source.indexOf("\n  };", start);
  const end = [nextFunction, blockEnd]
    .filter((index) => index !== -1)
    .sort((left, right) => left - right)[0];
  assert.notEqual(end, -1, `missing function end ${name}`);
  return source.slice(start, end);
}
