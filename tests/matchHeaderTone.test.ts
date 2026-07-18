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

test("map accent is a local masked gradient instead of remote artwork", () => {
  assert.match(css, /\.match-board-header\s*\{[^}]*position:\s*relative;[^}]*overflow:\s*hidden;/s);
  assert.match(css, /\.match-board-map-art\s*\{[^}]*background:\s*linear-gradient/s);
  assert.match(css, /mask-image:\s*linear-gradient\([\s\S]*transparent 0%[\s\S]*#000 34%[\s\S]*transparent 100%/s);
  assert.doesNotMatch(component, /https?:\/\/|valorantMapListViewIconUrl/);
});

test("agent portraits use local text initials over semantic row colors", () => {
  assert.match(component, /\{agentInitial\(name\)\}/);
  assert.match(
    css,
    /\.match-board-agent\s*\{[^}]*border-radius:\s*50%;[^}]*background:\s*linear-gradient/s,
  );
  assert.doesNotMatch(component, /valorantAgentDisplayIconUrl|<img/);
  assert.doesNotMatch(css, /inset 3px 0 0 rgba\(var\(--match-tone-rgb\)/);
});
