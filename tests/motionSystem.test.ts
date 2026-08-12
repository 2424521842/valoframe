import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { motionProfile } from "../src/lib/motionProfile.ts";

const reactGlobal = globalThis as typeof globalThis & { React: typeof React };
reactGlobal.React = React;

test("reduced motion removes displacement and ambient movement", () => {
  assert.deepEqual(motionProfile(true), {
    enterY: 0,
    hoverY: 0,
    duration: 0.01,
    stagger: 0,
    ambient: false,
  });
});

test("full motion stays restrained", () => {
  const profile = motionProfile(false);

  assert.equal(profile.enterY, 12);
  assert.equal(profile.hoverY, -4);
  assert.ok(profile.duration <= 0.45);
  assert.ok(profile.stagger <= 0.04);
});

test("application root delegates reduced motion to the user preference", () => {
  const mainSource = readFileSync(
    new URL("../src/main.tsx", import.meta.url),
    "utf8",
  );
  const appSource = readFileSync(
    new URL("../src/App.tsx", import.meta.url),
    "utf8",
  );

  assert.match(
    appSource,
    /<MotionConfig reducedMotion=\{preferences\.motionMode === "reduced" \? "always" : "user"\}>/,
  );
  assert.match(appSource, /document\.documentElement\.dataset\.motion = "reduced"/);
  assert.match(appSource, /delete document\.documentElement\.dataset\.motion/);
  assert.match(mainSource, /<LazyMotion features=\{domAnimation\}/);
});

test("ambient backdrop is decorative and exposes both cinematic light fields", async () => {
  const { AmbientBackdrop } = await import(
    "../src/components/AmbientBackdrop.tsx"
  );
  const html = renderToStaticMarkup(React.createElement(AmbientBackdrop));

  assert.match(html, /aria-hidden="true"/);
  assert.match(html, /ambient-orb ambient-orb--violet/);
  assert.match(html, /ambient-orb ambient-orb--mint/);
  assert.match(html, /ambient-noise/);
});
