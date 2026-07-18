import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../src/cinematic.css", import.meta.url), "utf8");

test("cinematic tokens use the charcoal and Valorant-red production foundation", () => {
  assert.match(css, /--canvas-0:\s*#06070a/);
  assert.match(css, /--text-primary:\s*#ece8e1/);
  assert.match(css, /--accent-rose:\s*#ff4655/);
  assert.match(css, /--surface-glass:/);
});

test("topbar and library command surfaces use the production material contract", () => {
  assert.match(
    css,
    /\.app-topbar\s*\{[^}]*background:\s*linear-gradient\([^}]*rgba\(18,\s*20,\s*27,/s,
  );
  assert.match(
    css,
    /\.library-command-bar\s*\{[^}]*border-bottom:\s*1px solid #292d36;[^}]*background:\s*rgba\(13,\s*15,\s*20,\s*0\.96\);/s,
  );
  assert.match(css, /\.library-global-search:focus-within\s*\{/);
});

test("production navigation uses the Valorant-red active state", () => {
  assert.match(
    css,
    /\.cinematic-sidebar-item--active\s*\{[^}]*border-color:\s*rgba\(255,\s*70,\s*85,[^}]*background:\s*linear-gradient\([^}]*rgba\(255,\s*70,\s*85,/s,
  );
  assert.doesNotMatch(css, /\.sidebar-item--active\s*\{/);
});

test("custom tag colors have distinct visual styles after the base tag rule", () => {
  const baseRuleIndex = css.indexOf(".tag {");

  assert.ok(baseRuleIndex >= 0);
  assert.ok(css.indexOf(".tag--red {") > baseRuleIndex);
  assert.ok(css.indexOf(".tag--teal {") > baseRuleIndex);
  assert.ok(css.indexOf(".tag--gold {") > baseRuleIndex);
  assert.ok(css.indexOf(".tag--blue {") > baseRuleIndex);
  assert.ok(css.indexOf(".tag--green {") > baseRuleIndex);
  assert.match(css, /\.tag--red\s*\{[^}]*color:\s*#ff8791;/s);
  assert.match(css, /\.tag--teal\s*\{[^}]*color:\s*#5ce3d1;/s);
  assert.match(css, /\.tag--gold\s*\{[^}]*color:\s*var\(--gold\);/s);
  assert.match(css, /\.tag--blue\s*\{[^}]*color:\s*#8bb8f0;/s);
  assert.match(css, /\.tag--green\s*\{[^}]*color:\s*#7ed79b;/s);
});

test("responsive CSS covers the production workspaces", () => {
  assert.match(css, /@media\s*\(max-width:\s*1190px\)/);
  assert.match(css, /@media\s*\(max-width:\s*919px\)/);
  assert.match(css, /@media\s*\(max-width:\s*680px\)/);
  assert.match(css, /@media\s*\(max-width:\s*1050px\)/);
  assert.match(css, /html,\s*#root\s*\{[^}]*min-width:\s*0;/s);
  assert.match(css, /body\s*\{[^}]*min-width:\s*0;/s);
  assert.match(css, /\.app-root\s*\{[^}]*min-width:\s*0;/s);
});

test("reduced motion stops ambient, selection, and dialog animation", () => {
  const reduced = extractBlocks(css, "@media (prefers-reduced-motion: reduce)");

  assert.match(reduced, /html\s*\{[^}]*scroll-behavior:\s*auto;/s);
  assert.match(reduced, /\.ambient-orb\s*\{[^}]*animation:\s*none\s*!important;/s);
  assert.match(reduced, /\.match-clip-select\s*\{[^}]*transition:\s*none;/s);
  assert.match(reduced, /\.ui-dialog-content\[data-state="open"\][^}]*\{[^}]*animation:\s*none;/s);
});

test("smallest supported viewport keeps current command and card layouts compact", () => {
  const smallest = extractBlocks(css, "@media (max-width: 680px)");

  assert.match(smallest, /\.library-search-row\s*\{[^}]*grid-template-columns:\s*1fr;/s);
  assert.match(smallest, /\.match-board-clips\s*\{[^}]*grid-template-columns:\s*1fr;/s);
  assert.match(smallest, /\.library-clear-button\s*\{[^}]*display:\s*none;/s);
});

test("grid clip cards keep the official per-video score visible", () => {
  assert.match(
    css,
    /\.match-clip-score\s*\{[^}]*display:\s*block;[^}]*grid-column:\s*2;[^}]*color:\s*var\(--red\);/s,
  );
  assert.match(
    css,
    /\.match-clip-score--unavailable\s*\{[^}]*color:\s*#8f939d;/s,
  );
  assert.doesNotMatch(
    css,
    /\.match-clip-copy\s*>\s*strong,\s*\.match-clip-score\s*\{[^}]*display:\s*none;/s,
  );
});

test("grid clip cards collapse the empty metadata footer", () => {
  assert.match(
    css,
    /\.match-clip-copy\s*\{[^}]*min-height:\s*44px;[^}]*padding:\s*8px 36px 8px 11px;/s,
  );
  assert.doesNotMatch(
    css,
    /\.match-clip-copy > div\s*\{[^}]*min-height:\s*22px;/s,
  );
  assert.match(
    css,
    /\.match-clip-favorite\s*\{[^}]*bottom:\s*8px;/s,
  );
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
