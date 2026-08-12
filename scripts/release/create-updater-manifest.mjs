#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  existsSync,
  lstatSync,
  readFileSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, resolve } from "node:path";

const PLATFORM = "windows-x86_64";
const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const MAX_NOTES_BYTES = 64 * 1024;

try {
  const options = parseArguments(process.argv.slice(2));
  const expected = buildExpectedManifest(options);

  if (options.verifyManifest !== undefined) {
    const actual = readJsonFile(options.verifyManifest, "updater manifest");
    assertDeepEqual(actual, expected, "updater manifest differs from the verified release inputs");
    process.stdout.write(`${JSON.stringify(buildReport(options, expected, "verified"), null, 2)}\n`);
  } else {
    const output = resolve(options.output);
    assert(!existsSync(output), `refusing to overwrite updater manifest: ${output}`);
    const parent = realpathSync(dirname(output));
    assert(lstatSync(parent).isDirectory(), `updater manifest parent is not a directory: ${parent}`);
    writeFileSync(output, `${JSON.stringify(expected, null, 2)}\n`, {
      encoding: "utf8",
      flag: "wx",
    });
    process.stdout.write(`${JSON.stringify(buildReport(options, expected, "created"), null, 2)}\n`);
  }
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`updater manifest failed: ${message}\n`);
  process.exitCode = 1;
}

function parseArguments(args) {
  const values = new Map();
  const known = new Set([
    "--artifact",
    "--signature",
    "--version",
    "--tag",
    "--repository",
    "--notes-file",
    "--published-at",
    "--output",
    "--verify",
  ]);
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    assert(known.has(name), `unknown argument: ${name ?? "<missing>"}`);
    assert(value !== undefined && !value.startsWith("--"), `missing value for ${name}`);
    assert(!values.has(name), `duplicate argument: ${name}`);
    values.set(name, value);
  }
  for (const name of [
    "--artifact",
    "--signature",
    "--version",
    "--tag",
    "--repository",
    "--notes-file",
    "--published-at",
  ]) {
    assert(values.has(name), `missing required argument: ${name}`);
  }
  assert(
    values.has("--output") !== values.has("--verify"),
    "provide exactly one of --output or --verify",
  );
  return {
    artifact: values.get("--artifact"),
    signature: values.get("--signature"),
    version: values.get("--version"),
    tag: values.get("--tag"),
    repository: values.get("--repository"),
    notesFile: values.get("--notes-file"),
    publishedAt: values.get("--published-at"),
    output: values.get("--output"),
    verifyManifest: values.get("--verify"),
  };
}

function buildExpectedManifest(options) {
  assert(STABLE_VERSION.test(options.version), "--version must be a stable MAJOR.MINOR.PATCH version");
  assert(options.tag === `v${options.version}`, `--tag must be exactly v${options.version}`);
  assert(REPOSITORY.test(options.repository), "--repository must be an owner/repository slug");

  const publishedAt = new Date(options.publishedAt);
  assert(
    Number.isFinite(publishedAt.valueOf()) && publishedAt.toISOString() === options.publishedAt,
    "--published-at must be a canonical UTC ISO-8601 timestamp",
  );

  const artifact = readRegularFile(options.artifact, "updater artifact");
  const signature = readRegularFile(options.signature, "updater signature")
    .toString("utf8")
    .trim();
  validateUpdaterSignature(signature);
  const notesBytes = readRegularFile(options.notesFile, "release notes");
  assert(notesBytes.length <= MAX_NOTES_BYTES, "release notes exceed 64 KiB");
  const notes = notesBytes.toString("utf8").trim();
  assert(notes.length > 0, "release notes must not be empty");

  const artifactName = basename(realpathSync(options.artifact));
  assert(artifactName === basename(options.artifact), "updater artifact must not resolve to a different file name");
  const url = `https://github.com/${options.repository}/releases/download/${options.tag}/${encodeURIComponent(artifactName)}`;

  return {
    version: options.version,
    notes,
    pub_date: options.publishedAt,
    platforms: {
      [PLATFORM]: { signature, url },
    },
  };
}

function buildReport(options, manifest, status) {
  const artifact = readRegularFile(options.artifact, "updater artifact");
  return {
    schemaVersion: 1,
    status,
    releaseChannel: "stable",
    version: manifest.version,
    tag: options.tag,
    platform: PLATFORM,
    artifact: {
      fileName: basename(options.artifact),
      sizeBytes: artifact.length,
      sha256: createHash("sha256").update(artifact).digest("hex"),
    },
    updaterUrl: manifest.platforms[PLATFORM].url,
  };
}

function readRegularFile(path, description) {
  const requested = resolve(path);
  const requestedItem = lstatSync(requested);
  assert(
    requestedItem.isFile() && !requestedItem.isSymbolicLink(),
    `${description} must be a regular file`,
  );
  const resolved = realpathSync(requested);
  const item = lstatSync(resolved);
  assert(item.isFile(), `${description} must resolve to a regular file`);
  return readFileSync(resolved);
}

function readJsonFile(path, description) {
  const bytes = readRegularFile(path, description);
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${description} is not valid JSON: ${error.message}`);
  }
}

function validateUpdaterSignature(value) {
  assert(
    value.length >= 128 && value.length <= 4096 && /^[A-Za-z0-9+/]+={0,2}$/.test(value),
    "updater signature must be the base64 contents of a Tauri .sig file",
  );
  assert(value.length % 4 === 0, "updater signature is not canonical base64");
  let decoded;
  try {
    decoded = Buffer.from(value, "base64").toString("utf8");
  } catch {
    throw new Error("updater signature is not valid base64");
  }
  assert(
    Buffer.from(Buffer.from(value, "base64")).toString("base64") === value,
    "updater signature is not canonical base64",
  );
  assert(
    decoded.startsWith("untrusted comment:") && decoded.includes("\ntrusted comment:"),
    "updater signature does not contain a Minisign signature envelope",
  );
}

function assertDeepEqual(actual, expected, message) {
  const actualCanonical = JSON.stringify(canonicalize(actual));
  const expectedCanonical = JSON.stringify(canonicalize(expected));
  assert(actualCanonical === expectedCanonical, message);
}

function canonicalize(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalize(value[key])]),
    );
  }
  return value;
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
