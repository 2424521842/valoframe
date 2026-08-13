#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..", "..");
const options = parseArguments(process.argv.slice(2));
const outputDirectory = resolve(repositoryRoot, options.output);
const targetTriple = options.target ?? "x86_64-pc-windows-msvc";
const releaseProfile = options.releaseProfile ?? "public";
const generatedAt = new Date().toISOString();
const licenseTextCatalog = new Map();

assertSafeOutputDirectory(outputDirectory);
mkdirSync(outputDirectory, { recursive: false });

const packageJsonPath = join(repositoryRoot, "package.json");
const packageLockPath = join(repositoryRoot, "package-lock.json");
const cargoTomlPath = join(repositoryRoot, "src-tauri", "Cargo.toml");
const cargoLockPath = join(repositoryRoot, "src-tauri", "Cargo.lock");
const ffmpegManifestRelativePath = normalizeSafeRelativePath(
  options.ffmpegManifest ?? "third_party/ffmpeg/windows-x64.json",
  "FFmpeg manifest",
);
const ffmpegManifestPath = join(
  repositoryRoot,
  ...ffmpegManifestRelativePath.split("/"),
);
const ffmpegExecutablePath = join(
  repositoryRoot,
  "src-tauri",
  "resources",
  "bin",
  "ffmpeg.exe",
);
const licenseOverrideManifestRelativePath =
  "third_party/licenses/license-text-overrides.json";
const licenseOverrideManifestPath = join(
  repositoryRoot,
  ...licenseOverrideManifestRelativePath.split("/"),
);
const licenseOverrideApprovalRelativePath =
  "third_party/licenses/license-text-override-approvals.json";
const licenseOverrideApprovalPath = join(
  repositoryRoot,
  ...licenseOverrideApprovalRelativePath.split("/"),
);

const packageJson = readJson(packageJsonPath);
const packageLock = readJson(packageLockPath);
const cargoLockIndex = parseCargoLock(readFileSync(cargoLockPath, "utf8"));
const ffmpegManifest = readJson(ffmpegManifestPath);
const licenseOverrideManifest = readJson(licenseOverrideManifestPath);
const licenseOverrideApprovals = readJson(licenseOverrideApprovalPath);
if (releaseProfile === "personal-community-stable") {
  validatePersonalCommunityStableFfmpeg(ffmpegManifest);
  validatePersonalCommunityStableLicenseOverrides(licenseOverrideManifest);
}
const npmRuntimeSpdx = runNpmSbom(["--omit", "dev"]);
const npmBuildSpdx = runNpmSbom([]);
const cargoMetadata = runCargoMetadata(targetTriple, options.offline);

validateNpmSpdx(npmRuntimeSpdx, "runtime");
validateNpmSpdx(npmBuildSpdx, "build");

const workspacePackageIds = new Set(cargoMetadata.workspace_members ?? []);
const cargoPackages = [...cargoMetadata.packages].sort(comparePackages);
const cargoThirdPartyPackages = cargoPackages.filter(
  (component) => !workspacePackageIds.has(component.id),
);
const npmRuntimePackages = npmRuntimeSpdx.packages
  .filter((component) => component.packageFileName)
  .sort(comparePackages);
const npmBuildPackages = npmBuildSpdx.packages
  .filter((component) => component.packageFileName)
  .sort(comparePackages);

assertThirdPartyLicenseDeclarations(npmRuntimePackages, cargoThirdPartyPackages);

const npmLicenseIndexWithoutOverrides = collectNpmLicenseTexts(npmRuntimePackages);
const cargoLicenseIndexWithoutOverrides = collectCargoLicenseTexts(cargoThirdPartyPackages);
const licenseOverrideResult = applyLicenseTextOverrides({
  manifest: licenseOverrideManifest,
  npmEntries: npmLicenseIndexWithoutOverrides.packages,
  npmComponents: npmRuntimePackages,
  cargoEntries: cargoLicenseIndexWithoutOverrides.packages,
  cargoComponents: cargoThirdPartyPackages,
  packageLock,
  cargoLockIndex,
  approvals: licenseOverrideApprovals,
});
const npmLicenseIndex = buildLicenseIndex(npmLicenseIndexWithoutOverrides.packages);
const cargoLicenseIndex = buildLicenseIndex(cargoLicenseIndexWithoutOverrides.packages);

const cargoSpdx = createCargoSpdx(cargoMetadata, targetTriple, generatedAt);
const ffmpegComponent = createFfmpegComponent(ffmpegManifest, ffmpegExecutablePath);
const blockers = collectBlockers({
  packageJson,
  npmLicenseIndex,
  cargoLicenseIndex,
  ffmpegManifest,
  licenseOverrideBlockers: licenseOverrideResult.blockers,
});
const personalCommunityStable = releaseProfile === "personal-community-stable";
const effectiveBlockers = personalCommunityStable
  ? blockers.filter((blocker) => ![
      "NPM_LICENSE_OVERRIDE_REVIEW_PENDING",
      "CARGO_LICENSE_OVERRIDE_REVIEW_PENDING",
      "FFMPEG_REDISTRIBUTION_BLOCKED",
      "FFMPEG_LICENSE_AUDIT_INCOMPLETE",
      "FFMPEG_PATENT_REVIEW_INCOMPLETE",
      "FFMPEG_LEGAL_APPROVAL_MISSING",
    ].includes(blocker.code))
  : blockers;
const advisories = personalCommunityStable
  ? [
      ...blockers.filter((blocker) => !effectiveBlockers.includes(blocker)),
      {
        code: "MPL_SOURCE_FORM_AVAILABLE",
        component: "selectors@0.36.1",
        message: "MPL-2.0 source form is pinned at https://github.com/servo/stylo/tree/635e1a19d02960588a00e189bd4bd5bdb150ec3d/selectors and remains available to recipients.",
      },
    ]
  : [];

writeJson("npm-runtime.spdx.json", npmRuntimeSpdx);
writeJson("npm-build.spdx.json", npmBuildSpdx);
writeJson("cargo-windows-x64.spdx.json", cargoSpdx);
writeJson("ffmpeg-component.json", ffmpegComponent);
writeJson("LICENSE-TEXTS-INDEX.json", {
  schemaVersion: 1,
  generatedAt,
  npm: npmLicenseIndex,
  cargo: cargoLicenseIndex,
  overrides: licenseOverrideResult.report,
});
writeText("THIRD-PARTY-LICENSES.txt", createConsolidatedLicenseText());

const summary = {
  schemaVersion: 1,
  releaseProfile,
  productionApproval: false,
  status: effectiveBlockers.length === 0
    ? personalCommunityStable ? "ready-for-channel" : "ready-for-approval"
    : "generated-with-blockers",
  generatedAt,
  target: targetTriple,
  publicRedistributionReady: !personalCommunityStable && effectiveBlockers.length === 0,
  ...(personalCommunityStable
    ? { channelDistributionReady: effectiveBlockers.length === 0 }
    : {}),
  scope: {
    npmRuntime: "production dependency graph from package-lock.json",
    npmBuild: "complete dependency graph from package-lock.json",
    cargo: `Cargo dependency graph filtered for ${targetTriple}`,
    ffmpeg: "pinned Windows x64 runtime and its recorded build configuration",
  },
  componentCounts: {
    npmRuntime: npmRuntimePackages.length,
    npmBuild: npmBuildPackages.length,
    cargoWindowsX64: cargoThirdPartyPackages.length,
    ffmpeg: 1,
  },
  licenseTextCoverage: {
    npm: npmLicenseIndex.coverage,
    cargo: cargoLicenseIndex.coverage,
  },
  licenseTextOverrides: licenseOverrideResult.summary,
  blockers: effectiveBlockers,
  ...(personalCommunityStable ? { advisories } : {}),
  limitations: [
    "Generated inventory and consolidated license texts are technical evidence, not legal approval.",
    "FFmpeg patent review and final corresponding-source publication remain release-owner decisions.",
    "The build SBOM is intentionally broader than the shipped runtime dependency set.",
  ],
};
writeJson("COMPLIANCE-SUMMARY.json", summary);
writeText(
  "THIRD-PARTY-NOTICES.md",
  createNotices({
    generatedAt,
    targetTriple,
    npmPackages: npmRuntimePackages,
    cargoPackages: cargoThirdPartyPackages,
    ffmpegComponent,
    npmLicenseIndex,
    cargoLicenseIndex,
    blockers: effectiveBlockers,
    advisories,
  }),
);

const evidenceFiles = listFilesRecursively(outputDirectory)
  .filter((path) => path !== "COMPLIANCE-MANIFEST.json")
  .map((path) => {
    const absolutePath = join(outputDirectory, ...path.split("/"));
    return {
      path,
      sizeBytes: statSync(absolutePath).size,
      sha256: sha256File(absolutePath),
    };
  });

writeJson("COMPLIANCE-MANIFEST.json", {
  schemaVersion: 1,
  releaseProfile,
  generatedAt,
  generator: {
    path: "scripts/release/generate-compliance-evidence.mjs",
    sha256: sha256File(fileURLToPath(import.meta.url)),
    node: process.version,
  },
  inputs: [
    ["package.json", packageJsonPath],
    ["package-lock.json", packageLockPath],
    ["src-tauri/Cargo.toml", cargoTomlPath],
    ["src-tauri/Cargo.lock", cargoLockPath],
    [ffmpegManifestRelativePath, ffmpegManifestPath],
    [licenseOverrideManifestRelativePath, licenseOverrideManifestPath],
    [licenseOverrideApprovalRelativePath, licenseOverrideApprovalPath],
    ...licenseOverrideResult.inputFiles.map((entry) => [entry.path, entry.absolutePath]),
  ].map(([path, absolutePath]) => inputReport(path, absolutePath)),
  target: targetTriple,
  fileCount: evidenceFiles.length,
  files: evidenceFiles,
});

const finalReport = {
  ...summary,
  outputDirectory,
  evidenceFileCount: evidenceFiles.length + 1,
  manifestSha256: sha256File(join(outputDirectory, "COMPLIANCE-MANIFEST.json")),
};
process.stdout.write(`${JSON.stringify(finalReport, null, 2)}\n`);

function parseArguments(argumentsList) {
  const parsed = {
    output: null,
    target: null,
    ffmpegManifest: null,
    releaseProfile: null,
    offline: false,
  };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (argument === "--offline") {
      parsed.offline = true;
      continue;
    }
    if (
      argument === "--output" ||
      argument === "--target" ||
      argument === "--ffmpeg-manifest" ||
      argument === "--release-profile"
    ) {
      const value = argumentsList[index + 1];
      if (!value || value.startsWith("--")) {
        throw new Error(`${argument} requires a value.`);
      }
      const key =
        argument === "--ffmpeg-manifest"
          ? "ffmpegManifest"
          : argument === "--release-profile"
            ? "releaseProfile"
            : argument.slice(2);
      parsed[key] = value;
      index += 1;
      continue;
    }
    throw new Error(`Unsupported argument: ${argument}`);
  }
  if (!parsed.output) {
    throw new Error("--output is required.");
  }
  if (parsed.target && !/^[A-Za-z0-9_.-]+$/.test(parsed.target)) {
    throw new Error(`Unsafe target triple: ${parsed.target}`);
  }
  if (parsed.releaseProfile && !["public", "community-beta", "personal-community-stable"].includes(parsed.releaseProfile)) {
    throw new Error(`Unsupported release profile: ${parsed.releaseProfile}`);
  }
  return parsed;
}

function validatePersonalCommunityStableFfmpeg(manifest) {
  const fail = (message) => {
    throw new Error(`Personal community stable FFmpeg manifest is invalid: ${message}`);
  };
  const requireValue = (condition, message) => {
    if (!condition) fail(message);
  };
  const hashPattern = /^[0-9a-f]{64}$/u;
  const commitPattern = /^[0-9a-f]{40}$/u;
  const canonicalTagPattern = /^v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$/u;

  requireValue(manifest?.schemaVersion === 2, "schemaVersion must be 2.");
  requireValue(
    manifest.releaseChannel === "personal-community-stable",
    "releaseChannel must be personal-community-stable.",
  );
  requireValue(
    manifest.productionPromotionAuthorized === false,
    "productionPromotionAuthorized must remain false.",
  );
  requireValue(
    manifest.platform === "windows" && manifest.architecture === "x86_64",
    "target must be Windows x86_64.",
  );

  const releaseTag = manifest.provider?.releaseTag;
  requireValue(canonicalTagPattern.test(releaseTag), "provider.releaseTag must be canonical vX.Y.Z.");
  const releaseBaseUrl = manifest.provider?.releaseUrl;
  requireValue(
    typeof releaseBaseUrl === "string" && releaseBaseUrl.endsWith(`/releases/tag/${releaseTag}`),
    "provider.releaseUrl must identify the same release tag.",
  );

  const artifact = manifest.artifact;
  requireValue(artifact?.fileName === "valoframe-ffmpeg-minimal-windows-x64.zip", "binary sidecar name is not canonical.");
  requireValue(Number.isSafeInteger(artifact?.sizeBytes) && artifact.sizeBytes > 0, "binary sidecar size is invalid.");
  requireValue(hashPattern.test(artifact?.sha256), "binary sidecar SHA-256 is invalid.");
  requireValue(Number.isSafeInteger(artifact?.executableSizeBytes) && artifact.executableSizeBytes > 0, "executable size is invalid.");
  requireValue(hashPattern.test(artifact?.executableSha256), "executable SHA-256 is invalid.");
  requireValue(artifact?.executableMember === "bin/ffmpeg.exe", "binary sidecar must contain bin/ffmpeg.exe.");
  requireValue(artifact?.destination === "src-tauri/resources/bin/ffmpeg.exe", "installer FFmpeg destination is invalid.");
  requireValue(artifact?.url === artifact?.projectMirrorUrl, "binary and project mirror URLs must be identical.");

  requireValue(manifest.ffmpeg?.licenseExpression === "LGPL-3.0-or-later", "license expression must be LGPL-3.0-or-later.");
  requireValue(commitPattern.test(manifest.ffmpeg?.upstreamCommit), "upstream commit must be pinned.");
  requireValue(
    manifest.ffmpeg.upstreamCommit === "ce3c09c101c83add623774d414a9f9498caf5c25",
    "upstream commit does not match the reviewed minimal-build contract.",
  );

  const flags = manifest.build?.configureFlags;
  requireValue(Array.isArray(flags), "build.configureFlags must be an array.");
  const requiredFlags = [
    "--disable-autodetect",
    "--disable-everything",
    "--disable-network",
    "--enable-protocol=file",
    "--enable-demuxer=mov",
    "--enable-parser=h264",
    "--enable-decoder=h264",
    "--enable-parser=hevc",
    "--enable-decoder=hevc",
    "--enable-filter=scale",
    "--enable-encoder=mjpeg",
    "--enable-muxer=image2",
    "--enable-version3",
  ];
  for (const flag of requiredFlags) {
    requireValue(flags.includes(flag), `required configure flag is missing: ${flag}`);
  }
  requireValue(
    !flags.includes("--enable-parser=av1") && !flags.includes("--enable-decoder=av1"),
    "native AV1 must remain disabled until a pinned software decoder is packaged and verified.",
  );
  requireValue(!flags.includes("--enable-gpl"), "--enable-gpl is forbidden for this profile.");
  requireValue(!flags.includes("--enable-nonfree"), "--enable-nonfree is forbidden for this profile.");
  requireValue(
    !flags.some((flag) => /^--enable-lib/u.test(flag)),
    "external --enable-lib* integrations are forbidden for this profile.",
  );
  requireValue(
    Array.isArray(manifest.build.externalLibraries) && manifest.build.externalLibraries.length === 0,
    "externalLibraries must be empty.",
  );

  const source = manifest.sourceCompliance;
  requireValue(source?.redistributionReady === false, "strict redistributionReady must remain false.");
  requireValue(
    source?.status === "personal-community-stable-source-bundled-owner-attested",
    "channel status is invalid.",
  );
  requireValue(source?.ownerAuthorizedForThisChannel === true, "release-owner channel authorization is missing.");
  requireValue(source?.ffmpegExternalLibraryAuditComplete === true, "external-library audit must be complete.");
  requireValue(source?.thirdPartyLicenseAuditComplete === false, "strict third-party legal audit must not be represented as complete.");
  requireValue(source?.ijgAttributionRequired === true && source?.ijgAttributionIncluded === true, "IJG attribution must be required and included.");
  requireValue(source?.patentReviewStatus === "pending-for-strict-public-release", "patent review status must remain honestly pending.");
  requireValue(source?.legalApprovalReference === null, "independent legal approval must not be asserted.");
  requireValue(source?.binaryMirrorUrl === artifact.url, "source compliance must identify the exact binary sidecar.");
  requireValue(source?.upstreamSource?.commit === manifest.ffmpeg.upstreamCommit, "corresponding source commit is inconsistent.");

  const sourceBundle = source?.correspondingSourceBundle;
  requireValue(Number.isSafeInteger(sourceBundle?.sizeBytes) && sourceBundle.sizeBytes > 0, "corresponding-source size is invalid.");
  requireValue(hashPattern.test(sourceBundle?.sha256), "corresponding-source SHA-256 is invalid.");
  for (const [description, urlValue, fileName] of [
    ["binary sidecar", artifact.url, artifact.fileName],
    ["corresponding source", sourceBundle?.url, "ffmpeg-corresponding-source.tar.xz"],
  ]) {
    let parsedUrl;
    try {
      parsedUrl = new URL(urlValue);
    } catch {
      fail(`${description} URL is invalid.`);
    }
    requireValue(parsedUrl.protocol === "https:", `${description} URL must use HTTPS.`);
    requireValue(
      parsedUrl.pathname.endsWith(`/releases/download/${releaseTag}/${fileName}`),
      `${description} must be published beside the installer under the same tag.`,
    );
  }
}

function validatePersonalCommunityStableLicenseOverrides(manifest) {
  const selectors = manifest?.overrides?.find(
    (entry) => entry.ecosystem === "cargo" && entry.name === "selectors" && entry.version === "0.36.1",
  );
  if (
    !selectors ||
    selectors.declaredLicense !== "MPL-2.0" ||
    normalizeRepositoryUrl(selectors.repository) !== "https://github.com/servo/stylo" ||
    selectors.vcsRevision !== "635e1a19d02960588a00e189bd4bd5bdb150ec3d" ||
    selectors.vcsPath !== "selectors" ||
    !selectors.textIds?.includes("mozilla-mpl-2.0") ||
    !selectors.obligations?.includes("mpl-2.0-source-code-form-review-required")
  ) {
    throw new Error(
      "Personal community stable requires the pinned selectors@0.36.1 MPL-2.0 source-form record.",
    );
  }
}

function assertSafeOutputDirectory(directory) {
  if (!isAbsolute(directory)) {
    throw new Error("Output directory must resolve to an absolute path.");
  }
  if (existsSync(directory)) {
    throw new Error(`Output directory must not already exist: ${directory}`);
  }
  const parent = dirname(directory);
  if (!existsSync(parent) || !statSync(parent).isDirectory()) {
    throw new Error(`Output parent directory does not exist: ${parent}`);
  }
  if (lstatSync(parent).isSymbolicLink()) {
    throw new Error(`Output parent directory must not be a symbolic link: ${parent}`);
  }
  const realParent = realpathSync(parent);
  const dangerous = new Set([
    realpathSync(repositoryRoot),
    resolve(repositoryRoot, sep),
    resolve(sep),
  ]);
  if (dangerous.has(resolve(directory)) || resolve(directory) === realParent) {
    throw new Error(`Refusing unsafe output directory: ${directory}`);
  }
}

function runNpmSbom(extraArguments) {
  const npmCli = findNpmCli();
  return runJsonCommand(
    process.execPath,
    [
      npmCli,
      "sbom",
      "--package-lock-only",
      "--sbom-format",
      "spdx",
      ...extraArguments,
    ],
    repositoryRoot,
    false,
  );
}

function findNpmCli() {
  const npmCliCandidates = [
    process.env.npm_execpath,
    join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js"),
  ].filter(Boolean);
  const npmCli = npmCliCandidates.find((candidate) => existsSync(candidate));
  if (!npmCli) {
    throw new Error("Could not locate npm-cli.js.");
  }
  return npmCli;
}

function runCargoMetadata(target, offline) {
  const argumentsList = [
    "metadata",
    "--manifest-path",
    cargoTomlPath,
    "--locked",
    "--format-version",
    "1",
    "--filter-platform",
    target,
  ];
  if (offline) {
    argumentsList.push("--offline");
  }
  return runJsonCommand("cargo", argumentsList, repositoryRoot, false);
}

function runJsonCommand(command, argumentsList, cwd, useShell) {
  let output;
  try {
    output = execFileSync(command, argumentsList, {
      cwd,
      encoding: "utf8",
      maxBuffer: 128 * 1024 * 1024,
      shell: useShell,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
  } catch (error) {
    const stderr = error?.stderr?.toString?.() ?? "";
    throw new Error(`${command} failed.\n${stderr}`.trim(), { cause: error });
  }
  try {
    return JSON.parse(output);
  } catch (error) {
    throw new Error(`${command} did not emit valid JSON.`, { cause: error });
  }
}

function validateNpmSpdx(document, scope) {
  if (document.spdxVersion !== "SPDX-2.3") {
    throw new Error(`npm ${scope} SBOM must use SPDX-2.3.`);
  }
  if (!Array.isArray(document.packages) || document.packages.length < 2) {
    throw new Error(`npm ${scope} SBOM did not contain a dependency graph.`);
  }
  if (!Array.isArray(document.relationships) || document.relationships.length === 0) {
    throw new Error(`npm ${scope} SBOM did not contain relationships.`);
  }
}

function parseCargoLock(text) {
  const packages = new Map();
  let current = null;
  const finish = () => {
    if (!current) {
      return;
    }
    if (current.name && current.version && current.source?.startsWith("registry+")) {
      if (!current.checksum || !/^[0-9a-f]{64}$/.test(current.checksum)) {
        throw new Error(`Cargo.lock registry package has no valid checksum: ${current.name}@${current.version}`);
      }
      const key = cargoLockKey(current.name, current.version, current.source);
      if (packages.has(key)) {
        throw new Error(`Cargo.lock contains a duplicate registry package: ${key}`);
      }
      packages.set(key, current);
    }
    current = null;
  };

  for (const rawLine of text.replaceAll("\r\n", "\n").replaceAll("\r", "\n").split("\n")) {
    const line = rawLine.trim();
    if (line === "[[package]]") {
      finish();
      current = {};
      continue;
    }
    if (!current) {
      continue;
    }
    const match = /^(name|version|source|checksum) = ("(?:\\.|[^"])*")$/.exec(line);
    if (match) {
      current[match[1]] = JSON.parse(match[2]);
    }
  }
  finish();
  return packages;
}

function cargoLockKey(name, version, source) {
  return `${name}\0${version}\0${source}`;
}

function applyLicenseTextOverrides({
  manifest,
  approvals,
  npmEntries,
  npmComponents,
  cargoEntries,
  cargoComponents,
  packageLock: lock,
  cargoLockIndex: lockIndex,
}) {
  assertNoLinkedPathWithinRoot(
    repositoryRoot,
    licenseOverrideManifestPath,
    "license override manifest",
  );
  const manifestItem = lstatSync(licenseOverrideManifestPath);
  if (!manifestItem.isFile() || manifestItem.isSymbolicLink()) {
    throw new Error("License override manifest must be a tracked regular file.");
  }
  assertExactKeys(manifest, ["schemaVersion", "purpose", "policy", "texts", "overrides"], "license override manifest");
  if (manifest.schemaVersion !== 1) {
    throw new Error("Unsupported license override manifest schemaVersion.");
  }
  if (typeof manifest.purpose !== "string" || !manifest.purpose.trim()) {
    throw new Error("License override manifest purpose must be non-empty.");
  }
  assertExactKeys(
    manifest.policy,
    [
      "offlineOnly",
      "exactComponentMatchRequired",
      "unusedOverrideIsError",
      "localPackageLicenseFilesTakePrecedence",
      "reviewDoesNotConstituteLegalApproval",
    ],
    "license override policy",
  );
  for (const [name, value] of Object.entries(manifest.policy)) {
    if (value !== true) {
      throw new Error(`License override policy ${name} must remain true.`);
    }
  }
  if (!Array.isArray(manifest.texts) || manifest.texts.length === 0) {
    throw new Error("License override manifest must declare tracked text files.");
  }
  if (!Array.isArray(manifest.overrides) || manifest.overrides.length === 0) {
    throw new Error("License override manifest must declare component overrides.");
  }

  const textById = new Map();
  const inputFiles = [];
  for (const text of manifest.texts) {
    assertExactKeys(
      text,
      ["id", "path", "sizeBytes", "sha256", "spdxLicenseId", "source", "equivalentSources"],
      "license override text",
    );
    validateLicenseTextSource(text.source, text.id);
    if (!Array.isArray(text.equivalentSources)) {
      throw new Error(`License override equivalentSources must be an array: ${text.id}`);
    }
    const sourceKeys = new Set([
      `${text.source.kind}\0${text.source.repository}\0${text.source.revision}\0${text.source.path}`,
    ]);
    for (const source of text.equivalentSources) {
      validateLicenseTextSource(source, text.id);
      if (source.kind !== "upstream-repository-file" ||
          source.relationship !== "byte-identical-upstream-file") {
        throw new Error(`License override equivalent source must identify a byte-identical upstream file: ${text.id}`);
      }
      const sourceKey = `${source.kind}\0${source.repository}\0${source.revision}\0${source.path}`;
      if (sourceKeys.has(sourceKey)) {
        throw new Error(`License override text contains duplicate upstream sources: ${text.id}`);
      }
      sourceKeys.add(sourceKey);
    }
    if (!/^[a-z0-9][a-z0-9.-]*$/.test(text.id) || textById.has(text.id)) {
      throw new Error(`License override text id is unsafe or duplicated: ${text.id}`);
    }
    const relativePath = normalizeSafeRelativePath(text.path, `license override text ${text.id}`);
    if (!relativePath.startsWith("third_party/licenses/texts/")) {
      throw new Error(`License override text must stay under third_party/licenses/texts: ${relativePath}`);
    }
    if (!Number.isSafeInteger(text.sizeBytes) || text.sizeBytes <= 0) {
      throw new Error(`License override text size is invalid: ${text.id}`);
    }
    if (!/^[0-9a-f]{64}$/.test(text.sha256)) {
      throw new Error(`License override text hash is invalid: ${text.id}`);
    }
    if (text.spdxLicenseId !== null &&
        (typeof text.spdxLicenseId !== "string" ||
         !/^[A-Za-z0-9][A-Za-z0-9.+-]*$/.test(text.spdxLicenseId) ||
         /^(AND|OR|WITH)$/i.test(text.spdxLicenseId))) {
      throw new Error(`License override SPDX license id is invalid: ${text.id}`);
    }
    if (text.source.kind === "license-steward-canonical" && text.spdxLicenseId === null) {
      throw new Error(`Canonical license text must declare its SPDX license id: ${text.id}`);
    }

    const absolutePath = resolve(repositoryRoot, ...relativePath.split("/"));
    assertPathWithinRoot(repositoryRoot, absolutePath, `license override text ${text.id}`);
    if (!existsSync(absolutePath)) {
      throw new Error(`License override text is missing: ${relativePath}`);
    }
    assertNoLinkedPathWithinRoot(
      repositoryRoot,
      absolutePath,
      `license override text ${text.id}`,
    );
    const item = lstatSync(absolutePath);
    if (!item.isFile() || item.isSymbolicLink()) {
      throw new Error(`License override text must be a regular file: ${relativePath}`);
    }
    if (item.size !== text.sizeBytes || sha256File(absolutePath) !== text.sha256) {
      throw new Error(`License override text does not match its approved size and SHA-256: ${relativePath}`);
    }
    const bytes = readFileSync(absolutePath);
    const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    if (decoded.includes("\0")) {
      throw new Error(`License override text contains a NUL character: ${relativePath}`);
    }
    textById.set(text.id, { ...text, path: relativePath, absolutePath });
    inputFiles.push({ path: relativePath, absolutePath });
  }

  assertGitIndexMatchesLicenseInputs([
    licenseOverrideManifestRelativePath,
    licenseOverrideApprovalRelativePath,
    ...inputFiles.map((entry) => entry.path),
  ]);

  const approvalByComponent = validateLicenseOverrideApprovals(approvals);

  const npmEntryByKey = new Map(npmEntries.map((entry) => [componentKey(entry.name, entry.version), entry]));
  const npmComponentByKey = new Map(
    npmComponents.map((entry) => [componentKey(entry.name, entry.versionInfo), entry]),
  );
  const cargoEntryByKey = new Map(cargoEntries.map((entry) => [componentKey(entry.name, entry.version), entry]));
  const cargoComponentByKey = new Map(
    cargoComponents.map((entry) => [componentKey(entry.name, entry.version), entry]),
  );
  const overrideKeys = new Set();
  const usedTextIds = new Set();
  const blockers = [];
  const reportEntries = [];

  for (const override of manifest.overrides) {
    const commonKeys = [
      "ecosystem",
      "name",
      "version",
      "declaredLicense",
      "repository",
      "reason",
      "textIds",
    ];
    const ecosystemKeys = override.ecosystem === "npm"
      ? ["packagePath", "resolved", "lockIntegrity", "registryTarballSha1", "registryGitHead"]
      : override.ecosystem === "cargo"
        ? ["registrySource", "registryChecksum", "vcsRevision", "vcsPath", "obligations"]
        : [];
    if (ecosystemKeys.length === 0) {
      throw new Error(`Unsupported license override ecosystem: ${override.ecosystem}`);
    }
    assertExactKeys(override, [...commonKeys, ...ecosystemKeys], `license override ${override.name}`);
    if (!override.name || !override.version || !override.declaredLicense ||
        !String(override.reason).trim() ||
        !String(override.repository).startsWith("https://")) {
      throw new Error(`License override component provenance is incomplete: ${override.name}@${override.version}`);
    }
    if (override.ecosystem === "npm" && !/^[0-9a-f]{40}$/.test(override.registryGitHead)) {
      throw new Error(`npm license override registry gitHead is invalid: ${override.name}@${override.version}`);
    }
    if (override.ecosystem === "cargo" && !/^[0-9a-f]{40}$/.test(override.vcsRevision)) {
      throw new Error(`Cargo license override VCS revision is invalid: ${override.name}@${override.version}`);
    }
    if (override.ecosystem === "cargo" && override.vcsPath !== null) {
      const normalizedVcsPath = normalizeSafeRelativePath(
        override.vcsPath,
        `license override vcsPath for ${override.name}@${override.version}`,
      );
      if (normalizedVcsPath !== override.vcsPath) {
        throw new Error(`License override vcsPath must use canonical forward slashes: ${override.name}@${override.version}`);
      }
    }
    if (!Array.isArray(override.textIds) || override.textIds.length === 0 ||
        new Set(override.textIds).size !== override.textIds.length) {
      throw new Error(`License override textIds are missing or duplicated: ${override.name}@${override.version}`);
    }
    const key = `${override.ecosystem}:${componentKey(override.name, override.version)}`;
    if (overrideKeys.has(key)) {
      throw new Error(`Duplicate license override component: ${key}`);
    }
    overrideKeys.add(key);
    const entry = override.ecosystem === "npm"
      ? npmEntryByKey.get(componentKey(override.name, override.version))
      : cargoEntryByKey.get(componentKey(override.name, override.version));
    const component = override.ecosystem === "npm"
      ? npmComponentByKey.get(componentKey(override.name, override.version))
      : cargoComponentByKey.get(componentKey(override.name, override.version));
    if (!entry || !component) {
      throw new Error(`License override does not match the locked runtime graph: ${key}`);
    }
    if (entry.files.length !== 0) {
      throw new Error(`License override is stale because the package now includes local text: ${key}`);
    }
    if (entry.licenseDeclared !== override.declaredLicense) {
      throw new Error(`License override SPDX expression does not match the package declaration: ${key}`);
    }

    if (override.ecosystem === "npm") {
      validateNpmLicenseOverride(override, component, lock);
    } else {
      validateCargoLicenseOverride(override, component, lockIndex);
    }

    const referencedTexts = override.textIds.map((textId) => {
      const text = textById.get(textId);
      if (!text) {
        throw new Error(`License override references an unknown text id: ${textId}`);
      }
      usedTextIds.add(textId);
      validateTextProvenanceForOverride(override, text);
      return text;
    });
    assertSpdxTextCoverage(override, referencedTexts);

    const componentId = `${override.ecosystem}:${override.name}@${override.version}`;
    const textSha256 = [...new Set(referencedTexts.map((text) => text.sha256))].sort();
    const approval = approvalByComponent.get(componentId) ?? null;
    if (approval &&
        (approval.declaredLicense !== override.declaredLicense ||
         JSON.stringify(approval.textSha256) !== JSON.stringify(textSha256))) {
      throw new Error(`License override approval does not match the component license texts: ${componentId}`);
    }
    approvalByComponent.delete(componentId);
    const review = approval
      ? {
          status: "approved",
          reviewer: approval.reviewer,
          reviewedAtUtc: approval.reviewedAtUtc,
          approvalReference: approval.approvalReference,
        }
      : {
          status: "pending",
          reviewer: null,
          reviewedAtUtc: null,
          approvalReference: null,
        };

    const files = referencedTexts.map((text) => {
      const collected = collectLicenseFiles(
        repositoryRoot,
        [text.absolutePath],
        `${override.name}@${override.version}`,
      )[0];
      return {
        ...collected,
        sourceKind: "tracked-license-override",
        overrideTextId: text.id,
        provenance: text.source,
        equivalentProvenance: text.equivalentSources,
      };
    });
    entry.files.push(...files);
    entry.licenseTextSource = "tracked-license-override";
    entry.override = {
      reason: override.reason,
      repository: override.repository,
      registryGitHead: override.registryGitHead ?? null,
      vcsRevision: override.vcsRevision ?? null,
      vcsPath: override.vcsPath ?? null,
      review,
      obligations: override.obligations ?? [],
    };
    if (review.status === "pending") {
      blockers.push({
        code: `${override.ecosystem.toUpperCase()}_LICENSE_OVERRIDE_REVIEW_PENDING`,
        component: `${override.name}@${override.version}`,
        message: "Tracked upstream license text is present, but the component-specific override still requires owner/legal review.",
      });
    }
    reportEntries.push({
      ecosystem: override.ecosystem,
      component: `${override.name}@${override.version}`,
      declaredLicense: override.declaredLicense,
      textIds: override.textIds,
      textSha256,
      review,
    });
  }

  if (approvalByComponent.size > 0) {
    throw new Error(
      `License override approval manifest contains unknown or stale components: ${[
        ...approvalByComponent.keys(),
      ].join(", ")}`,
    );
  }

  const unusedTextIds = [...textById.keys()].filter((id) => !usedTextIds.has(id));
  if (unusedTextIds.length > 0) {
    throw new Error(`License override manifest contains unused tracked texts: ${unusedTextIds.join(", ")}`);
  }
  const pendingReviewCount = reportEntries.filter((entry) => entry.review.status === "pending").length;
  return {
    blockers,
    inputFiles: inputFiles.sort((left, right) => left.path.localeCompare(right.path)),
    summary: {
      manifest: licenseOverrideManifestRelativePath,
      approvalManifest: licenseOverrideApprovalRelativePath,
      overrideCount: reportEntries.length,
      pendingReviewCount,
      approvedReviewCount: reportEntries.length - pendingReviewCount,
      legalApprovalGranted: false,
    },
    report: {
      schemaVersion: 1,
      manifest: licenseOverrideManifestRelativePath,
      manifestSha256: sha256File(licenseOverrideManifestPath),
      approvalManifest: licenseOverrideApprovalRelativePath,
      approvalManifestSha256: sha256File(licenseOverrideApprovalPath),
      entries: reportEntries,
    },
  };
}

function validateNpmLicenseOverride(override, component, lock) {
  const packagePath = normalizeSafeRelativePath(override.packagePath, "npm license override packagePath");
  if (component.packageFileName !== packagePath || !packagePath.startsWith("node_modules/")) {
    throw new Error(`npm license override package path does not match the SBOM: ${override.name}@${override.version}`);
  }
  const lockEntry = lock.packages?.[packagePath];
  if (!lockEntry || lockEntry.version !== override.version ||
      lockEntry.license !== override.declaredLicense ||
      lockEntry.resolved !== override.resolved ||
      lockEntry.integrity !== override.lockIntegrity) {
    throw new Error(`npm license override does not match package-lock.json: ${override.name}@${override.version}`);
  }
  if (!override.resolved.startsWith("https://registry.npmjs.org/") ||
      !override.lockIntegrity.startsWith("sha512-") ||
      !/^[0-9a-f]{40}$/.test(override.registryTarballSha1)) {
    throw new Error(`npm license override registry provenance is invalid: ${override.name}@${override.version}`);
  }
  validateCachedNpmTarball(override);
  const packageDirectory = resolve(repositoryRoot, ...packagePath.split("/"));
  assertPathWithinRoot(repositoryRoot, packageDirectory, "npm license override package directory");
  const installedManifest = readJson(join(packageDirectory, "package.json"));
  const repositoryValue = typeof installedManifest.repository === "string"
    ? installedManifest.repository
    : installedManifest.repository?.url;
  if (installedManifest.name !== override.name ||
      installedManifest.version !== override.version ||
      installedManifest.license !== override.declaredLicense ||
      normalizeRepositoryUrl(repositoryValue) !== normalizeRepositoryUrl(override.repository)) {
    throw new Error(`npm license override does not match the installed package metadata: ${override.name}@${override.version}`);
  }
}

function validateCachedNpmTarball(override) {
  const integrityMatch = /^sha512-([A-Za-z0-9+/]+={0,2})$/.exec(override.lockIntegrity);
  if (!integrityMatch) {
    throw new Error(`npm license override uses an invalid SHA-512 integrity value: ${override.name}@${override.version}`);
  }
  const digest = Buffer.from(integrityMatch[1], "base64");
  if (digest.length !== 64 || `sha512-${digest.toString("base64")}` !== override.lockIntegrity) {
    throw new Error(`npm license override uses a non-canonical SHA-512 integrity value: ${override.name}@${override.version}`);
  }
  const cacheRoot = execFileSync(
    process.execPath,
    [findNpmCli(), "config", "get", "cache"],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, npm_config_offline: "true", npm_config_loglevel: "silent" },
    },
  ).trim();
  if (!cacheRoot || !isAbsolute(cacheRoot)) {
    throw new Error("npm cache path must resolve to an absolute directory.");
  }
  const digestHex = digest.toString("hex");
  const tarballPath = join(
    cacheRoot,
    "_cacache",
    "content-v2",
    "sha512",
    digestHex.slice(0, 2),
    digestHex.slice(2, 4),
    digestHex.slice(4),
  );
  if (!existsSync(tarballPath)) {
    throw new Error(
      `Locked npm tarball is missing from the local cache; run npm ci before generating evidence: ${override.name}@${override.version}`,
    );
  }
  const item = lstatSync(tarballPath);
  if (!item.isFile() || item.isSymbolicLink()) {
    throw new Error(`Locked npm tarball cache entry must be a regular file: ${override.name}@${override.version}`);
  }
  const bytes = readFileSync(tarballPath);
  const actualSha512 = createHash("sha512").update(bytes).digest("base64");
  const actualSha1 = createHash("sha1").update(bytes).digest("hex");
  if (`sha512-${actualSha512}` !== override.lockIntegrity ||
      actualSha1 !== override.registryTarballSha1) {
    throw new Error(`Locked npm tarball does not match its SHA-512/SHA-1 provenance: ${override.name}@${override.version}`);
  }
}

function validateCargoLicenseOverride(override, component, lockIndex) {
  if (!override.registrySource.startsWith("registry+") ||
      component.source !== override.registrySource ||
      normalizeCargoLicense(component.license, component.license_file) !== override.declaredLicense ||
      normalizeRepositoryUrl(component.repository) !== normalizeRepositoryUrl(override.repository)) {
    throw new Error(`Cargo license override does not match package metadata: ${override.name}@${override.version}`);
  }
  const lockEntry = lockIndex.get(cargoLockKey(override.name, override.version, override.registrySource));
  if (!lockEntry || lockEntry.checksum !== override.registryChecksum ||
      !/^[0-9a-f]{64}$/.test(override.registryChecksum)) {
    throw new Error(`Cargo license override does not match Cargo.lock: ${override.name}@${override.version}`);
  }
  const packageDirectory = dirname(component.manifest_path);
  const vcsInfoPath = join(packageDirectory, ".cargo_vcs_info.json");
  if (!existsSync(vcsInfoPath)) {
    throw new Error(`Cargo license override package has no .cargo_vcs_info.json: ${override.name}@${override.version}`);
  }
  const vcsInfo = readJson(vcsInfoPath);
  const actualPath = vcsInfo.path_in_vcs ?? null;
  if (vcsInfo.git?.sha1 !== override.vcsRevision || actualPath !== override.vcsPath) {
    throw new Error(`Cargo license override VCS provenance does not match the crate archive: ${override.name}@${override.version}`);
  }
}

function validateLicenseTextSource(source, textId) {
  assertExactKeys(
    source,
    ["kind", "repository", "revision", "path", "url", "relationship"],
    `license override source ${textId}`,
  );
  if (typeof source.url !== "string" || !source.url.startsWith("https://") ||
      typeof source.relationship !== "string" || !source.relationship.trim() ||
      typeof source.revision !== "string" || !source.revision.trim()) {
    throw new Error(`License override source provenance is incomplete: ${textId}`);
  }
  if (source.kind === "upstream-repository-file") {
    if (typeof source.repository !== "string" || !source.repository.startsWith("https://") ||
        !/^[0-9a-f]{40}$/.test(source.revision) ||
        typeof source.path !== "string" || !source.path.trim()) {
      throw new Error(`Upstream repository license provenance is incomplete: ${textId}`);
    }
  } else if (source.kind === "license-steward-canonical") {
    if (source.repository !== null || source.path !== null) {
      throw new Error(`Canonical license-steward source must not claim a repository path: ${textId}`);
    }
  } else {
    throw new Error(`Unsupported license override source kind: ${source.kind}`);
  }
}

function validateTextProvenanceForOverride(override, text) {
  const sources = [text.source, ...text.equivalentSources];
  if (sources.some((source) => source.kind === "license-steward-canonical")) {
    return;
  }
  const exactSource = sources.some(
    (source) =>
      normalizeRepositoryUrl(source.repository) === normalizeRepositoryUrl(override.repository) &&
      source.revision === override.vcsRevision,
  );
  if (exactSource) {
    return;
  }
  const explicitPostReleaseClarification =
    override.ecosystem === "npm" &&
    sources.some(
      (source) =>
        source.relationship === "post-release-upstream-license-clarification" &&
        normalizeRepositoryUrl(source.repository) === normalizeRepositoryUrl(override.repository),
    );
  if (!explicitPostReleaseClarification) {
    throw new Error(
      `License override text provenance does not match the component VCS revision: ${override.ecosystem}:${override.name}@${override.version}`,
    );
  }
}

function assertSpdxTextCoverage(override, texts) {
  const expression = String(override.declaredLicense);
  if (!/^[A-Za-z0-9.+()\-\s]+$/.test(expression) || /\bWITH\b/.test(expression)) {
    throw new Error(`Unsupported SPDX expression in license override: ${expression}`);
  }
  const declaredIds = new Set(
    (expression.match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g) ?? []).filter(
      (token) => token !== "AND" && token !== "OR",
    ),
  );
  const coveredIds = new Set(
    texts.map((text) => text.spdxLicenseId).filter((value) => value !== null),
  );
  const missing = [...declaredIds].filter((id) => !coveredIds.has(id));
  const unrelated = [...coveredIds].filter((id) => !declaredIds.has(id));
  if (declaredIds.size === 0 || missing.length > 0 || unrelated.length > 0) {
    throw new Error(
      `License override text SPDX coverage mismatch for ${override.ecosystem}:${override.name}@${override.version}; missing=${missing.join(",") || "none"}, unrelated=${unrelated.join(",") || "none"}.`,
    );
  }
}

function validateLicenseOverrideApprovals(approvals) {
  assertNoLinkedPathWithinRoot(
    repositoryRoot,
    licenseOverrideApprovalPath,
    "license override approval manifest",
  );
  assertExactKeys(approvals, ["schemaVersion", "purpose", "approvals"], "license override approval manifest");
  if (approvals.schemaVersion !== 1 ||
      typeof approvals.purpose !== "string" || !approvals.purpose.trim() ||
      !Array.isArray(approvals.approvals)) {
    throw new Error("License override approval manifest is invalid.");
  }
  const byComponent = new Map();
  for (const approval of approvals.approvals) {
    assertExactKeys(
      approval,
      [
        "component",
        "declaredLicense",
        "textSha256",
        "decision",
        "reviewer",
        "reviewedAtUtc",
        "approvalReference",
      ],
      "license override approval",
    );
    const parsedReviewedAt = typeof approval.reviewedAtUtc === "string"
      ? new Date(approval.reviewedAtUtc)
      : null;
    const normalizedReviewedAt = typeof approval.reviewedAtUtc === "string" &&
      !approval.reviewedAtUtc.includes(".")
      ? approval.reviewedAtUtc.replace(/Z$/, ".000Z")
      : approval.reviewedAtUtc;
    if (typeof approval.component !== "string" ||
        !/^(npm|cargo):[^@\0]+@[^@\0]+$/.test(approval.component) ||
        byComponent.has(approval.component) ||
        typeof approval.declaredLicense !== "string" || !approval.declaredLicense.trim() ||
        approval.decision !== "approved" ||
        typeof approval.reviewer !== "string" || !approval.reviewer.trim() ||
        typeof approval.reviewedAtUtc !== "string" ||
        !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(approval.reviewedAtUtc) ||
        !parsedReviewedAt || Number.isNaN(parsedReviewedAt.valueOf()) ||
        parsedReviewedAt.toISOString() !== normalizedReviewedAt ||
        typeof approval.approvalReference !== "string" || !approval.approvalReference.trim() ||
        !Array.isArray(approval.textSha256) || approval.textSha256.length === 0 ||
        approval.textSha256.some((hash) => typeof hash !== "string" || !/^[0-9a-f]{64}$/.test(hash)) ||
        new Set(approval.textSha256).size !== approval.textSha256.length ||
        JSON.stringify([...approval.textSha256].sort()) !== JSON.stringify(approval.textSha256)) {
      throw new Error(`License override approval is invalid: ${approval.component ?? "unknown"}`);
    }
    byComponent.set(approval.component, approval);
  }
  return byComponent;
}

function assertGitIndexMatchesLicenseInputs(paths) {
  const uniquePaths = [...new Set(paths)].sort();
  for (const path of uniquePaths) {
    try {
      execFileSync(
        "git",
        ["-C", repositoryRoot, "ls-files", "--error-unmatch", "--", path],
        { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
      );
      execFileSync(
        "git",
        ["-C", repositoryRoot, "diff", "--quiet", "--", path],
        { stdio: ["ignore", "ignore", "pipe"] },
      );
    } catch {
      throw new Error(
        `License override input must be tracked in the Git index and match its indexed bytes: ${path}`,
      );
    }
  }
}

function componentKey(name, version) {
  return `${name}\0${version}`;
}

function normalizeRepositoryUrl(value) {
  return String(value ?? "")
    .replace(/^git\+/, "")
    .replace(/\.git\/?$/, "")
    .replace(/\/$/, "")
    .toLowerCase();
}

function assertExactKeys(value, allowedKeys, description) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${description} must be an object.`);
  }
  const allowed = new Set(allowedKeys);
  const unexpected = Object.keys(value).filter((key) => !allowed.has(key));
  if (unexpected.length > 0) {
    throw new Error(`${description} contains unsupported fields: ${unexpected.join(", ")}`);
  }
  const missing = allowedKeys.filter(
    (key) => !(key in value) && !(key === "obligations" && description.startsWith("license override ")),
  );
  if (missing.length > 0) {
    throw new Error(`${description} is missing required fields: ${missing.join(", ")}`);
  }
}

function assertThirdPartyLicenseDeclarations(npmPackages, cargoPackagesToCheck) {
  const npmMissing = npmPackages.filter(
    (component) =>
      !component.licenseDeclared || component.licenseDeclared === "NOASSERTION",
  );
  const cargoMissing = cargoPackagesToCheck.filter(
    (component) => !component.license && !component.license_file,
  );
  if (npmMissing.length > 0 || cargoMissing.length > 0) {
    const npmNames = npmMissing.map((component) => `${component.name}@${component.versionInfo}`);
    const cargoNames = cargoMissing.map((component) => `${component.name}@${component.version}`);
    throw new Error(
      `Third-party packages without a license declaration: ${[
        ...npmNames,
        ...cargoNames,
      ].join(", ")}`,
    );
  }
}

function collectNpmLicenseTexts(packages) {
  const entries = packages.map((component) => {
    const packageRelativePath = normalizeSafeRelativePath(
      component.packageFileName,
      `npm package path for ${component.name}`,
    );
    if (!packageRelativePath.startsWith("node_modules/")) {
      throw new Error(`npm package path is outside node_modules: ${packageRelativePath}`);
    }
    const packageDirectory = resolve(repositoryRoot, ...packageRelativePath.split("/"));
    assertPathWithinRoot(repositoryRoot, packageDirectory, "npm package directory");
    if (!existsSync(packageDirectory) || !statSync(packageDirectory).isDirectory()) {
      throw new Error(
        `npm package files are missing; run npm ci before generating evidence: ${packageRelativePath}`,
      );
    }
    const licenseFiles = findLicenseFiles(packageDirectory);
    const collected = collectLicenseFiles(
      packageDirectory,
      licenseFiles,
      `${component.name}@${component.versionInfo}`,
    );
    return {
      name: component.name,
      version: component.versionInfo,
      licenseDeclared: component.licenseDeclared,
      packagePath: packageRelativePath,
      files: collected,
    };
  });
  return buildLicenseIndex(entries);
}

function collectCargoLicenseTexts(packages) {
  const entries = packages.map((component) => {
    const packageDirectory = dirname(component.manifest_path);
    if (!existsSync(packageDirectory) || !statSync(packageDirectory).isDirectory()) {
      throw new Error(`Cargo package directory is missing: ${component.name}@${component.version}`);
    }
    const licenseFiles = findLicenseFiles(packageDirectory);
    if (component.license_file) {
      const declaredLicenseFile = resolve(packageDirectory, component.license_file);
      assertPathWithinRoot(packageDirectory, declaredLicenseFile, "Cargo license_file");
      if (existsSync(declaredLicenseFile) && statSync(declaredLicenseFile).isFile()) {
        licenseFiles.push(declaredLicenseFile);
      }
    }
    const uniqueLicenseFiles = [...new Set(licenseFiles)].sort();
    const collected = collectLicenseFiles(
      packageDirectory,
      uniqueLicenseFiles,
      `${component.name}@${component.version}`,
    );
    return {
      name: component.name,
      version: component.version,
      licenseDeclared: normalizeCargoLicense(component.license, component.license_file),
      source: cargoDownloadLocation(component),
      files: collected,
    };
  });
  return buildLicenseIndex(entries);
}

function findLicenseFiles(packageDirectory) {
  const candidates = [];
  for (const entry of readdirSync(packageDirectory, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) {
      continue;
    }
    if (entry.isFile() && isLicenseFileName(entry.name)) {
      candidates.push(join(packageDirectory, entry.name));
      continue;
    }
    if (entry.isDirectory() && /^(licenses?|notices?)$/i.test(entry.name)) {
      const nestedRoot = join(packageDirectory, entry.name);
      for (const nested of readdirSync(nestedRoot, { withFileTypes: true })) {
        if (!nested.isSymbolicLink() && nested.isFile()) {
          candidates.push(join(nestedRoot, nested.name));
        }
      }
    }
  }
  return candidates.sort();
}

function isLicenseFileName(name) {
  return /^(licen[cs]e|copying|notice|copyright|unlicense)(?:[._-].*)?$/i.test(name);
}

function collectLicenseFiles(packageDirectory, files, componentName) {
  return files.map((sourcePath) => {
    assertPathWithinRoot(packageDirectory, sourcePath, "package license file");
    const item = lstatSync(sourcePath);
    if (!item.isFile() || item.isSymbolicLink()) {
      throw new Error(`License source must be a regular file: ${sourcePath}`);
    }
    const bytes = readFileSync(sourcePath);
    const hash = createHash("sha256").update(bytes).digest("hex");
    const textId = `LicenseText-${hash.slice(0, 16)}`;
    let text = licenseTextCatalog.get(hash)?.text;
    if (!text) {
      text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      if (text.includes("\0")) {
        throw new Error(`License source contains a NUL character: ${sourcePath}`);
      }
      licenseTextCatalog.set(hash, {
        id: textId,
        sha256: hash,
        sizeBytes: bytes.length,
        text,
        components: new Set(),
      });
    }
    licenseTextCatalog.get(hash).components.add(componentName);
    return {
      sourceFileName: relative(packageDirectory, sourcePath).replaceAll("\\", "/"),
      textId,
      sizeBytes: bytes.length,
      sha256: hash,
    };
  });
}

function createConsolidatedLicenseText() {
  const entries = [...licenseTextCatalog.values()].sort((left, right) =>
    left.id.localeCompare(right.id),
  );
  const lines = [
    "VALOFRAME THIRD-PARTY LICENSE TEXTS",
    "",
    `Generated: ${generatedAt}`,
    "",
    "Each unique license file is included once. Package-to-text mappings are recorded in LICENSE-TEXTS-INDEX.json.",
    "",
  ];
  for (const entry of entries) {
    lines.push("=".repeat(80));
    lines.push(entry.id);
    lines.push(`SHA-256: ${entry.sha256}`);
    lines.push(`Referenced by: ${[...entry.components].sort().join(", ")}`);
    lines.push("=".repeat(80));
    lines.push("");
    lines.push(entry.text.replaceAll("\r\n", "\n").replaceAll("\r", "\n").trimEnd());
    lines.push("");
  }
  return `${lines.join("\n")}\n`;
}

function buildLicenseIndex(entries) {
  const withoutLocalText = entries
    .filter((entry) => entry.files.length === 0)
    .map((entry) => `${entry.name}@${entry.version}`);
  return {
    coverage: {
      packageCount: entries.length,
      packagesWithLocalText: entries.length - withoutLocalText.length,
      packagesWithoutLocalText: withoutLocalText.length,
    },
    packagesWithoutLocalText: withoutLocalText,
    packages: entries,
  };
}

function createCargoSpdx(metadata, target, createdAt) {
  const packages = [...metadata.packages].sort(comparePackages);
  const idMap = new Map(
    packages.map((component) => [component.id, cargoSpdxId(component)]),
  );
  const lockHash = sha256File(cargoLockPath);
  const spdxPackages = packages.map((component) => ({
    name: component.name,
    SPDXID: idMap.get(component.id),
    versionInfo: component.version,
    downloadLocation: cargoDownloadLocation(component),
    filesAnalyzed: false,
    licenseConcluded: "NOASSERTION",
    licenseDeclared: normalizeCargoLicense(component.license, component.license_file),
    copyrightText: "NOASSERTION",
    homepage: component.homepage || component.repository || "NOASSERTION",
    externalRefs: [
      {
        referenceCategory: "PACKAGE-MANAGER",
        referenceType: "purl",
        referenceLocator: `pkg:cargo/${encodeURIComponent(component.name)}@${encodeURIComponent(
          component.version,
        )}`,
      },
    ],
  }));
  const relationships = [];
  if (metadata.resolve?.root && idMap.has(metadata.resolve.root)) {
    relationships.push({
      spdxElementId: "SPDXRef-DOCUMENT",
      relationshipType: "DESCRIBES",
      relatedSpdxElement: idMap.get(metadata.resolve.root),
    });
  }
  for (const node of metadata.resolve?.nodes ?? []) {
    const sourceId = idMap.get(node.id);
    for (const dependency of node.deps ?? []) {
      const destinationId = idMap.get(dependency.pkg);
      if (sourceId && destinationId) {
        relationships.push({
          spdxElementId: sourceId,
          relationshipType: "DEPENDS_ON",
          relatedSpdxElement: destinationId,
        });
      }
    }
  }
  relationships.sort((left, right) =>
    `${left.spdxElementId}:${left.relatedSpdxElement}`.localeCompare(
      `${right.spdxElementId}:${right.relatedSpdxElement}`,
    ),
  );
  return {
    spdxVersion: "SPDX-2.3",
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: `valorant-highlight-manager-cargo-${target}`,
    documentNamespace: `https://github.com/2424521842/valoframe/sbom/cargo/${target}/${lockHash}`,
    creationInfo: {
      created: createdAt,
      creators: ["Tool: VALOFRAME compliance evidence generator"],
    },
    packages: spdxPackages,
    relationships,
  };
}

function createFfmpegComponent(manifest, executablePath) {
  const manifestHash = sha256File(ffmpegManifestPath);
  const component = {
    schemaVersion: 1,
    generatedAt,
    platform: manifest.platform,
    architecture: manifest.architecture,
    provider: manifest.provider,
    artifact: manifest.artifact,
    ffmpeg: manifest.ffmpeg,
    sourceCompliance: manifest.sourceCompliance,
    manifestSha256: manifestHash,
    runtimeVerification: {
      performed: false,
      executableSha256: null,
      versionOutput: null,
      buildConfiguration: null,
      enableFlags: [],
    },
  };
  if (!existsSync(executablePath)) {
    component.runtimeVerification.reason = "Pinned executable is not prepared.";
    return component;
  }
  const actualHash = sha256File(executablePath);
  if (actualHash !== String(manifest.artifact.executableSha256).toLowerCase()) {
    throw new Error("Prepared FFmpeg executable does not match the provenance manifest.");
  }
  component.runtimeVerification.executableSha256 = actualHash;
  if (process.platform !== "win32") {
    component.runtimeVerification.reason = "Windows executable runtime check requires Windows.";
    return component;
  }
  const versionOutput = runTextCommand(executablePath, ["-hide_banner", "-version"]);
  const buildConfiguration = runTextCommand(executablePath, ["-hide_banner", "-buildconf"]);
  if (!versionOutput.startsWith(manifest.ffmpeg.versionPrefix)) {
    throw new Error("FFmpeg runtime version does not match the provenance manifest.");
  }
  component.runtimeVerification = {
    performed: true,
    executableSha256: actualHash,
    versionOutput,
    buildConfiguration,
    enableFlags: [...buildConfiguration.matchAll(/--enable-[^\s]+/g)].map(
      (match) => match[0],
    ),
  };
  return component;
}

function runTextCommand(command, argumentsList) {
  try {
    return execFileSync(command, argumentsList, {
      cwd: repositoryRoot,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    }).trim();
  } catch (error) {
    const stdout = error?.stdout?.toString?.() ?? "";
    const stderr = error?.stderr?.toString?.() ?? "";
    const combined = `${stdout}\n${stderr}`.trim();
    if (error?.status === 0 && combined) {
      return combined;
    }
    throw new Error(`Could not inspect ${command}.\n${combined}`.trim(), { cause: error });
  }
}

function collectBlockers({
  packageJson: project,
  npmLicenseIndex,
  cargoLicenseIndex,
  ffmpegManifest: manifest,
  licenseOverrideBlockers,
}) {
  const blockers = [];
  const projectLicensePath = findProjectLicensePath();
  if (!project.license || !projectLicensePath) {
    blockers.push({
      code: "PROJECT_LICENSE_UNDECLARED",
      message: "The project has no approved package license and tracked license file.",
    });
  }
  for (const packageName of npmLicenseIndex.packagesWithoutLocalText) {
    blockers.push({
      code: "NPM_LICENSE_TEXT_MISSING",
      component: packageName,
      message: "The installed npm package did not contain a local license text; an audited override is required.",
    });
  }
  for (const packageName of cargoLicenseIndex.packagesWithoutLocalText) {
    blockers.push({
      code: "CARGO_LICENSE_TEXT_MISSING",
      component: packageName,
      message: "The Cargo package did not contain a local license text; an audited clarification is required.",
    });
  }
  blockers.push(...licenseOverrideBlockers);
  const source = manifest.sourceCompliance;
  if (source.redistributionReady !== true || source.status !== "ready-for-redistribution") {
    blockers.push({
      code: "FFMPEG_REDISTRIBUTION_BLOCKED",
      message: `FFmpeg source compliance status is '${source.status}'.`,
    });
  }
  if (source.thirdPartyLicenseAuditComplete !== true) {
    blockers.push({
      code: "FFMPEG_LICENSE_AUDIT_INCOMPLETE",
      message: "FFmpeg third-party license audit is incomplete.",
    });
  }
  if (source.patentReviewStatus !== "approved" && source.patentReviewStatus !== "not-required") {
    blockers.push({
      code: "FFMPEG_PATENT_REVIEW_INCOMPLETE",
      message: `FFmpeg patent review status is '${source.patentReviewStatus}'.`,
    });
  }
  if (!source.legalApprovalReference) {
    blockers.push({
      code: "FFMPEG_LEGAL_APPROVAL_MISSING",
      message: "FFmpeg legal approval reference is missing.",
    });
  }
  return blockers;
}

function createNotices({
  generatedAt: createdAt,
  targetTriple: target,
  npmPackages,
  cargoPackages: rustPackages,
  ffmpegComponent: ffmpeg,
  npmLicenseIndex,
  cargoLicenseIndex,
  blockers,
  advisories = [],
}) {
  const lines = [
    "# VALOFRAME third-party component notice",
    "",
    `Generated: ${createdAt}`,
    "",
    "This document is a machine-generated inventory for release review. It does not replace the license texts under `license-texts/` and does not constitute legal approval.",
    "",
    "## npm runtime components",
    "",
    "| Component | Version | Declared license |",
    "| --- | --- | --- |",
    ...npmPackages.map(
      (component) =>
        `| ${escapeMarkdown(component.name)} | ${escapeMarkdown(
          component.versionInfo,
        )} | ${escapeMarkdown(component.licenseDeclared)} |`,
    ),
    "",
    `Local license text coverage: ${npmLicenseIndex.coverage.packagesWithLocalText}/${npmLicenseIndex.coverage.packageCount} packages.`,
    "",
    `## Cargo components for ${target}`,
    "",
    "| Component | Version | Declared license |",
    "| --- | --- | --- |",
    ...rustPackages.map(
      (component) =>
        `| ${escapeMarkdown(component.name)} | ${escapeMarkdown(
          component.version,
        )} | ${escapeMarkdown(normalizeCargoLicense(component.license, component.license_file))} |`,
    ),
    "",
    `Local license text coverage: ${cargoLicenseIndex.coverage.packagesWithLocalText}/${cargoLicenseIndex.coverage.packageCount} packages.`,
    "",
    "## FFmpeg",
    "",
    `- Version: ${ffmpeg.ffmpeg.version}`,
    `- Provider: ${ffmpeg.provider.name}`,
    `- License expression: ${ffmpeg.ffmpeg.licenseExpression}`,
    `- Redistribution status: ${ffmpeg.sourceCompliance.status}`,
    `- Provenance manifest SHA-256: ${ffmpeg.manifestSha256}`,
    "",
    "The application invokes FFmpeg as a separate command-line program. See `../ffmpeg/` for its bundled license and source-status files.",
    "",
    "## Open release blockers",
    "",
    ...blockers.map(
      (blocker) =>
        `- ${blocker.code}${blocker.component ? ` (${escapeMarkdown(blocker.component)})` : ""}: ${blocker.message}`,
    ),
    "",
    ...(advisories.length === 0
      ? []
      : [
          "## Personal community release advisories",
          "",
          ...advisories.map(
          (advisory) =>
            `- ${advisory.code}${advisory.component ? ` (${escapeMarkdown(advisory.component)})` : ""}: ${advisory.message}`,
          ),
          "",
        ]),
  ];
  return `${lines.join("\n")}\n`;
}

function normalizeCargoLicense(license, licenseFile) {
  if (license) {
    return license.replace(/([A-Za-z0-9.+-]+)\s*\/\s*([A-Za-z0-9.+-]+)/g, "$1 OR $2");
  }
  return licenseFile ? "LicenseRef-Cargo-License-File" : "NOASSERTION";
}

function cargoDownloadLocation(component) {
  if (component.source?.startsWith("registry+")) {
    return `https://crates.io/api/v1/crates/${encodeURIComponent(
      component.name,
    )}/${encodeURIComponent(component.version)}/download`;
  }
  if (component.source?.startsWith("git+")) {
    return component.source.slice(4);
  }
  return "NOASSERTION";
}

function cargoSpdxId(component) {
  return `SPDXRef-Package-${safeName(component.name)}-${safeName(
    component.version,
  )}-${sha256Text(component.id).slice(0, 12)}`;
}

function findProjectLicensePath() {
  for (const name of ["LICENSE", "LICENSE.txt", "LICENSE.md", "COPYING"]) {
    if (existsSync(join(repositoryRoot, name))) {
      return name;
    }
  }
  return null;
}

function writeJson(relativePath, value) {
  writeText(relativePath, `${JSON.stringify(value, null, 2)}\n`);
}

function writeText(relativePath, value) {
  const safePath = normalizeSafeRelativePath(relativePath, "output file");
  const absolutePath = join(outputDirectory, ...safePath.split("/"));
  assertPathWithinRoot(outputDirectory, absolutePath, "output file");
  mkdirSync(dirname(absolutePath), { recursive: true });
  writeFileSync(absolutePath, value, { encoding: "utf8", flag: "wx" });
}

function listFilesRecursively(root) {
  const files = [];
  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) {
        throw new Error(`Evidence output must not contain symlinks: ${join(directory, entry.name)}`);
      }
      const candidate = join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(candidate);
      } else if (entry.isFile()) {
        files.push(relative(root, candidate).replaceAll("\\", "/"));
      } else {
        throw new Error(`Unsupported evidence output entry: ${candidate}`);
      }
    }
  }
  visit(root);
  return files.sort();
}

function inputReport(path, absolutePath) {
  const safePath = normalizeSafeRelativePath(path, "compliance input");
  const canonicalPath = resolve(repositoryRoot, ...safePath.split("/"));
  assertPathWithinRoot(repositoryRoot, canonicalPath, "compliance input");
  if (resolve(absolutePath) !== canonicalPath) {
    throw new Error(`Compliance input path does not match its repository path: ${safePath}`);
  }
  assertNoLinkedPathWithinRoot(repositoryRoot, canonicalPath, "compliance input");
  const item = lstatSync(canonicalPath);
  if (!item.isFile() || item.isSymbolicLink()) {
    throw new Error(`Compliance input must be a regular file: ${safePath}`);
  }
  return {
    path: safePath,
    sizeBytes: item.size,
    sha256: sha256File(canonicalPath),
  };
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function sha256Text(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

function normalizeSafeRelativePath(value, description) {
  if (!value || isAbsolute(value) || value.includes("\0")) {
    throw new Error(`${description} must be a non-empty relative path.`);
  }
  const normalized = value.replaceAll("\\", "/");
  const segments = normalized.split("/");
  if (segments.some((segment) => !segment || segment === "." || segment === "..")) {
    throw new Error(`${description} contains an unsafe path segment: ${value}`);
  }
  return normalized;
}

function assertPathWithinRoot(root, candidate, description) {
  const relativePath = relative(resolve(root), resolve(candidate));
  if (!relativePath || relativePath === ".." || relativePath.startsWith(`..${sep}`) || isAbsolute(relativePath)) {
    throw new Error(`${description} escapes its root: ${candidate}`);
  }
}

function assertNoLinkedPathWithinRoot(root, target, description) {
  const absoluteRoot = resolve(root);
  const absoluteTarget = resolve(target);
  assertPathWithinRoot(absoluteRoot, absoluteTarget, description);
  let cursor = absoluteTarget;
  while (true) {
    const item = lstatSync(cursor);
    if (item.isSymbolicLink()) {
      throw new Error(`${description} traverses a symbolic link or junction: ${cursor}`);
    }
    if (cursor === absoluteRoot) {
      break;
    }
    const parent = dirname(cursor);
    if (parent === cursor) {
      throw new Error(`${description} could not be proven to remain inside the repository.`);
    }
    cursor = parent;
  }
  const realRoot = realpathSync(absoluteRoot);
  const realTarget = realpathSync(absoluteTarget);
  const realRelative = relative(realRoot, realTarget);
  if (!realRelative || realRelative === ".." ||
      realRelative.startsWith(`..${sep}`) || isAbsolute(realRelative)) {
    throw new Error(`${description} resolves outside the repository: ${absoluteTarget}`);
  }
}

function safeName(value) {
  const normalized = String(value)
    .normalize("NFKD")
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "unnamed";
}

function comparePackages(left, right) {
  return `${left.name}\0${left.version ?? left.versionInfo ?? ""}\0${
    left.id ?? left.SPDXID ?? ""
  }`.localeCompare(
    `${right.name}\0${right.version ?? right.versionInfo ?? ""}\0${
      right.id ?? right.SPDXID ?? ""
    }`,
  );
}

function escapeMarkdown(value) {
  return String(value).replaceAll("|", "\\|").replaceAll("\n", " ");
}
