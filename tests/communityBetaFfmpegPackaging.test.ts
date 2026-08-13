import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

type JsonObject = Record<string, any>;

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const scriptPath = join(
  repositoryRoot,
  "scripts",
  "release",
  "package-minimal-ffmpeg-community-beta.mjs",
);
const contractPath = join(
  repositoryRoot,
  "third_party",
  "ffmpeg",
  "community-beta-minimal-windows-x64.json",
);
const personalStableContractRelativePath =
  "third_party/ffmpeg/personal-community-stable-minimal-windows-x64.json";
const personalStableContractPath = join(
  repositoryRoot,
  ...personalStableContractRelativePath.split("/"),
);
const candidateManifestPath = join(
  repositoryRoot,
  "third_party",
  "ffmpeg",
  "minimal-windows-x64-candidate.json",
);
const contract = readJson(contractPath);
const personalStableContract = readJson(personalStableContractPath);
const candidateManifest = readJson(candidateManifestPath);

function readJson(path: string): JsonObject {
  return JSON.parse(readFileSync(path, "utf8")) as JsonObject;
}

function sha256(value: Buffer | string): string {
  return createHash("sha256").update(value).digest("hex");
}

function sha256File(path: string): string {
  return sha256(readFileSync(path));
}

function writeJson(path: string, value: unknown): void {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function makeSyntheticX64Pe(): Buffer {
  const bytes = Buffer.alloc(512, 0);
  bytes.writeUInt16LE(0x5a4d, 0);
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.writeUInt32LE(0x00004550, 0x80);
  bytes.writeUInt16LE(0x8664, 0x84);
  Buffer.from("synthetic-test-only", "ascii").copy(bytes, 0x100);
  return bytes;
}

function createCandidateArtifact(parent: string): {
  artifactRoot: string;
  checksums: Record<string, string>;
} {
  const artifactRoot = join(parent, "candidate");
  mkdirSync(artifactRoot);
  const ffmpeg = makeSyntheticX64Pe();
  const ffmpegHash = sha256(ffmpeg);
  const sourceArchive = Buffer.concat([
    Buffer.from([0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]),
    Buffer.from("synthetic-corresponding-source-test-fixture", "utf8"),
  ]);
  const metadata = {
    schemaVersion: 1,
    status: contract.requiredInput.buildMetadataStatus,
    sourceCommit: contract.sourceCommit,
    sourceDateEpoch: 1_750_000_000,
    targetTriple: candidateManifest.build.targetTriple,
    licenseExpression: candidateManifest.build.licenseExpression,
    configureFlags: candidateManifest.build.configureFlags,
    externalLibraries: [],
    executable: {
      fileName: "ffmpeg.exe",
      sizeBytes: ffmpeg.length,
      sha256: ffmpegHash,
    },
  };
  const artifactContents: Record<string, Buffer | string> = {
    "BUILD-METADATA.json": `${JSON.stringify(metadata, null, 2)}\n`,
    "compiler-version.txt": "synthetic mingw test toolchain\n",
    "config.h": "#define SYNTHETIC_TEST_ONLY 1\n",
    "config.mak": "SYNTHETIC_TEST_ONLY=yes\n",
    "configure-flags.txt": `${candidateManifest.build.configureFlags.join("\n")}\n`,
    "ffmpeg-corresponding-source.tar.xz": sourceArchive,
    "ffmpeg.exe": ffmpeg,
    "pe-imports.txt": "DLL Name: KERNEL32.dll\n",
  };
  const checksums: Record<string, string> = {};
  for (const name of contract.requiredInput.artifactFiles as string[]) {
    const contents = artifactContents[name];
    assert.notEqual(contents, undefined, `missing fixture contents for ${name}`);
    writeFileSync(join(artifactRoot, name), contents);
    checksums[name] = sha256(
      typeof contents === "string" ? Buffer.from(contents, "utf8") : contents,
    );
  }
  const checksumText = (contract.requiredInput.artifactFiles as string[])
    .map((name) => `${checksums[name]}  ${name}`)
    .join("\n");
  writeFileSync(
    join(artifactRoot, contract.requiredInput.checksumManifest),
    `${checksumText}\n`,
    "utf8",
  );
  const verification = {
    schemaVersion: 1,
    status: contract.requiredInput.windowsVerificationStatus,
    checkedAtUtc: "2026-07-21T00:00:00.000Z",
    executable: {
      sizeBytes: ffmpeg.length,
      sha256: checksums["ffmpeg.exe"],
    },
    sourceCommit: contract.sourceCommit,
    licenseExpression: candidateManifest.build.licenseExpression,
    externalLibraries: [],
    artifactEvidence: {
      root: "synthetic-test-artifact-root",
      checksumManifestSha256: sha256File(
        join(artifactRoot, contract.requiredInput.checksumManifest),
      ),
      files: checksums,
      sourceArchiveSha256:
        checksums["ffmpeg-corresponding-source.tar.xz"],
      peImports: ["KERNEL32.dll"],
    },
    requiredCapabilities: candidateManifest.runtimeContract.requiredCapabilities,
    smoke: {
      fixtureSha256:
        candidateManifest.runtimeContract.smokeFixture.sha256,
      outputSizeBytes: 128,
      outputSha256: sha256("synthetic jpeg output"),
    },
    smokeFixtures: [
      candidateManifest.runtimeContract.smokeFixture,
      ...(candidateManifest.runtimeContract.additionalSmokeFixtures ?? []).filter(
        (fixture: JsonObject) => fixture.enabled !== false,
      ),
    ].map((fixture: JsonObject) => ({
      codec: fixture.codec,
      fixtureSha256: fixture.sha256,
      outputSizeBytes: 128,
      outputSha256: sha256(`synthetic ${fixture.codec} jpeg output`),
    })),
    promotionAuthorized: false,
  };
  writeJson(
    join(artifactRoot, contract.requiredInput.windowsVerificationFile),
    verification,
  );
  return { artifactRoot, checksums };
}

function runPackager(
  artifactRoot: string,
  output: string,
  contractRelativePath?: string,
) {
  return spawnSync(
    process.execPath,
    [
      scriptPath,
      "--candidate-root",
      artifactRoot,
      "--output",
      output,
      "--repository-root",
      repositoryRoot,
      ...(contractRelativePath
        ? ["--contract", contractRelativePath]
        : []),
    ],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
    },
  );
}

function listFiles(root: string): string[] {
  const files: string[] = [];
  function walk(directory: string): void {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      if (entry.isDirectory()) walk(full);
      else files.push(relative(root, full).split(sep).join("/"));
    }
  }
  walk(root);
  return files.sort();
}

test("community-beta FFmpeg contract cannot claim distribution or public approval", () => {
  assert.equal(contract.schemaVersion, 1);
  assert.equal(contract.status, "community-beta-technical-packaging-contract");
  assert.equal(contract.channel, "community-beta");
  assert.equal(contract.sourceCommit, candidateManifest.source.commit);
  assert.equal(contract.complianceBoundary.technicalPackagingOnly, true);
  assert.equal(
    contract.complianceBoundary.communityBetaDistributionAuthorized,
    false,
  );
  assert.equal(contract.complianceBoundary.publicReleaseApproved, false);
  assert.equal(contract.complianceBoundary.modifiesPublicReleasePolicy, false);
  assert.equal(
    contract.requiredInput.windowsVerificationPromotionAuthorized,
    false,
  );
  assert.deepEqual(
    (contract.licenseMaterials as JsonObject[]).map((item) => item.outputFileName),
    ["COPYING.LGPLv3.txt", "COPYING.GPLv3.txt"],
  );
  const script = readFileSync(scriptPath, "utf8");
  assert.doesNotMatch(script, /public-release-policy\.json/u);
  assert.equal(
    contract.output.status,
    "prepared-community-beta-candidate-not-public-release-approved",
  );
  assert.match(script, /Independent JPEG Group/u);
});

test("verified minimal candidate becomes a hash-bound community-beta package", () => {
  const temporaryRoot = mkdtempSync(
    join(tmpdir(), "vhm-community-beta-ffmpeg-test-"),
  );
  try {
    const { artifactRoot, checksums } = createCandidateArtifact(temporaryRoot);
    const output = join(temporaryRoot, "package");
    const result = runPackager(artifactRoot, output);
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const summary = JSON.parse(result.stdout) as JsonObject;
    assert.equal(summary.status, contract.output.status);
    assert.equal(summary.communityBetaDistributionAuthorized, false);
    assert.equal(summary.publicReleaseApproved, false);

    const packageManifestPath = join(
      output,
      contract.output.packageManifestFileName,
    );
    const packageManifest = readJson(packageManifestPath);
    assert.equal(packageManifest.status, contract.output.status);
    assert.equal(packageManifest.sourceCommit, contract.sourceCommit);
    assert.equal(
      packageManifest.executable.sha256,
      checksums["ffmpeg.exe"],
    );
    assert.equal(
      packageManifest.correspondingSource.sha256,
      checksums["ffmpeg-corresponding-source.tar.xz"],
    );
    assert.equal(
      packageManifest.inputEvidence.candidateManifestSha256,
      sha256File(candidateManifestPath),
    );
    assert.equal(
      packageManifest.technicalPromotion.installerResourceOverlayPrepared,
      true,
    );
    assert.equal(
      packageManifest.technicalPromotion.communityBetaDistributionAuthorized,
      false,
    );

    const runtimePath = join(
      output,
      ...String(contract.output.runtimeRelativePath).split("/"),
    );
    const sourcePath = join(
      output,
      ...String(contract.output.sourceArchiveRelativePath).split("/"),
    );
    assert.equal(sha256File(runtimePath), checksums["ffmpeg.exe"]);
    assert.equal(
      sha256File(sourcePath),
      checksums["ffmpeg-corresponding-source.tar.xz"],
    );

    const licenseRoot = join(
      output,
      ...String(contract.output.licenseRootRelativePath).split("/"),
    );
    const buildInfo = readJson(join(licenseRoot, "BUILD-INFO.json"));
    assert.equal(buildInfo.status, "community-beta-technical-candidate");
    assert.equal(
      buildInfo.complianceBoundary.communityBetaDistributionAuthorized,
      false,
    );
    const sourceOffer = readFileSync(join(licenseRoot, "SOURCE-OFFER.md"), "utf8");
    assert.match(
      sourceOffer,
      new RegExp(checksums["ffmpeg-corresponding-source.tar.xz"], "u"),
    );
    assert.match(
      sourceOffer,
      /Community-beta distribution additionally requires the repository release-owner decision/u,
    );

    const checksumPath = join(
      output,
      contract.output.checksumManifestFileName,
    );
    const checksumLines = readFileSync(checksumPath, "utf8")
      .trim()
      .split(/\r?\n/u);
    const outputFiles = listFiles(output);
    assert.equal(
      checksumLines.length,
      outputFiles.length - 1,
      "The generated checksum manifest must cover every file except itself.",
    );
    assert.equal(
      checksumLines.some((line) =>
        line.endsWith(`  ${contract.output.packageManifestFileName}`),
      ),
      true,
    );
    for (const line of checksumLines) {
      const match = /^([0-9a-f]{64})  (.+)$/u.exec(line);
      assert.ok(match);
      const path = join(output, ...match[2].split("/"));
      assert.equal(sha256File(path), match[1]);
    }
    assert.equal(statSync(packageManifestPath).size > 0, true);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("community-beta packager rejects forged promotion authorization", () => {
  const temporaryRoot = mkdtempSync(
    join(tmpdir(), "vhm-community-beta-ffmpeg-reject-"),
  );
  try {
    const { artifactRoot } = createCandidateArtifact(temporaryRoot);
    const verificationPath = join(
      artifactRoot,
      contract.requiredInput.windowsVerificationFile,
    );
    const verification = readJson(verificationPath);
    verification.promotionAuthorized = true;
    writeJson(verificationPath, verification);
    const output = join(temporaryRoot, "package");
    const result = runPackager(artifactRoot, output);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /promotionAuthorized must be false/u);
    assert.equal(existsSync(output), false);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("the same verified minimal candidate can be packaged for owner-authorized personal stable", () => {
  const temporaryRoot = mkdtempSync(
    join(tmpdir(), "vhm-personal-stable-ffmpeg-test-"),
  );
  try {
    const { artifactRoot, checksums } = createCandidateArtifact(temporaryRoot);
    const output = join(temporaryRoot, "package");
    const result = runPackager(
      artifactRoot,
      output,
      personalStableContractRelativePath,
    );
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);

    const summary = JSON.parse(result.stdout) as JsonObject;
    assert.equal(summary.channel, "personal-community-stable");
    assert.equal(summary.status, personalStableContract.output.status);
    assert.equal(summary.ownerAuthorizedForThisChannel, true);
    assert.equal(summary.strictPublicReleaseApproved, false);
    assert.equal("communityBetaDistributionAuthorized" in summary, false);

    const packageManifest = readJson(
      join(output, personalStableContract.output.packageManifestFileName),
    );
    assert.equal(packageManifest.channel, "personal-community-stable");
    assert.equal(
      packageManifest.technicalPromotion.ownerAuthorizedForThisChannel,
      true,
    );
    assert.equal(
      packageManifest.technicalPromotion.strictPublicReleaseApproved,
      false,
    );
    assert.equal(packageManifest.executable.sha256, checksums["ffmpeg.exe"]);
    assert.equal(
      packageManifest.correspondingSource.sha256,
      checksums["ffmpeg-corresponding-source.tar.xz"],
    );

    const buildInfo = readJson(
      join(
        output,
        ...String(personalStableContract.output.licenseRootRelativePath).split(
          "/",
        ),
        "BUILD-INFO.json",
      ),
    );
    assert.equal(
      buildInfo.status,
      "personal-community-stable-technical-candidate",
    );
    assert.equal(
      buildInfo.complianceBoundary.ownerAuthorizedForThisChannel,
      true,
    );
    assert.equal(
      buildInfo.complianceBoundary.strictPublicReleaseApproved,
      false,
    );
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});
