import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const scriptPath = fileURLToPath(
  new URL("../scripts/release/community-beta-preflight.mjs", import.meta.url),
);
const sourceCommit = "a".repeat(40);

test("accepts a fully disclosed, unsigned community beta without public-release approval", () => {
  withFixture((root) => {
    const result = runPreflight(root);

    assert.equal(result.status, 0, result.stderr);
    const output = JSON.parse(result.stdout);
    assert.equal(output.status, "ready-for-community-beta-build");
    assert.equal(output.releaseChannel, "community-beta");
    assert.equal(output.version, "0.1.0");
    assert.equal(output.tag, "v0.1.0-beta.2");
    assert.equal(output.sourceCommit, sourceCommit);
    assert.equal(output.strictPublicReleaseApproved, false);
    assert.equal(output.publicReleasePolicyReady, false);
    assert.equal(output.updater.enabled, false);
    assert.equal(output.updater.createUpdaterArtifacts, false);
    assert.equal(output.gameContent.assetCount, 42);
    assert.equal(output.gameContent.thirdPartyApprovalClaimed, false);
    assert.equal(output.gameContent.independentLegalReviewCompleted, false);
  });
});

test("requires package, Tauri, Cargo, tag, and approval versions to agree", () => {
  const cases: Array<[string, (root: string) => void]> = [
    ["Tauri", (root) => mutateJson(root, "src-tauri/tauri.conf.json", (value) => (value.version = "0.1.1"))],
    [
      "Cargo",
      (root) =>
        writeText(
          root,
          "src-tauri/Cargo.toml",
          readText(root, "src-tauri/Cargo.toml").replace('version = "0.1.0"', 'version = "0.1.1"'),
        ),
    ],
    [
      "approval",
      (root) =>
        mutateJson(root, "release/approvals/community-beta-v0.1.0.json", (value) => {
          value.version = "0.1.1";
        }),
    ],
  ];

  for (const [name, mutate] of cases) {
    withFixture((root) => {
      mutate(root);
      const result = runPreflight(root);
      assert.notEqual(result.status, 0, `${name} mismatch unexpectedly passed`);
    });
  }

  withFixture((root) => {
    const result = runPreflight(root, { tag: "v0.1.1-beta.1" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /does not match application version/);
  });
});

test("accepts beta tags only", () => {
  for (const tag of [
    "v0.1.0",
    "0.1.0-beta.1",
    "v0.1.0-rc.1",
    "v0.1.0-beta.0",
    "v0.1.0-beta.01",
  ]) {
    withFixture((root) => {
      const result = runPreflight(root, { tag });
      assert.notEqual(result.status, 0, `${tag} unexpectedly passed`);
      assert.match(result.stderr, /beta-only form/);
    });
  }
});

test("fails closed on approval schema, channel, strict approval, and false or missing confirmations", () => {
  const cases: Array<[string, (approval: Record<string, any>) => void]> = [
    ["schema", (approval) => (approval.schemaVersion = 2)],
    ["channel", (approval) => (approval.channel = "public")],
    ["strict", (approval) => (approval.strictPublicReleaseApproval = true)],
    ["game images", (approval) => (approval.releaseOwnerConfirmations.gameImagesMayBeDistributedInThisChannel = false)],
    ["brand icon", (approval) => delete approval.releaseOwnerConfirmations.projectBrandIconMayBeDistributedInThisChannel],
    ["disclaimer", (approval) => delete approval.releaseOwnerConfirmations.unofficialProjectDisclaimerApprovedForThisChannel],
    ["FFmpeg distribution", (approval) => (approval.releaseOwnerConfirmations.ffmpegMinimalBuildMayBeDistributedInThisChannel = false)],
    ["patent deferral", (approval) => delete approval.releaseOwnerConfirmations.codecPatentReviewDeferredToStrictRelease],
    ["distribution requirement", (approval) => (approval.distributionRequirements.communityBetaLimitationsMustBeDisclosed = false)],
    ["extra false confirmation", (approval) => (approval.releaseOwnerConfirmations.unreviewedExtraCondition = false)],
  ];

  for (const [name, mutate] of cases) {
    withFixture((root) => {
      mutateJson(root, "release/approvals/community-beta-v0.1.0.json", mutate);
      const result = runPreflight(root);
      assert.notEqual(result.status, 0, `${name} mutation unexpectedly passed`);
    });
  }
});

test("binds the Community Beta game-content scope to the exact manifest and asset bytes", () => {
  withFixture((root) => {
    mutateJson(
      root,
      "release/approvals/community-beta-v0.1.0-game-content-scope.json",
      (value) => {
        value.distributionScopes = value.distributionScopes.filter(
          (scope: string) => scope !== "public-windows-installer",
        );
      },
    );
    const result = runPreflight(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /distributionScopes/);
  });

  withFixture((root) => {
    writeText(root, "public/valorant-assets/agents/00.png", "tampered");
    const result = runPreflight(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /asset bytes do not match the manifest/);
  });

  withFixture((root) => {
    mutateJson(
      root,
      "release/approvals/community-beta-v0.1.0-game-content-scope.json",
      (value) => {
        value.baseRecordRelationship.doesNotAmendStrictPublicReleasePolicy = false;
      },
    );
    const result = runPreflight(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /narrow exception/);
  });
});

test("rejects updater enablement in npm, Cargo, Tauri config, or updater artifacts", () => {
  const cases: Array<[string, (root: string) => void]> = [
    [
      "npm manifest",
      (root) =>
        mutateJson(root, "package.json", (value) => {
          value.dependencies["@tauri-apps/plugin-updater"] = "^2";
        }),
    ],
    [
      "npm lock",
      (root) =>
        mutateJson(root, "package-lock.json", (value) => {
          value.packages["node_modules/@tauri-apps/plugin-updater"] = { version: "2.0.0" };
        }),
    ],
    [
      "Cargo",
      (root) =>
        writeText(
          root,
          "src-tauri/Cargo.toml",
          `${readText(root, "src-tauri/Cargo.toml")}tauri-plugin-updater = "2"\n`,
        ),
    ],
    [
      "Tauri plugin",
      (root) =>
        mutateJson(root, "src-tauri/tauri.conf.json", (value) => {
          value.plugins = { updater: { endpoints: ["https://example.invalid/update.json"] } };
        }),
    ],
    [
      "boolean updater artifacts",
      (root) =>
        mutateJson(root, "src-tauri/tauri.conf.json", (value) => {
          value.bundle.createUpdaterArtifacts = true;
        }),
    ],
    [
      "v1-compatible updater artifacts",
      (root) =>
        mutateJson(root, "src-tauri/tauri.conf.json", (value) => {
          value.bundle.createUpdaterArtifacts = "v1Compatible";
        }),
    ],
  ];

  for (const [name, mutate] of cases) {
    withFixture((root) => {
      mutate(root);
      const result = runPreflight(root);
      assert.notEqual(result.status, 0, `${name} unexpectedly passed`);
      assert.match(result.stderr, /updater/i);
    });
  }
});

test("requires every community-beta disclosure, including the unofficial disclaimer", () => {
  const sections = documentationSections();
  for (const omitted of Object.keys(sections)) {
    withFixture((root) => {
      writeText(
        root,
        "docs/COMMUNITY_BETA.md",
        Object.entries(sections)
          .filter(([name]) => name !== omitted)
          .map(([, text]) => text)
          .join("\n\n"),
      );
      const result = runPreflight(root);
      assert.notEqual(result.status, 0, `documentation without ${omitted} unexpectedly passed`);
      assert.match(result.stderr, /COMMUNITY_BETA\.md/);
    });
  }
});

test("refuses the beta exception once the strict public-release policy is ready", () => {
  withFixture((root) => {
    writeJson(root, "release/public-release-policy.json", readyPublicReleasePolicy());
    const result = runPreflight(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /strict public-release policy is already ready/);
  });
});

test("validates optional CI ref and default branch as a pair", () => {
  withFixture((root) => {
    const checkedOutCommit = initializeGitFixture(root);
    const success = runPreflight(root, {
      expectedSourceCommit: checkedOutCommit,
      githubRef: "refs/heads/main",
      defaultBranch: "main",
    });
    assert.equal(success.status, 0, success.stderr);
    assert.equal(JSON.parse(success.stdout).githubRef, "refs/heads/main");
  });

  withFixture((root) => {
    const missingPair = runPreflight(root, { githubRef: "refs/heads/main" });
    assert.notEqual(missingPair.status, 0);
    assert.match(missingPair.stderr, /must be provided together/);
  });

  withFixture((root) => {
    initializeGitFixture(root);
    const wrongBranch = runPreflight(root, {
      githubRef: "refs/heads/feature",
      defaultBranch: "main",
    });
    assert.notEqual(wrongBranch.status, 0);
    assert.match(wrongBranch.stderr, /refs\/heads\/main/);
  });

  withFixture((root) => {
    const checkedOutCommit = initializeGitFixture(root);
    const wrongCommit = checkedOutCommit === "b".repeat(40) ? "c".repeat(40) : "b".repeat(40);
    const mismatch = runPreflight(root, {
      expectedSourceCommit: wrongCommit,
      githubRef: "refs/heads/main",
      defaultBranch: "main",
    });
    assert.notEqual(mismatch.status, 0);
    assert.match(mismatch.stderr, /checked-out source commit/);
  });
});

function withFixture(callback: (root: string) => void) {
  const root = mkdtempSync(join(tmpdir(), "community-beta-preflight-"));
  try {
    createFixture(root);
    callback(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function createFixture(root: string) {
  writeJson(root, "package.json", {
    name: "fixture",
    version: "0.1.0",
    dependencies: { "@tauri-apps/api": "^2" },
    devDependencies: { "@tauri-apps/cli": "^2" },
  });
  writeJson(root, "package-lock.json", {
    name: "fixture",
    version: "0.1.0",
    lockfileVersion: 3,
    packages: { "": { name: "fixture", version: "0.1.0" } },
  });
  writeJson(root, "src-tauri/tauri.conf.json", {
    productName: "Fixture",
    version: "0.1.0",
    bundle: { icon: ["icons/icon.ico"] },
  });
  writeText(
    root,
    "src-tauri/Cargo.toml",
    '[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\ntauri = "2"\n',
  );
  writeText(root, "src-tauri/Cargo.lock", 'version = 4\n\n[[package]]\nname = "tauri"\nversion = "2.0.0"\n');
  writeText(root, "src-tauri/icons/icon.ico", "fixture-icon");
  writeText(root, "src-tauri/resources/licenses/ffmpeg/LICENSE.txt", "fixture license");
  writeText(root, "src-tauri/resources/licenses/ffmpeg/SOURCE-OFFER.md", "fixture source availability");
  const gameContent = createGameContentFixture(root);
  writeJson(root, "release/approvals/game-content-rights.json", gameContent.baseRecord);
  writeJson(
    root,
    "release/approvals/community-beta-v0.1.0-game-content-scope.json",
    gameContent.channelRecord,
  );
  writeJson(root, "release/approvals/community-beta-v0.1.0.json", betaApproval());
  writeText(root, "docs/COMMUNITY_BETA.md", Object.values(documentationSections()).join("\n\n"));
  writeJson(root, "release/public-release-policy.json", {
    schemaVersion: 1,
    releaseMode: "public",
    thirdPartyCompliance: { approved: false },
    updater: { decision: "pending", approvalReference: null },
  });
}

function betaApproval() {
  return {
    schemaVersion: 1,
    recordId: "community-beta-v0.1.0-release-owner-decision",
    version: "0.1.0",
    channel: "community-beta",
    decision: "approved-by-release-owner-for-community-beta",
    decisionAuthority: "repository-release-owner",
    strictPublicReleaseApproval: false,
    releaseOwnerConfirmations: {
      gameImagesMayBeDistributedInThisChannel: true,
      projectBrandIconMayBeDistributedInThisChannel: true,
      unofficialProjectDisclaimerApprovedForThisChannel: true,
      ffmpegMinimalBuildMayBeDistributedInThisChannel: true,
      codecPatentReviewDeferredToStrictRelease: true,
      automaticUpdatesAreDisabled: true,
      installerIsUnsigned: true,
      ffmpegUseIsLimitedToThumbnailGeneration: true,
    },
    distributionRequirements: {
      windowsUnsignedWarningMustBeDisclosed: true,
      manualUpdateInstructionsMustBeDisclosed: true,
      ffmpegLicenseMaterialsMustAccompanyInstaller: true,
      ffmpegBinaryAndBuildEvidenceMustAccompanyInstaller: true,
      ffmpegCorrespondingSourceMustAccompanyInstaller: true,
      communityBetaLimitationsMustBeDisclosed: true,
    },
    ffmpegMaterials: {
      licenseDirectory: "src-tauri/resources/licenses/ffmpeg",
      sourceAvailabilityDocument: "src-tauri/resources/licenses/ffmpeg/SOURCE-OFFER.md",
      correspondingSourceSidecarRequired: true,
    },
    gameContentRecord: "release/approvals/game-content-rights.json",
    gameContentChannelScopeRecord:
      "release/approvals/community-beta-v0.1.0-game-content-scope.json",
    userDocumentation: "docs/COMMUNITY_BETA.md",
  };
}

function createGameContentFixture(root: string) {
  const makeEntry = (category: string, index: number) => {
    const relativePath = `${category}/${String(index).padStart(2, "0")}.png`;
    const bytes = Buffer.from(`${category}-${index}`, "utf8");
    const absolutePath = join(root, "public", "valorant-assets", ...relativePath.split("/"));
    mkdirSync(dirname(absolutePath), { recursive: true });
    writeFileSync(absolutePath, bytes);
    return {
      relativePath,
      byteLength: bytes.length,
      sha256: sha256(bytes),
    };
  };
  const agents = Array.from({ length: 29 }, (_, index) => makeEntry("agents", index));
  const maps = Array.from({ length: 13 }, (_, index) => makeEntry("maps", index));
  const entries = [...agents, ...maps];
  const collectionFingerprint = sha256(
    Buffer.from(
      entries
        .map((entry) => `${entry.relativePath}\t${entry.byteLength}\t${entry.sha256}\n`)
        .sort()
        .join(""),
      "utf8",
    ),
  );
  const manifest = {
    schemaVersion: 2,
    sourceService: "https://valorant-api.com/",
    assetRoot: "public/valorant-assets",
    authorizationReference: "release/approvals/game-content-rights.json",
    retrievedAt: "2026-07-20",
    collectionFingerprint,
    agents,
    maps,
  };
  writeJson(root, "src/data/valorantAssets.json", manifest);
  const manifestBytes = readFileSync(join(root, "src", "data", "valorantAssets.json"));
  const manifestSha256 = sha256(manifestBytes);
  const totalBytes = entries.reduce((sum, entry) => sum + entry.byteLength, 0);
  const assetSet = {
    manifest: "src/data/valorantAssets.json",
    manifestSha256,
    assetRoot: "public/valorant-assets",
    assetCount: 42,
    totalBytes,
    collectionFingerprint,
    sourceService: "https://valorant-api.com/",
    retrievedAt: "2026-07-20",
  };
  return {
    baseRecord: {
      schemaVersion: 1,
      ownerAttestationReceived: true,
      sourceDocumentReviewed: false,
      legalReviewApproved: false,
      assetSet,
    },
    channelRecord: {
      schemaVersion: 1,
      recordId: "community-beta-v0.1.0-game-content-channel-scope",
      version: "0.1.0",
      channel: "community-beta",
      status: "approved-by-release-owner-for-community-beta",
      releaseOwnerAuthorization: {
        authority: "repository-release-owner",
        explicitChannelAuthorizationReceived: true,
        recordedOn: "2026-07-21",
        strictPublicReleaseApproval: false,
      },
      baseRightsRecord: "release/approvals/game-content-rights.json",
      baseRecordRelationship: {
        preservesPendingSourceEvidenceAndLegalReviewState: true,
        channelSpecificDistributionException: true,
        appliesOnlyToThisVersionAndChannel: true,
        doesNotAmendStrictPublicReleasePolicy: true,
      },
      distributionScopes: [
        "github-public-prerelease",
        "public-release-artifact-download",
        "public-windows-installer",
        "in-app-display",
      ],
      channelRestrictions: {
        nonCommercialCommunityTestingOnly: true,
        sourceAssetBytesMustRemainManifestExact: true,
        noSublicensing: true,
        noStandaloneDerivativeAssetFiles: true,
        unofficialProjectDisclaimerRequired: true,
      },
      assetSet,
      requiredPublicStatements: [
        "not-official",
        "not-affiliated",
        "not-sponsored",
        "not-endorsed",
        "game-content-not-covered-by-mit",
      ],
      nonAssertions: [
        "no-riot-games-approval-claimed",
        "no-tencent-approval-claimed",
        "no-other-third-party-approval-claimed",
        "no-official-affiliation-sponsorship-or-endorsement-claimed",
        "no-independent-legal-review-completed",
        "no-strict-public-release-approval",
      ],
    },
  };
}

function initializeGitFixture(root: string) {
  for (const args of [
    ["init", "--initial-branch=main"],
    ["config", "user.name", "Community Beta Test"],
    ["config", "user.email", "community-beta-test@example.invalid"],
    ["add", "."],
    ["commit", "--no-gpg-sign", "-m", "fixture"],
  ]) {
    const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  }
  const head = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(head.status, 0, head.stderr);
  return head.stdout.trim().toLowerCase();
}

function sha256(value: Buffer) {
  return createHash("sha256").update(value).digest("hex");
}

function documentationSections() {
  return {
    unsigned: "安装包尚未进行 Authenticode 签名，Windows 可能显示未知发布者。",
    manualUpdate: "Community Beta 没有自动更新，请手动更新并查看项目发布页。",
    formalRelease:
      "Community Beta 不代表严格正式发布，`release/public-release-policy.json` 的门禁仍独立跟踪。",
    ffmpegSource: "FFmpeg 对应源码 sidecar 应与安装包一同提供。",
    disclaimer:
      "瓦刻是非官方社区项目，与 Riot Games、腾讯不存在隶属、赞助或认可关系。",
  };
}

function readyPublicReleasePolicy() {
  return {
    schemaVersion: 1,
    releaseMode: "public",
    projectLicense: { approved: true },
    eula: { required: false, approved: true },
    thirdPartyCompliance: { approved: true },
    identity: {
      brandApproved: true,
      publisherApproved: true,
      identifierApproved: true,
    },
    gameContentRights: { approved: true },
    iconRights: { approved: true },
    riotTencentDisclaimer: { approved: true },
    authenticode: {
      certificateProvisioned: true,
      expectedPublisherSubject: "CN=Fixture",
      expectedCertificateThumbprint: "b".repeat(40),
      timestampUrl: "https://timestamp.example.invalid",
    },
    cleanVmValidation: {
      approved: true,
      evidenceManifest: "release/evidence/clean-vm.json",
      sourceCommit,
    },
    updater: {
      decision: "disabled",
      approvalReference: "release/approvals/updater.json",
    },
    dataSafety: {
      approved: true,
      evidenceManifest: "release/evidence/data-safety.json",
      sourceCommit,
    },
  };
}

function runPreflight(
  root: string,
  overrides: {
    tag?: string;
    expectedSourceCommit?: string;
    githubRef?: string;
    defaultBranch?: string;
  } = {},
) {
  const args = [
    scriptPath,
    "--tag",
    overrides.tag ?? "v0.1.0-beta.2",
    "--expected-source-commit",
    overrides.expectedSourceCommit ?? sourceCommit,
    "--repository-root",
    root,
  ];
  if (overrides.githubRef !== undefined) {
    args.push("--github-ref", overrides.githubRef);
  }
  if (overrides.defaultBranch !== undefined) {
    args.push("--default-branch", overrides.defaultBranch);
  }
  return spawnSync(process.execPath, args, { encoding: "utf8" });
}

function mutateJson(
  root: string,
  relativePath: string,
  mutate: (value: Record<string, any>) => void,
) {
  const value = JSON.parse(readText(root, relativePath));
  mutate(value);
  writeJson(root, relativePath, value);
}

function readText(root: string, relativePath: string) {
  return readFileSync(join(root, ...relativePath.split("/")), "utf8");
}

function writeJson(root: string, relativePath: string, value: unknown) {
  writeText(root, relativePath, `${JSON.stringify(value, null, 2)}\n`);
}

function writeText(root: string, relativePath: string, contents: string) {
  const path = join(root, ...relativePath.split("/"));
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, contents, "utf8");
}
