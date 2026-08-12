import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(
  new URL("../scripts/release/create-updater-manifest.mjs", import.meta.url),
);
const signatureEnvelope = [
  "untrusted comment: signature from minisign secret key",
  "R".repeat(100),
  "trusted comment: timestamp:1786204800 file:瓦刻_0.2.0_x64-setup.nsis.zip",
  "Q".repeat(100),
].join("\n");
const signature = Buffer.from(signatureEnvelope, "utf8").toString("base64");

test("creates and verifies a stable static updater manifest", () => {
  withFixture((fixture) => {
    const created = run(fixture);
    assert.equal(created.status, 0, created.stderr);
    const manifest = JSON.parse(readFileSync(fixture.manifest, "utf8"));
    assert.deepEqual(Object.keys(manifest.platforms), ["windows-x86_64"]);
    assert.equal(manifest.version, "0.2.0");
    assert.equal(manifest.platforms["windows-x86_64"].signature, signature);
    assert.equal(
      manifest.platforms["windows-x86_64"].url,
      "https://github.com/2424521842/valoframe/releases/download/v0.2.0/valoframe_0.2.0_x64-setup.nsis.zip",
    );

    const verified = run(fixture, true);
    assert.equal(verified.status, 0, verified.stderr);
    assert.equal(JSON.parse(verified.stdout).status, "verified");
  });
});

test("rejects prerelease metadata and a mismatched tag", () => {
  withFixture((fixture) => {
    const prerelease = run(fixture, false, { version: "0.2.0-rc.1", tag: "v0.2.0-rc.1" });
    assert.notEqual(prerelease.status, 0);
    assert.match(prerelease.stderr, /stable MAJOR\.MINOR\.PATCH/);

    const wrongTag = run(fixture, false, { tag: "v0.2.1" });
    assert.notEqual(wrongTag.status, 0);
    assert.match(wrongTag.stderr, /must be exactly v0\.2\.0/);
  });
});

test("verification detects tampered URLs, notes, and signatures", () => {
  for (const mutate of [
    (manifest: any) => (manifest.platforms["windows-x86_64"].url = "https://example.invalid/update.zip"),
    (manifest: any) => (manifest.notes = "tampered"),
    (manifest: any) => (manifest.platforms["windows-x86_64"].signature = signature.slice(0, -4) + "AAAA"),
  ]) {
    withFixture((fixture) => {
      const created = run(fixture);
      assert.equal(created.status, 0, created.stderr);
      const manifest = JSON.parse(readFileSync(fixture.manifest, "utf8"));
      mutate(manifest);
      writeFileSync(fixture.manifest, `${JSON.stringify(manifest, null, 2)}\n`);
      const verified = run(fixture, true);
      assert.notEqual(verified.status, 0);
      assert.match(verified.stderr, /differs from the verified release inputs/);
    });
  }
});

function withFixture(callback: (fixture: Fixture) => void) {
  const root = mkdtempSync(join(tmpdir(), "vhm-updater-manifest-"));
  try {
    const fixture = {
      root,
      artifact: join(root, "valoframe_0.2.0_x64-setup.nsis.zip"),
      signature: join(root, "valoframe_0.2.0_x64-setup.nsis.zip.sig"),
      notes: join(root, "notes.md"),
      manifest: join(root, "latest.json"),
    };
    writeFileSync(fixture.artifact, "signed updater bytes");
    writeFileSync(fixture.signature, `${signature}\n`);
    writeFileSync(fixture.notes, "## 0.2.0\n\nStable update notes.\n");
    callback(fixture);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function run(
  fixture: Fixture,
  verify = false,
  overrides: { version?: string; tag?: string } = {},
) {
  const args = [
    "--artifact",
    fixture.artifact,
    "--signature",
    fixture.signature,
    "--version",
    overrides.version ?? "0.2.0",
    "--tag",
    overrides.tag ?? "v0.2.0",
    "--repository",
    "2424521842/valoframe",
    "--notes-file",
    fixture.notes,
    "--published-at",
    "2026-08-08T16:00:00.000Z",
    verify ? "--verify" : "--output",
    fixture.manifest,
  ];
  return spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
}

interface Fixture {
  root: string;
  artifact: string;
  signature: string;
  notes: string;
  manifest: string;
}
