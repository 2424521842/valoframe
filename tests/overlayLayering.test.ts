import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../src/cinematic.css", import.meta.url), "utf8");

function zIndexOf(selector: string): number {
  const pattern = new RegExp(
    `${selector.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")}\\s*\\{[^}]*?z-index:\\s*(\\d+)`,
    "s",
  );
  const match = css.match(pattern);
  assert.ok(match, `expected a z-index declaration for ${selector}`);
  return Number(match![1]);
}

const DIALOG_OVERLAY = zIndexOf(".ui-dialog-overlay");
const DIALOG_CONTENT = zIndexOf(".ui-dialog-content");

test("dialog content renders above its own overlay", () => {
  assert.ok(
    DIALOG_CONTENT > DIALOG_OVERLAY,
    `dialog content (${DIALOG_CONTENT}) must sit above the overlay (${DIALOG_OVERLAY})`,
  );
});

// Radix portals selects, popovers, tooltips and context menus to <body>, i.e. as siblings of the
// dialog overlay rather than descendants. Any of them layered below the overlay is still painted
// but the overlay intercepts every pointer event, so the control looks fine and cannot be used.
for (const selector of [
  ".ui-select-content",
  ".ui-context-menu-content",
  ".ui-tooltip-content",
  ".library-search-popover",
]) {
  test(`${selector} stays clickable above dialog layers`, () => {
    const layer = zIndexOf(selector);
    assert.ok(
      layer > DIALOG_CONTENT,
      `${selector} (${layer}) must sit above dialog content (${DIALOG_CONTENT}) to remain interactive inside a dialog`,
    );
  });
}

test("select trigger has a base style so unscoped selects match the shell", () => {
  // Every consumer used to supply its own scoped trigger rule; a select rendered without one fell
  // back to an unstyled button.
  assert.match(
    css,
    /^\.ui-select-trigger\s*\{[^}]*border:\s*1px solid[^}]*background:\s*#0d1015;/ms,
  );
  assert.match(css, /^\.ui-select-trigger\[data-state="open"\]|^\.ui-select-trigger:focus-visible/ms);
  assert.match(css, /^\.ui-select-trigger:disabled\s*\{[^}]*cursor:\s*not-allowed;/ms);
});

test("manual import dialog scrolls internally and bounds the preview player", () => {
  assert.match(css, /\.manual-import-dialog\s*\{[^}]*overflow-y:\s*auto;/s);
  assert.match(css, /\.manual-import-video\s*\{[^}]*max-height:\s*240px;/s);
});
