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
const generatedAt = new Date().toISOString();
const licenseTextCatalog = new Map();

assertSafeOutputDirectory(outputDirectory);
mkdirSync(outputDirectory, { recursive: false });

const packageJsonPath = join(repositoryRoot, "package.json");
const packageLockPath = join(repositoryRoot, "package-lock.json");
const cargoTomlPath = join(repositoryRoot, "src-tauri", "Cargo.toml");
const cargoLockPath = join(repositoryRoot, "src-tauri", "Cargo.lock");
const ffmpegManifestPath = join(
  repositoryRoot,
  "third_party",
  "ffmpeg",
  "windows-x64.json",
);
const ffmpegExecutablePath = join(
  repositoryRoot,
  "src-tauri",
  "resources",
  "bin",
  "ffmpeg.exe",
);

const packageJson = readJson(packageJsonPath);
const ffmpegManifest = readJson(ffmpegManifestPath);
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

const npmLicenseIndex = collectNpmLicenseTexts(npmRuntimePackages);
const cargoLicenseIndex = collectCargoLicenseTexts(cargoThirdPartyPackages);

const cargoSpdx = createCargoSpdx(cargoMetadata, targetTriple, generatedAt);
const ffmpegComponent = createFfmpegComponent(ffmpegManifest, ffmpegExecutablePath);
const blockers = collectBlockers({
  packageJson,
  npmLicenseIndex,
  cargoLicenseIndex,
  ffmpegManifest,
});

writeJson("npm-runtime.spdx.json", npmRuntimeSpdx);
writeJson("npm-build.spdx.json", npmBuildSpdx);
writeJson("cargo-windows-x64.spdx.json", cargoSpdx);
writeJson("ffmpeg-component.json", ffmpegComponent);
writeJson("LICENSE-TEXTS-INDEX.json", {
  schemaVersion: 1,
  generatedAt,
  npm: npmLicenseIndex,
  cargo: cargoLicenseIndex,
});
writeText("THIRD-PARTY-LICENSES.txt", createConsolidatedLicenseText());

const summary = {
  schemaVersion: 1,
  status: blockers.length === 0 ? "ready-for-approval" : "generated-with-blockers",
  generatedAt,
  target: targetTriple,
  publicRedistributionReady: blockers.length === 0,
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
  blockers,
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
    blockers,
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
  generatedAt,
  generator: {
    path: "scripts/release/generate-compliance-evidence.mjs",
    sha256: sha256File(fileURLToPath(import.meta.url)),
    node: process.version,
  },
  inputs: [
    inputReport("package.json", packageJsonPath),
    inputReport("package-lock.json", packageLockPath),
    inputReport("src-tauri/Cargo.toml", cargoTomlPath),
    inputReport("src-tauri/Cargo.lock", cargoLockPath),
    inputReport("third_party/ffmpeg/windows-x64.json", ffmpegManifestPath),
  ],
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
  const parsed = { output: null, target: null, offline: false };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (argument === "--offline") {
      parsed.offline = true;
      continue;
    }
    if (argument === "--output" || argument === "--target") {
      const value = argumentsList[index + 1];
      if (!value || value.startsWith("--")) {
        throw new Error(`${argument} requires a value.`);
      }
      parsed[argument.slice(2)] = value;
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
  return parsed;
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
  const npmCliCandidates = [
    process.env.npm_execpath,
    join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js"),
  ].filter(Boolean);
  const npmCli = npmCliCandidates.find((candidate) => existsSync(candidate));
  if (!npmCli) {
    throw new Error("Could not locate npm-cli.js for SBOM generation.");
  }
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

function collectBlockers({ packageJson: project, npmLicenseIndex, cargoLicenseIndex, ffmpegManifest: manifest }) {
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
  return {
    path,
    sizeBytes: statSync(absolutePath).size,
    sha256: sha256File(absolutePath),
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
