import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const app = source("../src/App.tsx");
const scanController = source("../src/hooks/useScanController.ts");
const scan = source("../src/screens/ScanWorkspace.tsx");
const preview = source("../src/screens/PreviewWorkspace.tsx");
const matchLibrary = source("../src/components/MatchLibrary.tsx");
const libraryToolbar = source("../src/components/LibraryToolbar.tsx");
const select = source("../src/components/ui/select.tsx");
const command = source("../src/components/ui/command.tsx");
const contextMenu = source("../src/components/ui/context-menu.tsx");
const checkbox = source("../src/components/ui/checkbox.tsx");
const dialog = source("../src/components/ui/dialog.tsx");
const alertDialog = source("../src/components/ui/alert-dialog.tsx");
const sidebar = source("../src/components/CinematicSidebar.tsx");
const libraryWorkspace = source("../src/screens/LibraryWorkspace.tsx");
const backend = source("../src/api/backend.ts");
const css = source("../src/cinematic.css");
const mockData = source("../src/data/mockData.ts");

test("the production app exposes the scan, library, and preview workspaces", () => {
  assert.match(app, /useState<AppScreen>\("library"\)/);
  assert.match(app, /lazy\(\(\) =>\s*import\("\.\/screens\/LibraryWorkspace"\)/s);
  assert.match(app, /lazy\(\(\) =>\s*import\("\.\/screens\/PreviewWorkspace"\)/s);
  assert.match(app, /<Suspense fallback=\{<WorkspaceLoading \/>\}>/);
  assert.match(app, /activeScreen === "library"/);
  assert.match(app, /activeScreen === "scan"/);
  assert.match(app, /<PreviewWorkspace/);
  assert.match(app, /<ScanWorkspace/);
  assert.match(app, /<LibraryWorkspace/);
  assert.match(app, /<CinematicSidebar/);
  assert.match(app, /setActiveScreen\("preview"\)/);
  assert.match(app, /groupClipsByMatch\(visibleClips\)/);
});

test("the migration replaces the old view controller with the requested workflow", () => {
  assert.match(app, /manualScanTargets/);
  assert.match(app, /mergeScanTargets\(sourceDirs, manualScanTargets, excludedScanPaths\)/);
  assert.match(app, /sourcePaths: scanTargets\.map\(\(target\) => target\.path\)/);
  assert.match(scanController, /scanRoots\(paths\)/);
  assert.doesNotMatch(app, /for \(const \[index, target\] of scanTargets\.entries\(\)\)/);
  assert.match(app, /highlightFilter/);
  assert.match(app, /dateRangeForPreset/);
  assert.match(libraryToolbar, /全局搜索素材/);
  assert.match(libraryToolbar, /视频类型/);
  assert.match(libraryToolbar, /网格视图/);
  assert.match(libraryToolbar, /列表视图/);
  assert.match(matchLibrary, /virtualRows\.map/);
  assert.match(matchLibrary, /match-board-header/);
});

test("the library search and wide match headers keep a stable single-row layout", () => {
  assert.doesNotMatch(libraryToolbar, /<span className="sr-only">全局搜索<\/span>/);
  assert.match(libraryToolbar, /<MagnifyingGlass[^>]*weight="bold" \/>\s*<UiCommandInput/);
  assert.match(matchLibrary, /className="match-board-count"/);
  assert.match(css, /minmax\(110px, 180px\).*minmax\(90px, 140px\).*minmax\(72px, 1fr\)/s);
  assert.match(css, /\.match-board-count\s*\{[^}]*justify-self:\s*end;/s);
});

test("sidebar metadata stays on one row and the retired smart-filter entry is absent", () => {
  assert.match(sidebar, /className="cinematic-sidebar-item-meta"/);
  assert.match(css, /\.cinematic-sidebar-item-meta\s*\{[^}]*display:\s*flex;/s);
  assert.doesNotMatch(sidebar, /智能筛选|onOpenSmartFilters|<Funnel/);
});

test("library controls use editable shadcn-style Radix primitives instead of native menus", () => {
  assert.match(libraryToolbar, /UiSelect/);
  assert.match(libraryToolbar, /UiPopover/);
  assert.match(libraryToolbar, /UiCommandInput/);
  assert.match(libraryToolbar, /UiTooltip/);
  assert.doesNotMatch(libraryToolbar, /<select|<option/);
  assert.match(select, /Select as SelectPrimitive.*from "radix-ui"/s);
  assert.match(command, /Command as CommandPrimitive.*from "cmdk"/s);
  assert.match(css, /\.ui-select-content/);
  assert.match(css, /\.library-search-popover/);
});

test("clip context menu reuses the real preview and file actions", () => {
  assert.match(matchLibrary, /UiContextMenuTrigger asChild/);
  assert.match(matchLibrary, /预览素材/);
  assert.match(matchLibrary, /onOpenOriginal\(clip\.id\)/);
  assert.match(matchLibrary, /onCopyPath\(clip\.id\)/);
  assert.match(contextMenu, /ContextMenu as ContextMenuPrimitive.*from "radix-ui"/s);
  assert.match(app, /onCopyPath=\{handleCopyPath\}/);
  assert.match(app, /onOpenOriginal=\{handleOpenOriginal\}/);
  assert.match(css, /\.ui-context-menu-content/);
});

test("clip cards use only imported per-video official scores without match-wide fallbacks", () => {
  assert.match(matchLibrary, /formatOfficialVideoScore\(clip\)/);
  assert.match(matchLibrary, /if \(clip\.roundScore != null\) return `\$\{clip\.roundScore\} 评分`/);
  assert.match(matchLibrary, /expectsOfficialRoundScore\(clip\) \? "官方未同步" : null/);
  assert.doesNotMatch(matchLibrary, /"暂无"/);
  assert.match(backend, /roundScore:\s*clip\.roundScore \?\? null/);
  assert.doesNotMatch(matchLibrary, /clip\.combatScore/);

  for (const simulatedRatingPattern of [
    /clipRating/,
    /durationBonus/,
    /charCodeAt\(0\).*评分/s,
    /Math\.random\(\).*评分/s,
    /return\s+520\s+\+/,
  ]) {
    assert.doesNotMatch(matchLibrary, simulatedRatingPattern);
    assert.doesNotMatch(backend, simulatedRatingPattern);
  }
});

test("batch management uses Radix selection and confirmation primitives with real backend actions", () => {
  assert.match(libraryWorkspace, /updateClipSelection/);
  assert.match(libraryWorkspace, /Ctrl 点击多选/);
  assert.match(libraryWorkspace, /<BatchTagDialog/);
  assert.match(libraryWorkspace, /<UiAlertDialog/);
  assert.match(libraryWorkspace, /onSetTrashedForClips/);
  assert.match(matchLibrary, /<UiCheckbox/);
  assert.match(matchLibrary, /移入回收站/);
  assert.match(matchLibrary, /永久删除本地视频/);
  assert.match(checkbox, /Checkbox as CheckboxPrimitive.*from "radix-ui"/s);
  assert.match(dialog, /Dialog as DialogPrimitive.*from "radix-ui"/s);
  assert.match(alertDialog, /AlertDialog as AlertDialogPrimitive.*from "radix-ui"/s);
  assert.match(backend, /invoke<BackendClip>\("set_clip_trashed"/);
  assert.match(backend, /"delete_clips_permanently"/);
  assert.match(backend, /invoke\("remove_clip_from_index"/);
  assert.match(css, /\.library-batch-toolbar/);
  assert.match(css, /\.batch-tag-dialog/);
});

test("library animation keeps its visual effects while isolating offscreen and repeated work", () => {
  assert.match(matchLibrary, /const prefersReducedMotion = Boolean\(useReducedMotion\(\)\)/);
  assert.match(matchLibrary, /motionProfile=\{sharedMotionProfile\}/);
  assert.match(matchLibrary, /<m\.article/);
  assert.match(matchLibrary, /whileHover=\{\{ y: profile\.hoverY \}\}/);
  assert.match(libraryWorkspace, /const handleSelectionGesture = useCallback/);
  assert.match(libraryWorkspace, /selectedClipIdsRef/);
  assert.match(css, /\.match-board\s*\{[^}]*content-visibility:\s*auto;[^}]*contain:\s*layout paint style;/s);
  assert.match(css, /\.ambient-orb\s*\{[^}]*will-change:\s*transform;/s);
});

test("scan workspace is connected to the existing backend operations", () => {
  assert.match(scan, /onClick=\{onAddDirectory\}/);
  assert.match(scan, /onClick=\{onDiscoverAll\}/);
  assert.match(scan, /onClick=\{onStartScan\}/);
  assert.match(scan, /aria-label="全部扫描目录"/);
  assert.match(css, /\.scan-directory-grid\s*\{[^}]*max-height:[^}]*overflow-y:\s*auto;[^}]*scrollbar-gutter:\s*stable;/s);
  assert.match(scan, /\{accounts\.map\(\(account\) => \(/);
  assert.doesNotMatch(scan, /accounts\.slice/);
  assert.match(css, /\.scan-account-grid\s*\{[^}]*max-height:[^}]*overflow-y:\s*auto;[^}]*scrollbar-gutter:\s*stable;/s);
  assert.match(scan, /progress\.processed \/ progress\.total/);
  assert.match(scan, /onClick=\{onCancelScan\}/);
  assert.match(scan, /summary\.newClipCount/);
  assert.match(scan, /onClick=\{onOpenLibrary\}/);
});

test("preview keeps interactive timeline flags without restoring the event list", () => {
  assert.match(preview, /clip\?\.clipEvents/);
  assert.match(preview, /preview-timeline-flag/);
  assert.match(preview, /onClick=\{\(\) => seekTo\(event\.videoTimeMs \/ 1000\)\}/);
  assert.doesNotMatch(preview, /preview-event-list|事件标记|event-row/);
  assert.match(mockData, /id: "clip-1-kill-1"/);
  assert.match(mockData, /videoTimeMs: 17_800/);
});

test("preview removes decorative titles and prioritizes favorite and tag organization", () => {
  assert.doesNotMatch(app, /视频预览/);
  assert.doesNotMatch(preview, /preview-video-hud|preview-rec|ROUND/);
  assert.match(app, /activeScreen !== "preview" \? \(\s*<AppTopBar/s);
  assert.match(css, /\.app-root--preview\s*\{[^}]*grid-template-rows:\s*minmax\(0,\s*1fr\);/s);
  assert.doesNotMatch(css, /app-topbar--preview/);
  assert.match(preview, /preview-favorite-card/);
  assert.match(preview, /preview-tag-section/);
  assert.match(preview, /素材整理/);
  assert.match(css, /\.preview-favorite-card\s*\{/);
  assert.match(css, /\.preview-tag-section\s*\{/);
  assert.match(
    css,
    /\.preview-intel-list\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/s,
  );
  assert.match(css, /\.preview-intel-list > div\s*\{[^}]*min-height:\s*34px;/s);
});

test("preview actions use the existing media, favorite, tag, note, and file APIs", () => {
  assert.match(preview, /getClipMedia\(clip\.id\)/);
  assert.match(preview, /onToggleFavorite\(clip\.id\)/);
  assert.match(preview, /onToggleTag\(clip\.id/);
  assert.match(preview, /onUpdateNote\(clip\.id, noteDraft\)/);
  assert.match(preview, /onOpenOriginal\(clip\.id\)/);
  assert.match(preview, /onCopyPath\(clip\.id\)/);
  assert.match(preview, /displayHighlightTitle\(clip\)/);
});

test("cinematic theme uses charcoal, warm white, and Valorant red across all screens", () => {
  assert.match(css, /--canvas-0:\s*#06070a/);
  assert.match(css, /--text-primary:\s*#ece8e1/);
  assert.match(css, /--accent-rose:\s*#ff4655/);
  assert.match(css, /\.app-shell--scan/);
  assert.match(css, /\.app-shell--library/);
  assert.match(css, /\.app-shell--preview/);
  assert.match(css, /grid-template-columns:\s*242px minmax\(480px, 1fr\) 320px/);

  for (const forbidden of ["#39dfbd", "#2ddfba", "#4fffe7", "#15836f", "#146156", "#bffcf1"]) {
    assert.equal(css.toLowerCase().includes(forbidden), false, `legacy teal ${forbidden} leaked into the migrated theme`);
  }
});

test("the migration preserves thumbnail and local text avatar fallbacks", () => {
  assert.match(matchLibrary, /<ThumbnailImage/);
  assert.match(matchLibrary, /clip-thumb-fallback/);
  assert.match(preview, /<ThumbnailImage/);
  assert.match(preview, /cinematic-artwork-fallback/);
  assert.match(matchLibrary, /agentInitial/);
  assert.doesNotMatch(
    matchLibrary,
    /tactical-fallback-frame|agent-duelist-fallback|agent-controller-fallback/,
  );
});

function source(path: string): string {
  return readFileSync(new URL(path, import.meta.url), "utf8");
}
