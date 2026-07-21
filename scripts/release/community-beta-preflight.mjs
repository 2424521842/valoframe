#!/usr/bin/env node

import {
  existsSync,
  lstatSync,
  readFileSync,
  realpathSync,
  statSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { isAbsolute, relative, resolve, sep } from "node:path";

const REQUIRED_CONFIRMATIONS = [
  "gameImagesMayBeDistributedInThisChannel",
  "projectBrandIconMayBeDistributedInThisChannel",
  "unofficialProjectDisclaimerApprovedForThisChannel",
  "ffmpegMinimalBuildMayBeDistributedInThisChannel",
  "codecPatentReviewDeferredToStrictRelease",
  "automaticUpdatesAreDisabled",
  "installerIsUnsigned",
  "ffmpegUseIsLimitedToThumbnailGeneration",
];

const REQUIRED_DISTRIBUTION_REQUIREMENTS = [
  "windowsUnsignedWarningMustBeDisclosed",
  "manualUpdateInstructionsMustBeDisclosed",
  "ffmpegLicenseMaterialsMustAccompanyInstaller",
  "ffmpegBinaryAndBuildEvidenceMustAccompanyInstaller",
  "ffmpegCorrespondingSourceMustAccompanyInstaller",
  "communityBetaLimitationsMustBeDisclosed",
];
const GAME_CONTENT_DISTRIBUTION_SCOPES = [
  "github-public-prerelease",
  "public-release-artifact-download",
  "public-windows-installer",
  "in-app-display",
];
const GAME_CONTENT_REQUIRED_PUBLIC_STATEMENTS = [
  "not-official",
  "not-affiliated",
  "not-sponsored",
  "not-endorsed",
  "game-content-not-covered-by-mit",
];
const GAME_CONTENT_NON_ASSERTIONS = [
  "no-riot-games-approval-claimed",
  "no-tencent-approval-claimed",
  "no-other-third-party-approval-claimed",
  "no-official-affiliation-sponsorship-or-endorsement-claimed",
  "no-independent-legal-review-completed",
  "no-strict-public-release-approval",
];
const SHA256_PATTERN = /^[0-9a-f]{64}$/;

try {
  const options = parseArguments(process.argv.slice(2));
  const result = runPreflight(options);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`community-beta preflight failed: ${message}\n`);
  process.exitCode = 1;
}

function runPreflight(options) {
  const repositoryRoot = resolve(options.repositoryRoot);
  assertDirectory(repositoryRoot, "repository root");
  const canonicalRoot = realpathSync(repositoryRoot);

  const packageJson = readRepositoryJson(canonicalRoot, "package.json", "package.json");
  const tauriConfig = readRepositoryJson(
    canonicalRoot,
    "src-tauri/tauri.conf.json",
    "Tauri configuration",
  );
  const cargoToml = readRepositoryText(
    canonicalRoot,
    "src-tauri/Cargo.toml",
    "Cargo manifest",
  );

  const packageVersion = requireVersion(packageJson.version, "package.json version");
  const tauriVersion = requireVersion(tauriConfig.version, "Tauri version");
  const cargoVersion = requireVersion(
    parseCargoPackageVersion(cargoToml),
    "Cargo package version",
  );

  assert(
    packageVersion === tauriVersion && packageVersion === cargoVersion,
    `version mismatch: package.json=${packageVersion}, Tauri=${tauriVersion}, Cargo=${cargoVersion}`,
  );

  const tag = parseBetaTag(options.tag);
  assert(
    tag.version === packageVersion,
    `tag ${options.tag} does not match application version ${packageVersion}`,
  );

  assert(
    /^[0-9a-f]{40}$/i.test(options.expectedSourceCommit),
    "--expected-source-commit must be exactly 40 hexadecimal characters",
  );
  const sourceCommit = options.expectedSourceCommit.toLowerCase();
  validateCiBranchOptions(options.githubRef, options.defaultBranch);
  if (options.githubRef !== undefined) {
    const checkedOutCommit = readCheckedOutCommit(canonicalRoot);
    assert(
      checkedOutCommit === sourceCommit,
      `checked-out source commit ${checkedOutCommit} does not match --expected-source-commit ${sourceCommit}`,
    );
  }

  const approvalPath = `release/approvals/community-beta-v${packageVersion}.json`;
  const approval = readRepositoryJson(
    canonicalRoot,
    approvalPath,
    "community beta release-owner decision",
  );
  validateApproval(approval, packageVersion);

  const gameContent = validateGameContentChannelScope(
    canonicalRoot,
    approval,
    packageVersion,
  );

  const documentationPath = requireSafeRelativeString(
    approval.userDocumentation,
    "userDocumentation",
  );
  assert(
    documentationPath === "docs/COMMUNITY_BETA.md",
    "userDocumentation must be docs/COMMUNITY_BETA.md",
  );
  const documentation = readRepositoryText(
    canonicalRoot,
    documentationPath,
    "community beta documentation",
  );
  validateCommunityBetaDocumentation(documentation);

  validateFfmpegMaterials(canonicalRoot, approval.ffmpegMaterials);
  const updater = validateUpdaterDisabled(canonicalRoot, packageJson, tauriConfig, cargoToml);
  validateProjectBrandIcons(canonicalRoot, tauriConfig);

  const publicPolicy = readRepositoryJson(
    canonicalRoot,
    "release/public-release-policy.json",
    "strict public-release policy",
  );
  const publicReleaseAssessment = assessStrictPublicReleasePolicy(publicPolicy);
  assert(
    publicReleaseAssessment.ready === false,
    "strict public-release policy is already ready; the community-beta exception must not be used",
  );

  return {
    schemaVersion: 1,
    status: "ready-for-community-beta-build",
    releaseChannel: "community-beta",
    version: packageVersion,
    tag: options.tag,
    betaSequence: tag.sequence,
    sourceCommit,
    githubRef: options.githubRef ?? null,
    defaultBranch: options.defaultBranch ?? null,
    strictPublicReleaseApproved: false,
    publicReleasePolicyReady: false,
    publicReleaseBlockingSignals: publicReleaseAssessment.blockingSignals,
    updater,
    approval: {
      path: approvalPath,
      decision: approval.decision,
      authority: approval.decisionAuthority,
      releaseOwnerConfirmationsVerified: REQUIRED_CONFIRMATIONS,
      distributionRequirementsVerified: REQUIRED_DISTRIBUTION_REQUIREMENTS,
    },
    gameContent,
    documentation: {
      path: documentationPath,
      unsignedInstallerDisclosed: true,
      manualUpdatesDisclosed: true,
      notFormalPublicReleaseDisclosed: true,
      ffmpegSourceSidecarDisclosed: true,
      unofficialProjectDisclaimerDisclosed: true,
    },
  };
}

function validateGameContentChannelScope(repositoryRoot, approval, version) {
  const baseRecordPath = requireSafeRelativeString(
    approval.gameContentRecord,
    "gameContentRecord",
  );
  assert(
    baseRecordPath === "release/approvals/game-content-rights.json",
    "gameContentRecord must be release/approvals/game-content-rights.json",
  );
  const baseRecord = readRepositoryJson(
    repositoryRoot,
    baseRecordPath,
    "game-content rights record",
  );
  assert(
    baseRecord.schemaVersion === 1 &&
      baseRecord.ownerAttestationReceived === true &&
      baseRecord.sourceDocumentReviewed === false &&
      baseRecord.legalReviewApproved === false,
    "base game-content record must preserve the owner attestation and pending independent review state",
  );

  const channelRecordPath = requireSafeRelativeString(
    approval.gameContentChannelScopeRecord,
    "gameContentChannelScopeRecord",
  );
  const expectedChannelRecordPath =
    `release/approvals/community-beta-v${version}-game-content-scope.json`;
  assert(
    channelRecordPath === expectedChannelRecordPath,
    `gameContentChannelScopeRecord must be ${expectedChannelRecordPath}`,
  );
  const channelRecord = readRepositoryJson(
    repositoryRoot,
    channelRecordPath,
    "Community Beta game-content channel scope record",
  );
  assert(
    channelRecord.schemaVersion === 1 &&
      channelRecord.recordId === `community-beta-v${version}-game-content-channel-scope` &&
      channelRecord.version === version &&
      channelRecord.channel === "community-beta" &&
      channelRecord.status === "approved-by-release-owner-for-community-beta",
    "Community Beta game-content channel scope identity is invalid",
  );
  assert(
    channelRecord.baseRightsRecord === baseRecordPath,
    "Community Beta game-content channel scope does not reference the base rights record",
  );
  const baseRelationship = channelRecord.baseRecordRelationship;
  assert(
    isPlainObject(baseRelationship) &&
      baseRelationship.preservesPendingSourceEvidenceAndLegalReviewState === true &&
      baseRelationship.channelSpecificDistributionException === true &&
      baseRelationship.appliesOnlyToThisVersionAndChannel === true &&
      baseRelationship.doesNotAmendStrictPublicReleasePolicy === true,
    "Community Beta game-content channel record must remain a narrow exception without rewriting the base or strict-public state",
  );

  const ownerAuthorization = channelRecord.releaseOwnerAuthorization;
  assert(
    isPlainObject(ownerAuthorization) &&
      ownerAuthorization.authority === "repository-release-owner" &&
      ownerAuthorization.explicitChannelAuthorizationReceived === true &&
      /^\d{4}-\d{2}-\d{2}$/.test(ownerAuthorization.recordedOn ?? "") &&
      ownerAuthorization.strictPublicReleaseApproval === false,
    "Community Beta game-content release-owner authorization is incomplete or overclaims strict approval",
  );
  assertExactStringSet(
    channelRecord.distributionScopes,
    GAME_CONTENT_DISTRIBUTION_SCOPES,
    "Community Beta game-content distributionScopes",
  );
  assertExactStringSet(
    channelRecord.requiredPublicStatements,
    GAME_CONTENT_REQUIRED_PUBLIC_STATEMENTS,
    "Community Beta game-content requiredPublicStatements",
  );
  assertExactStringSet(
    channelRecord.nonAssertions,
    GAME_CONTENT_NON_ASSERTIONS,
    "Community Beta game-content nonAssertions",
  );

  const restrictions = channelRecord.channelRestrictions;
  assert(
    isPlainObject(restrictions) &&
      restrictions.nonCommercialCommunityTestingOnly === true &&
      restrictions.sourceAssetBytesMustRemainManifestExact === true &&
      restrictions.noSublicensing === true &&
      restrictions.noStandaloneDerivativeAssetFiles === true &&
      restrictions.unofficialProjectDisclaimerRequired === true,
    "Community Beta game-content channel restrictions are incomplete",
  );

  const assetSet = channelRecord.assetSet;
  assert(isPlainObject(assetSet), "Community Beta game-content assetSet must be an object");
  const manifestPath = requireSafeRelativeString(
    assetSet.manifest,
    "Community Beta game-content assetSet.manifest",
  );
  assert(
    manifestPath === "src/data/valorantAssets.json",
    "Community Beta game-content manifest must be src/data/valorantAssets.json",
  );
  const manifestFile = resolveRepositoryEntry(
    repositoryRoot,
    manifestPath,
    "VALORANT asset manifest",
    "file",
  );
  const manifestBytes = readFileSync(manifestFile);
  let manifest;
  try {
    manifest = JSON.parse(manifestBytes.toString("utf8"));
  } catch (error) {
    throw new Error(`VALORANT asset manifest is not valid JSON: ${error.message}`);
  }
  assert(
    manifest.schemaVersion === 2 &&
      manifest.sourceService === "https://valorant-api.com/" &&
      manifest.assetRoot === "public/valorant-assets" &&
      manifest.authorizationReference === baseRecordPath &&
      /^\d{4}-\d{2}-\d{2}$/.test(manifest.retrievedAt ?? ""),
    "VALORANT asset manifest identity or source snapshot is invalid",
  );

  const entries = [];
  const paths = new Set();
  for (const [category, expectedCount] of [
    ["agents", 29],
    ["maps", 13],
  ]) {
    const categoryEntries = manifest[category];
    assert(
      Array.isArray(categoryEntries) && categoryEntries.length === expectedCount,
      `VALORANT asset manifest ${category} must contain ${expectedCount} entries`,
    );
    for (const entry of categoryEntries) {
      assert(isPlainObject(entry), `VALORANT asset manifest ${category} entry must be an object`);
      const relativePath = requireSafeRelativeString(
        entry.relativePath,
        `VALORANT asset manifest ${category} relativePath`,
      );
      assert(!paths.has(relativePath), `duplicate VALORANT asset path: ${relativePath}`);
      paths.add(relativePath);
      assert(
        Number.isSafeInteger(entry.byteLength) && entry.byteLength > 0,
        `invalid VALORANT asset byteLength: ${relativePath}`,
      );
      assert(
        typeof entry.sha256 === "string" && SHA256_PATTERN.test(entry.sha256),
        `invalid VALORANT asset SHA-256: ${relativePath}`,
      );
      const assetFile = resolveRepositoryEntry(
        repositoryRoot,
        `${manifest.assetRoot}/${relativePath}`,
        `VALORANT asset '${relativePath}'`,
        "file",
      );
      const assetBytes = readFileSync(assetFile);
      assert(
        assetBytes.length === entry.byteLength && sha256(assetBytes) === entry.sha256,
        `VALORANT asset bytes do not match the manifest: ${relativePath}`,
      );
      entries.push(entry);
    }
  }
  assert(entries.length === 42, "VALORANT asset manifest must contain exactly 42 entries");
  const totalBytes = entries.reduce((sum, entry) => sum + entry.byteLength, 0);
  const fingerprintPayload = entries
    .map((entry) => `${entry.relativePath}\t${entry.byteLength}\t${entry.sha256}\n`)
    .sort()
    .join("");
  const collectionFingerprint = sha256(Buffer.from(fingerprintPayload, "utf8"));
  const manifestSha256 = sha256(manifestBytes);
  assert(
    manifest.collectionFingerprint === collectionFingerprint,
    "VALORANT asset manifest collection fingerprint is invalid",
  );
  assert(
    assetSet.manifestSha256 === manifestSha256 &&
      assetSet.assetRoot === manifest.assetRoot &&
      assetSet.assetCount === entries.length &&
      assetSet.totalBytes === totalBytes &&
      assetSet.collectionFingerprint === collectionFingerprint &&
      assetSet.sourceService === manifest.sourceService &&
      assetSet.retrievedAt === manifest.retrievedAt,
    "Community Beta game-content channel record does not match the current asset manifest",
  );
  const baseAssetSet = baseRecord.assetSet;
  assert(
    isPlainObject(baseAssetSet) &&
      baseAssetSet.manifest === manifestPath &&
      baseAssetSet.manifestSha256 === manifestSha256 &&
      baseAssetSet.assetCount === entries.length &&
      baseAssetSet.totalBytes === totalBytes &&
      baseAssetSet.collectionFingerprint === collectionFingerprint &&
      baseAssetSet.sourceService === manifest.sourceService,
    "base game-content rights record does not match the current asset manifest",
  );

  return {
    baseRecordPath,
    channelScopeRecordPath: channelRecordPath,
    manifestPath,
    manifestSha256,
    assetCount: entries.length,
    totalBytes,
    collectionFingerprint,
    sourceService: manifest.sourceService,
    distributionScopes: GAME_CONTENT_DISTRIBUTION_SCOPES,
    thirdPartyApprovalClaimed: false,
    independentLegalReviewCompleted: false,
  };
}

function readCheckedOutCommit(repositoryRoot) {
  let output;
  try {
    output = execFileSync(
      "git",
      ["-C", repositoryRoot, "rev-parse", "--verify", "HEAD"],
      { encoding: "utf8", windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch {
    throw new Error("CI source verification requires repository root to be a Git checkout with HEAD");
  }
  const commit = output.trim().toLowerCase();
  assert(/^[0-9a-f]{40}$/.test(commit), "git rev-parse HEAD did not return a full commit SHA");
  return commit;
}

function parseArguments(args) {
  const known = new Set([
    "--tag",
    "--expected-source-commit",
    "--repository-root",
    "--github-ref",
    "--default-branch",
  ]);
  const values = new Map();

  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    assert(known.has(name), `unknown argument: ${name ?? "<missing>"}`);
    assert(value !== undefined && !value.startsWith("--"), `missing value for ${name}`);
    assert(!values.has(name), `duplicate argument: ${name}`);
    values.set(name, value);
  }

  for (const required of ["--tag", "--expected-source-commit", "--repository-root"]) {
    assert(values.has(required), `missing required argument: ${required}`);
  }

  return {
    tag: values.get("--tag"),
    expectedSourceCommit: values.get("--expected-source-commit"),
    repositoryRoot: values.get("--repository-root"),
    githubRef: values.get("--github-ref"),
    defaultBranch: values.get("--default-branch"),
  };
}

function parseBetaTag(value) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-beta\.([1-9]\d*)$/.exec(
    value,
  );
  assert(
    match,
    "--tag must use the beta-only form vMAJOR.MINOR.PATCH-beta.N with N greater than zero",
  );
  return {
    version: `${match[1]}.${match[2]}.${match[3]}`,
    sequence: Number(match[4]),
  };
}

function validateCiBranchOptions(githubRef, defaultBranch) {
  assert(
    (githubRef === undefined) === (defaultBranch === undefined),
    "--github-ref and --default-branch must be provided together",
  );
  if (githubRef === undefined) {
    return;
  }

  assert(
    /^[A-Za-z0-9._/-]+$/.test(defaultBranch) &&
      !defaultBranch.startsWith("/") &&
      !defaultBranch.endsWith("/") &&
      !defaultBranch.includes("..") &&
      !defaultBranch.startsWith("refs/"),
    "--default-branch is not a valid branch name",
  );
  assert(
    githubRef === `refs/heads/${defaultBranch}`,
    `--github-ref must identify the default branch refs/heads/${defaultBranch}`,
  );
}

function validateApproval(approval, version) {
  assert(approval.schemaVersion === 1, "community beta decision schemaVersion must be 1");
  assert(approval.version === version, "community beta decision version does not match the application");
  assert(approval.channel === "community-beta", "community beta decision channel must be community-beta");
  assert(
    approval.decision === "approved-by-release-owner-for-community-beta",
    "community beta decision is not release-owner approved for this channel",
  );
  assert(
    approval.decisionAuthority === "repository-release-owner",
    "community beta decision authority must be repository-release-owner",
  );
  assert(
    approval.strictPublicReleaseApproval === false,
    "strictPublicReleaseApproval must be explicitly false",
  );

  assertAllBooleanTrue(
    approval.releaseOwnerConfirmations,
    "releaseOwnerConfirmations",
    REQUIRED_CONFIRMATIONS,
  );
  assertAllBooleanTrue(
    approval.distributionRequirements,
    "distributionRequirements",
    REQUIRED_DISTRIBUTION_REQUIREMENTS,
  );
}

function assertAllBooleanTrue(value, label, requiredKeys) {
  assert(isPlainObject(value), `${label} must be an object`);
  const entries = Object.entries(value);
  assert(entries.length > 0, `${label} must not be empty`);
  for (const key of requiredKeys) {
    assert(Object.hasOwn(value, key), `${label}.${key} is required`);
  }
  for (const [key, fieldValue] of entries) {
    assert(fieldValue === true, `${label}.${key} must be true`);
  }
}

function validateFfmpegMaterials(repositoryRoot, materials) {
  assert(isPlainObject(materials), "ffmpegMaterials must be an object");
  assert(
    materials.correspondingSourceSidecarRequired === true,
    "ffmpegMaterials.correspondingSourceSidecarRequired must be true",
  );
  const licenseDirectory = requireSafeRelativeString(
    materials.licenseDirectory,
    "ffmpegMaterials.licenseDirectory",
  );
  const sourceDocument = requireSafeRelativeString(
    materials.sourceAvailabilityDocument,
    "ffmpegMaterials.sourceAvailabilityDocument",
  );
  resolveRepositoryEntry(repositoryRoot, licenseDirectory, "FFmpeg license directory", "directory");
  resolveRepositoryEntry(repositoryRoot, sourceDocument, "FFmpeg source availability document", "file");
}

function validateUpdaterDisabled(repositoryRoot, packageJson, tauriConfig, cargoToml) {
  for (const sectionName of [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
  ]) {
    const section = packageJson[sectionName];
    if (isPlainObject(section)) {
      assert(
        !Object.hasOwn(section, "@tauri-apps/plugin-updater"),
        `npm ${sectionName} must not include @tauri-apps/plugin-updater`,
      );
    }
  }

  const packageLockPath = "package-lock.json";
  if (repositoryEntryExists(repositoryRoot, packageLockPath)) {
    const packageLock = readRepositoryJson(repositoryRoot, packageLockPath, "package-lock.json");
    assert(
      !serializedValueContainsUpdater(packageLock),
      "package-lock.json must not include @tauri-apps/plugin-updater",
    );
  }

  assert(
    !/(^|[^A-Za-z0-9_-])tauri-plugin-updater([^A-Za-z0-9_-]|$)/im.test(cargoToml),
    "Cargo.toml must not include tauri-plugin-updater",
  );
  const cargoLockPath = "src-tauri/Cargo.lock";
  if (repositoryEntryExists(repositoryRoot, cargoLockPath)) {
    const cargoLock = readRepositoryText(repositoryRoot, cargoLockPath, "Cargo.lock");
    assert(
      !/(^|[^A-Za-z0-9_-])tauri-plugin-updater([^A-Za-z0-9_-]|$)/im.test(cargoLock),
      "Cargo.lock must not include tauri-plugin-updater",
    );
  }

  const plugins = tauriConfig.plugins;
  assert(
    !isPlainObject(plugins) || !Object.hasOwn(plugins, "updater"),
    "Tauri configuration must not enable the updater plugin",
  );
  const createUpdaterArtifacts = tauriConfig.bundle?.createUpdaterArtifacts;
  assert(
    createUpdaterArtifacts === undefined || createUpdaterArtifacts === false,
    "Tauri bundle.createUpdaterArtifacts must be absent or false",
  );

  return {
    enabled: false,
    npmPluginPresent: false,
    cargoPluginPresent: false,
    tauriPluginConfigured: false,
    createUpdaterArtifacts: false,
  };
}

function serializedValueContainsUpdater(value) {
  const serialized = JSON.stringify(value).toLowerCase();
  return (
    serialized.includes('"@tauri-apps/plugin-updater"') ||
    serialized.includes("node_modules/@tauri-apps/plugin-updater")
  );
}

function validateProjectBrandIcons(repositoryRoot, tauriConfig) {
  const icons = tauriConfig.bundle?.icon;
  assert(Array.isArray(icons) && icons.length > 0, "Tauri bundle.icon must list project brand icons");
  for (const icon of icons) {
    assert(typeof icon === "string" && icon.length > 0, "each Tauri bundle icon must be a path");
    resolveRepositoryEntry(
      repositoryRoot,
      `src-tauri/${requireSafeRelativeString(icon, "Tauri bundle icon")}`,
      `project brand icon ${icon}`,
      "file",
    );
  }
}

function validateCommunityBetaDocumentation(markdown) {
  const normalized = markdown.replace(/\r\n/g, "\n");
  const lower = normalized.toLowerCase();

  assert(
    lower.includes("authenticode") && /(尚未|没有|未).{0,16}签名/u.test(normalized),
    "docs/COMMUNITY_BETA.md must explicitly disclose that the installer is unsigned",
  );
  assert(
    /(没有|不含|不会|未启用).{0,12}自动更新/u.test(normalized) &&
      /手动.{0,12}(更新|查看|下载)/u.test(normalized),
    "docs/COMMUNITY_BETA.md must explicitly disclose manual updates and no automatic updater",
  );
  assert(
    /(不代表|不等于|并非|不是).{0,40}(严格)?(正式|公开)发布/u.test(normalized) &&
      normalized.includes("release/public-release-policy.json"),
    "docs/COMMUNITY_BETA.md must state that the beta is not the formal public release",
  );
  assert(
    lower.includes("ffmpeg") &&
      /对应源码/u.test(normalized) &&
      /(一同|同时|伴随|sidecar)/iu.test(normalized),
    "docs/COMMUNITY_BETA.md must disclose the FFmpeg corresponding-source sidecar",
  );
  assert(
    /非官方/u.test(normalized) &&
      lower.includes("riot games") &&
      /腾讯/u.test(normalized) &&
      /(不存在|没有|无).{0,24}(隶属|关联|赞助|认可)/u.test(normalized),
    "docs/COMMUNITY_BETA.md must contain the unofficial and non-affiliation disclaimer",
  );
}

function assessStrictPublicReleasePolicy(policy) {
  assert(policy.schemaVersion === 1, "public-release-policy schemaVersion must be 1");
  assert(policy.releaseMode === "public", "public-release-policy releaseMode must be public");

  const checks = [
    ["project-license", policy.projectLicense?.approved === true],
    ["eula", policy.eula?.required === false || policy.eula?.approved === true],
    ["third-party-compliance", policy.thirdPartyCompliance?.approved === true],
    [
      "identity",
      policy.identity?.brandApproved === true &&
        policy.identity?.publisherApproved === true &&
        policy.identity?.identifierApproved === true,
    ],
    ["game-content-rights", policy.gameContentRights?.approved === true],
    ["icon-rights", policy.iconRights?.approved === true],
    ["unofficial-disclaimer", policy.riotTencentDisclaimer?.approved === true],
    [
      "authenticode-and-timestamp",
      policy.authenticode?.certificateProvisioned === true &&
        nonEmptyString(policy.authenticode?.expectedPublisherSubject) &&
        nonEmptyString(policy.authenticode?.expectedCertificateThumbprint) &&
        nonEmptyString(policy.authenticode?.timestampUrl),
    ],
    [
      "clean-vm-validation",
      policy.cleanVmValidation?.approved === true &&
        nonEmptyString(policy.cleanVmValidation?.evidenceManifest) &&
        /^[0-9a-f]{40}$/i.test(policy.cleanVmValidation?.sourceCommit ?? ""),
    ],
    ["updater-decision", isApprovedUpdaterDecision(policy.updater)],
    [
      "data-safety",
      policy.dataSafety?.approved === true &&
        nonEmptyString(policy.dataSafety?.evidenceManifest) &&
        /^[0-9a-f]{40}$/i.test(policy.dataSafety?.sourceCommit ?? ""),
    ],
  ];
  const blockingSignals = checks.filter(([, ready]) => !ready).map(([name]) => name);
  return { ready: blockingSignals.length === 0, blockingSignals };
}

function isApprovedUpdaterDecision(updater) {
  if (!isPlainObject(updater) || !nonEmptyString(updater.approvalReference)) {
    return false;
  }
  if (updater.decision === "disabled") {
    return true;
  }
  return (
    updater.decision === "enabled" &&
    nonEmptyString(updater.endpoint) &&
    nonEmptyString(updater.publicKeyReference)
  );
}

function parseCargoPackageVersion(cargoToml) {
  let inPackageSection = false;
  let foundPackageSection = false;
  for (const line of cargoToml.split(/\r?\n/)) {
    const section = /^\s*\[([^\]]+)]\s*(?:#.*)?$/.exec(line)?.[1];
    if (section !== undefined) {
      inPackageSection = section === "package";
      foundPackageSection ||= inPackageSection;
      continue;
    }
    if (inPackageSection) {
      const version = /^\s*version\s*=\s*["']([^"']+)["']\s*(?:#.*)?$/.exec(line)?.[1];
      if (version !== undefined) {
        return version;
      }
    }
  }
  assert(foundPackageSection, "Cargo.toml is missing a [package] section");
  throw new Error("Cargo.toml [package] is missing version");
}

function requireVersion(value, label) {
  assert(
    typeof value === "string" && /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value),
    `${label} must be a stable MAJOR.MINOR.PATCH version`,
  );
  return value;
}

function readRepositoryJson(repositoryRoot, relativePath, label) {
  const text = readRepositoryText(repositoryRoot, relativePath, label);
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`);
  }
}

function readRepositoryText(repositoryRoot, relativePath, label) {
  const filePath = resolveRepositoryEntry(repositoryRoot, relativePath, label, "file");
  return readFileSync(filePath, "utf8");
}

function repositoryEntryExists(repositoryRoot, relativePath) {
  const normalized = requireSafeRelativeString(relativePath, "repository path");
  return existsSync(resolve(repositoryRoot, ...normalized.split("/")));
}

function resolveRepositoryEntry(repositoryRoot, relativePath, label, expectedType) {
  const normalized = requireSafeRelativeString(relativePath, label);
  const candidate = resolve(repositoryRoot, ...normalized.split("/"));
  assertPathInside(repositoryRoot, candidate, label);
  assert(existsSync(candidate), `${label} does not exist: ${normalized}`);
  assert(!lstatSync(candidate).isSymbolicLink(), `${label} must not be a symbolic link`);
  const canonical = realpathSync(candidate);
  assertPathInside(repositoryRoot, canonical, label);
  const statistics = statSync(canonical);
  if (expectedType === "file") {
    assert(statistics.isFile(), `${label} must be a file`);
  } else {
    assert(statistics.isDirectory(), `${label} must be a directory`);
  }
  return canonical;
}

function requireSafeRelativeString(value, label) {
  assert(typeof value === "string" && value.trim() === value && value.length > 0, `${label} must be a non-empty path`);
  assert(!isAbsolute(value), `${label} must be repository-relative`);
  const normalized = value.replaceAll("\\", "/");
  const segments = normalized.split("/");
  assert(
    segments.every((segment) => segment.length > 0 && segment !== "." && segment !== ".."),
    `${label} contains an unsafe path segment`,
  );
  assert(!segments[0].includes(":"), `${label} must not contain a drive prefix`);
  return normalized;
}

function assertPathInside(repositoryRoot, candidate, label) {
  const pathFromRoot = relative(repositoryRoot, candidate);
  assert(
    pathFromRoot === "" ||
      (!pathFromRoot.startsWith(`..${sep}`) && pathFromRoot !== ".." && !isAbsolute(pathFromRoot)),
    `${label} resolves outside the repository root`,
  );
}

function assertDirectory(path, label) {
  assert(existsSync(path), `${label} does not exist: ${path}`);
  assert(statSync(path).isDirectory(), `${label} is not a directory: ${path}`);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function assertExactStringSet(value, expected, label) {
  assert(
    Array.isArray(value) &&
      value.length === expected.length &&
      new Set(value).size === value.length &&
      value.every((entry) => typeof entry === "string" && entry.trim() === entry) &&
      expected.every((entry) => value.includes(entry)),
    `${label} must match the exact required set`,
  );
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
