import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../src/cinematic.css", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const mediumRules = extractBlocks(css, "@media (max-width: 1190px)");
const compactRules = extractBlocks(css, "@media (max-width: 919px)");
const smallestRules = extractBlocks(css, "@media (max-width: 680px)");
const fullscreenPreviewRules = extractBlocks(
  css,
  "@media (min-width: 1600px) and (min-height: 900px)",
);

test("wide shell uses the production sidebar and one content column", () => {
  assert.match(css, /body\s*\{[^}]*min-width:\s*0;/s);
  assert.match(app, /className=\{`app-root app-root--\$\{activeScreen\}`\}/);
  assert.match(
    css,
    /\.app-shell,\s*\.app-shell--library,\s*\.app-shell--scan,\s*\.app-shell--tags\s*\{[^}]*grid-template-columns:\s*214px\s+minmax\(0,\s*1fr\);/s,
  );
  assert.match(
    css,
    /\.app-topbar\s*\{[^}]*grid-template-columns:\s*214px\s+minmax\(0,\s*1fr\)\s+auto;[^}]*gap:\s*0;[^}]*padding:\s*0;/s,
  );
  assert.match(
    css,
    /\.topbar-brand\s*\{[^}]*align-self:\s*stretch;[^}]*padding:\s*0\s+18px;[^}]*border-right:\s*1px\s+solid\s+#292d36;/s,
  );
  assert.match(
    css,
    /\.app-shell\.app-shell--preview\s*\{[^}]*display:\s*block;[^}]*grid-template-columns:\s*none;/s,
  );
  assert.match(
    css,
    /\.app-root--preview\s*\{[^}]*grid-template-rows:\s*minmax\(0,\s*1fr\);/s,
  );
  assert.match(app, /activeScreen !== "preview" \? \(\s*<AppTopBar/s);
  assert.match(
    css,
    /@media\s*\(min-width:\s*920px\)\s*\{[\s\S]*?\.app-root--library\s*\{[^}]*grid-template-rows:\s*minmax\(0,\s*1fr\);[^}]*\}[\s\S]*?\.app-root--library\s*>\s*\.app-topbar\s*\{[^}]*position:\s*absolute;[^}]*width:\s*214px;[^}]*height:\s*54px;[^}]*\}[\s\S]*?\.app-root--library\s*>\s*\.app-topbar\s+\.topbar-context,\s*\.app-root--library\s*>\s*\.app-topbar\s+\.topbar-status\s*\{[^}]*display:\s*none;/s,
  );
  assert.match(
    css,
    /\.app-root--library\s+\.cinematic-sidebar\s*\{[^}]*padding-top:\s*54px;/s,
  );
  assert.doesNotMatch(css, /body\s*\{[^}]*min-width:\s*(?:320|1180)px/s);
});

test("medium shell narrows the production sidebar without adding a detail column", () => {
  assert.match(
    mediumRules,
    /\.app-shell,\s*\.app-shell--library,\s*\.app-shell--scan,\s*\.app-shell--tags\s*\{[^}]*grid-template-columns:\s*190px\s+minmax\(0,\s*1fr\);/s,
  );
  assert.match(
    mediumRules,
    /\.app-topbar\s*\{[^}]*grid-template-columns:\s*190px\s+minmax\(0,\s*1fr\)\s+auto;/s,
  );
  assert.match(
    css,
    /@media\s*\(min-width:\s*920px\)\s+and\s+\(max-width:\s*1190px\)\s*\{[^}]*\.app-root--library\s*>\s*\.app-topbar\s*\{[^}]*width:\s*190px;/s,
  );
  assert.doesNotMatch(mediumRules, /detail-panel|workspace-summary|account-group-header/);
});

test("compact shell exposes only the cinematic sidebar drawer and backdrop", () => {
  assert.match(
    compactRules,
    /\.cinematic-sidebar\s*\{[^}]*position:\s*fixed;[^}]*z-index:\s*50;[^}]*inset:\s*54px\s+auto\s+0\s+0;[^}]*transform:\s*translateX\(-105%\);/s,
  );
  assert.match(
    compactRules,
    /\.cinematic-sidebar--open\s*\{[^}]*transform:\s*translateX\(0\);/s,
  );
  assert.match(
    compactRules,
    /\.app-backdrop--sidebar\s*\{[^}]*z-index:\s*40;[^}]*inset:\s*54px\s+0\s+0;[^}]*display:\s*block;/s,
  );
  assert.match(
    compactRules,
    /\.app-topbar\s*\{[^}]*grid-template-columns:\s*auto\s+minmax\(0,\s*1fr\)\s+auto;[^}]*padding:\s*0\s+12px;/s,
  );
  assert.match(
    compactRules,
    /\.topbar-brand\s*\{[^}]*align-self:\s*center;[^}]*padding:\s*0;[^}]*border-right:\s*0;/s,
  );
  assert.match(compactRules, /\.topbar-menu-button\s*\{[^}]*display:\s*inline-grid;/s);
  assert.doesNotMatch(compactRules, /detail-panel|app-backdrop--detail/);
});

test("smallest layout keeps current library controls and cards in one column", () => {
  assert.match(
    css,
    /\.library-workspace\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/s,
  );
  assert.match(
    smallestRules,
    /\.library-search-row\s*\{[^}]*grid-template-columns:\s*1fr;/s,
  );
  assert.match(
    smallestRules,
    /\.match-board-clips\s*\{[^}]*grid-template-columns:\s*1fr;/s,
  );
  assert.match(smallestRules, /\.library-batch-toolbar\s*\{[^}]*left:\s*8px;/s);
});

test("thumbnail ratio and overlay stacking remain explicit", () => {
  assert.match(css, /\.match-clip-thumb\s*\{[^}]*aspect-ratio:\s*16\s*\/\s*9;/s);
  assert.match(css, /\.app-shell--preview\s*\{[^}]*display:\s*block;/s);
  assert.match(compactRules, /\.cinematic-sidebar\s*\{[^}]*z-index:\s*50;/s);
  assert.match(compactRules, /\.app-backdrop--sidebar\s*\{[^}]*z-index:\s*40;/s);
});

test("fullscreen preview expands the video while keeping controls in the viewport", () => {
  assert.match(
    fullscreenPreviewRules,
    /\.preview-video-stage\s*\{[^}]*height:\s*auto;[^}]*max-height:\s*calc\(100vh\s*-\s*170px\);[^}]*aspect-ratio:\s*16\s*\/\s*9;/s,
  );
});

test("topbar contains no fake navigation or window controls", () => {
  assert.doesNotMatch(app, /topbar-tabs|topbar-tab|window-actions/);
  assert.doesNotMatch(css, /\.topbar-tabs|\.topbar-tab|\.window-actions/);
});

test("App routes the compact sidebar overlay through one matching close handler", () => {
  assert.match(app, /const handleCloseSidebar = useCallback\(/);
  assert.match(app, /onClose=\{handleCloseSidebar\}/);
  assert.match(app, /if \(event\.key === "Escape"\)[\s\S]*handleCloseSidebar\(\);/);
  assert.match(
    app,
    /hasSidebarOverlay \? \([\s\S]*className="app-backdrop app-backdrop--sidebar"[\s\S]*onClick=\{handleCloseSidebar\}/,
  );
  assert.doesNotMatch(app, /handleCloseDetail|app-backdrop--detail/);
  assert.match(app, /menuTriggerRef\.current\?\.focus\(\)/);
});

test("motion is disabled for reduced-motion users", () => {
  assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/);
});

function extractBlocks(source: string, heading: string): string {
  const blocks: string[] = [];
  let searchFrom = 0;

  while (true) {
    const headingIndex = source.indexOf(heading, searchFrom);
    if (headingIndex === -1) return blocks.join("\n");
    const openBraceIndex = source.indexOf("{", headingIndex);
    let depth = 0;

    for (let index = openBraceIndex; index < source.length; index += 1) {
      if (source[index] === "{") depth += 1;
      if (source[index] === "}") depth -= 1;
      if (depth === 0) {
        blocks.push(source.slice(openBraceIndex + 1, index));
        searchFrom = index + 1;
        break;
      }
    }
  }
}
