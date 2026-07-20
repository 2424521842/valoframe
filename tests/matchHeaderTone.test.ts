import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const component = readFileSync(
  new URL("../src/components/MatchLibrary.tsx", import.meta.url),
  "utf8",
);
const css = readFileSync(
  new URL("../src/cinematic.css", import.meta.url),
  "utf8",
);

test("match headers expose semantic win and loss surface classes", () => {
  assert.match(component, /const tone = resultTone\(matchGroup\.resultLabel\)/);
  assert.match(component, /match-board-header match-board-header--\$\{tone\}/);
  assert.match(css, /\.match-board-header--win\s*\{[^}]*55, 184, 122/s);
  assert.match(css, /\.match-board-header--loss\s*\{[^}]*231, 72, 88/s);
  assert.match(css, /\.match-board-result--win\s*\{[^}]*#237d5b/s);
  assert.match(css, /\.match-board-result--loss\s*\{[^}]*#ad3543/s);
});

test("map art uses a bundled image with a masked gradient fallback", () => {
  assert.match(css, /\.match-board-header\s*\{[^}]*position:\s*relative;[^}]*overflow:\s*hidden;/s);
  assert.match(css, /\.match-board-map-art\s*\{[^}]*background:\s*linear-gradient/s);
  assert.match(css, /\.match-board-map-art img\s*\{[^}]*position:\s*absolute;[^}]*right:\s*82px;/s);
  assert.match(css, /mask-image:\s*linear-gradient\([\s\S]*transparent 0%[\s\S]*#000 34%[\s\S]*transparent 100%/s);
  assert.match(component, /<MapSlice name=\{matchGroup\.mapName\}/);
  assert.match(component, /valorantMapListViewIconUrl\(name\)/);
  assert.doesNotMatch(component, /https?:\/\//);
});

test("agent portraits use bundled artwork with local text fallbacks", () => {
  assert.match(component, /valorantAgentDisplayIconUrl\(name\)/);
  assert.match(component, /<img[\s\S]*src=\{url\}/);
  assert.match(component, /agentInitial\(name\)/);
  assert.match(
    css,
    /\.match-board-agent img\s*\{[^}]*object-fit:\s*contain;[^}]*drop-shadow/s,
  );
  assert.match(css, /\.match-board-agent--fallback\s*\{[^}]*border-radius:\s*50%;[^}]*background:\s*linear-gradient/s);
  assert.doesNotMatch(css, /inset 3px 0 0 rgba\(var\(--match-tone-rgb\)/);
});
