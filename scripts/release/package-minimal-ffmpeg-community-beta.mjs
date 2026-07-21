#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import {
  closeSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, parse, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url));
const DEFAULT_REPOSITORY_ROOT = resolve(SCRIPT_DIRECTORY, "../..");
const DEFAULT_CONTRACT = "third_party/ffmpeg/community-beta-minimal-windows-x64.json";
const HEX_64 = /^[0-9a-f]{64}$/;
const SAFE_FILE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

function fail(message) {
  throw new Error(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function parseArguments(argv) {
  const values = new Map();
  const allowed = new Set([
    "--candidate-root",
    "--contract",
    "--output",
    "--repository-root",
  ]);
  for (let index = 0; index < argv.length; index += 1) {
    const name = argv[index];
    if (name === "--help") {
      return { help: true };
    }
    assert(allowed.has(name), `Unknown argument '${name}'.`);
    assert(!values.has(name), `Duplicate argument '${name}'.`);
    const value = argv[index + 1];
    assert(value && !value.startsWith("--"), `Argument '${name}' requires a value.`);
    values.set(name, value);
    index += 1;
  }
  assert(values.has("--candidate-root"), "--candidate-root is required.");
  assert(values.has("--output"), "--output is required.");
  return {
    help: false,
    candidateRoot: values.get("--candidate-root"),
    contract: values.get("--contract") ?? DEFAULT_CONTRACT,
    output: values.get("--output"),
    repositoryRoot: values.get("--repository-root") ?? DEFAULT_REPOSITORY_ROOT,
  };
}

function printHelp() {
  process.stdout.write(
    [
      "usage: node package-minimal-ffmpeg-community-beta.mjs \\",
      "  --candidate-root <verified-candidate-artifact> \\",
      "  --output <new-package-directory> \\",
      "  [--repository-root <repository>] \\",
      "  [--contract <repository-relative-contract>]",
      "",
      "The candidate root must contain the exact artifact produced by",
      "ffmpeg-minimal-candidate.yml plus WINDOWS-VERIFICATION.json.",
      "The output is a technical community-beta package, not distribution approval.",
      "",
    ].join("\n"),
  );
}

function assertNoLinkInExistingChain(pathValue, description) {
  const absolute = resolve(pathValue);
  const root = parse(absolute).root;
  const remainder = absolute.slice(root.length).split(sep).filter(Boolean);
  let current = root;
  for (const segment of remainder) {
    current = join(current, segment);
    if (!existsSync(current)) break;
    assert(!lstatSync(current).isSymbolicLink(), `${description} traverses a symlink or junction: ${current}`);
  }
}

function canonicalDirectory(pathValue, description) {
  const absolute = resolve(pathValue);
  assert(existsSync(absolute), `${description} does not exist: ${absolute}`);
  assertNoLinkInExistingChain(absolute, description);
  const item = lstatSync(absolute);
  assert(item.isDirectory(), `${description} is not a directory: ${absolute}`);
  return realpathSync.native(absolute);
}

function canonicalFile(pathValue, description) {
  const absolute = resolve(pathValue);
  assert(existsSync(absolute), `${description} does not exist: ${absolute}`);
  assertNoLinkInExistingChain(absolute, description);
  const item = lstatSync(absolute);
  assert(item.isFile() && item.size > 0, `${description} must be a non-empty regular file: ${absolute}`);
  return realpathSync.native(absolute);
}

function safeRelativePath(value, description) {
  assert(typeof value === "string" && value.length > 0, `${description} must be a non-empty string.`);
  assert(!isAbsolute(value) && !value.includes("\\") && !value.includes("\0"), `${description} must be a portable relative path.`);
  const segments = value.split("/");
  assert(segments.every((segment) => segment && segment !== "." && segment !== ".."), `${description} contains an unsafe segment.`);
  return segments;
}

function resolveInside(root, portablePath, description) {
  const segments = safeRelativePath(portablePath, description);
  const candidate = resolve(root, ...segments);
  const prefix = root.endsWith(sep) ? root : `${root}${sep}`;
  assert(candidate.startsWith(prefix), `${description} escapes its root.`);
  return candidate;
}

function readJson(pathValue, description) {
  try {
    return JSON.parse(readFileSync(pathValue, "utf8"));
  } catch (error) {
    fail(`${description} is not valid JSON: ${error.message}`);
  }
}

function sha256File(pathValue) {
  const hash = createHash("sha256");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  const handle = openSync(pathValue, "r");
  try {
    while (true) {
      const count = readSync(handle, buffer, 0, buffer.length, null);
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
    }
  } finally {
    closeSync(handle);
  }
  return hash.digest("hex");
}

function requireObject(value, description) {
  assert(value && typeof value === "object" && !Array.isArray(value), `${description} must be an object.`);
  return value;
}

function requireString(value, description) {
  assert(typeof value === "string" && value.length > 0, `${description} must be a non-empty string.`);
  return value;
}

function requireBoolean(value, expected, description) {
  assert(typeof value === "boolean" && value === expected, `${description} must be ${expected}.`);
}

function requirePositiveInteger(value, description) {
  assert(Number.isSafeInteger(value) && value > 0, `${description} must be a positive integer.`);
  return value;
}

function requireHash(value, description) {
  assert(typeof value === "string" && HEX_64.test(value), `${description} must be a lowercase SHA-256.`);
  return value;
}

function requireStringArray(value, description) {
  assert(Array.isArray(value) && value.length > 0, `${description} must be a non-empty array.`);
  const seen = new Set();
  for (const entry of value) {
    assert(typeof entry === "string" && entry.length > 0 && !seen.has(entry), `${description} contains an invalid or duplicate value.`);
    seen.add(entry);
  }
  return value;
}

function assertExactArray(actual, expected, description) {
  assert(Array.isArray(actual) && actual.length === expected.length, `${description} length does not match.`);
  for (let index = 0; index < expected.length; index += 1) {
    assert(actual[index] === expected[index], `${description} differs at index ${index}.`);
  }
}

function assertExactSet(actual, expected, description) {
  const actualSet = new Set(actual);
  const expectedSet = new Set(expected);
  assert(actualSet.size === actual.length && expectedSet.size === expected.length, `${description} contains duplicates.`);
  assert(actualSet.size === expectedSet.size && [...expectedSet].every((item) => actualSet.has(item)), `${description} does not match the required set.`);
}

function assertX64Pe(pathValue) {
  const size = statSync(pathValue).size;
  assert(size >= 70, "ffmpeg.exe is too small to be a PE file.");
  const handle = openSync(pathValue, "r");
  try {
    const dos = Buffer.alloc(64);
    assert(readSync(handle, dos, 0, dos.length, 0) === dos.length, "ffmpeg.exe DOS header is truncated.");
    assert(dos.readUInt16LE(0) === 0x5a4d, "ffmpeg.exe is missing the MZ signature.");
    const peOffset = dos.readUInt32LE(0x3c);
    assert(peOffset >= 0x40 && peOffset <= size - 6, "ffmpeg.exe PE offset is invalid.");
    const pe = Buffer.alloc(6);
    assert(readSync(handle, pe, 0, pe.length, peOffset) === pe.length, "ffmpeg.exe PE header is truncated.");
    assert(pe.readUInt32LE(0) === 0x00004550, "ffmpeg.exe is missing the PE signature.");
    assert(pe.readUInt16LE(4) === 0x8664, "ffmpeg.exe is not an x64 executable.");
  } finally {
    closeSync(handle);
  }
}

function assertXz(pathValue) {
  const expected = Buffer.from([0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]);
  const actual = Buffer.alloc(expected.length);
  const handle = openSync(pathValue, "r");
  try {
    assert(readSync(handle, actual, 0, actual.length, 0) === actual.length, "corresponding-source archive is truncated.");
  } finally {
    closeSync(handle);
  }
  assert(actual.equals(expected), "corresponding-source archive is not an XZ stream.");
}

function parseChecksums(pathValue, requiredNames) {
  const lines = readFileSync(pathValue, "utf8").split(/\r?\n/u).filter(Boolean);
  const entries = new Map();
  for (const line of lines) {
    const match = /^([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)$/u.exec(line);
    assert(match, `SHA256SUMS.txt contains an invalid line: '${line}'.`);
    assert(requiredNames.includes(match[2]) && !entries.has(match[2]), `SHA256SUMS.txt contains an unknown or duplicate file '${match[2]}'.`);
    entries.set(match[2], match[1]);
  }
  assertExactSet([...entries.keys()], requiredNames, "SHA256SUMS.txt file set");
  return entries;
}

function validateContract(contract) {
  assert(contract.schemaVersion === 1, "Unsupported community-beta FFmpeg contract schemaVersion.");
  assert(contract.status === "community-beta-technical-packaging-contract", "Community-beta contract status is invalid.");
  assert(contract.channel === "community-beta", "Community-beta contract channel is invalid.");
  assert(contract.platform === "windows" && contract.architecture === "x86_64", "Community-beta contract target is invalid.");
  assert(/^[0-9a-f]{40}$/u.test(contract.sourceCommit), "Community-beta contract sourceCommit is invalid.");
  const requiredInput = requireObject(contract.requiredInput, "requiredInput");
  requireStringArray(requiredInput.artifactFiles, "requiredInput.artifactFiles");
  for (const name of requiredInput.artifactFiles) {
    assert(SAFE_FILE_NAME.test(name), `requiredInput artifact file name is unsafe: '${name}'.`);
  }
  assert(requiredInput.candidateStatus === "candidate-not-promoted", "Candidate status contract is unsafe.");
  assert(requiredInput.buildMetadataStatus === "built-candidate-not-promoted", "Build metadata status contract is unsafe.");
  assert(requiredInput.windowsVerificationStatus === "passed-candidate-not-promoted", "Windows verification status contract is unsafe.");
  requireBoolean(requiredInput.windowsVerificationPromotionAuthorized, false, "requiredInput.windowsVerificationPromotionAuthorized");
  assert(SAFE_FILE_NAME.test(requireString(requiredInput.checksumManifest, "requiredInput.checksumManifest")), "Checksum manifest file name is unsafe.");
  assert(SAFE_FILE_NAME.test(requireString(requiredInput.windowsVerificationFile, "requiredInput.windowsVerificationFile")), "Windows verification file name is unsafe.");
  requirePositiveInteger(requiredInput.maximumExecutableBytes, "requiredInput.maximumExecutableBytes");
  requirePositiveInteger(requiredInput.maximumSourceArchiveBytes, "requiredInput.maximumSourceArchiveBytes");
  requireString(contract.candidateManifest, "candidateManifest");
  assert(Array.isArray(contract.licenseMaterials) && contract.licenseMaterials.length === 2, "licenseMaterials must contain LGPLv3 and GPLv3 texts.");
  const outputNames = new Set();
  for (const material of contract.licenseMaterials) {
    requireString(material.sourcePath, "license material sourcePath");
    const outputFileName = requireString(material.outputFileName, "license material outputFileName");
    assert(SAFE_FILE_NAME.test(outputFileName) && !outputNames.has(outputFileName), "license material output name is unsafe or duplicate.");
    outputNames.add(material.outputFileName);
    requirePositiveInteger(material.sizeBytes, `license material '${material.outputFileName}' sizeBytes`);
    requireHash(material.sha256, `license material '${material.outputFileName}' sha256`);
  }
  const output = requireObject(contract.output, "output");
  assert(output.status === "prepared-community-beta-candidate-not-public-release-approved", "Community-beta package output status is unsafe.");
  for (const field of [
    "runtimeRelativePath",
    "licenseRootRelativePath",
    "releaseAssetsRootRelativePath",
    "sourceArchiveRelativePath",
    "buildEvidenceRootRelativePath",
    "windowsVerificationRelativePath",
  ]) {
    safeRelativePath(output[field], `output.${field}`);
  }
  assert(SAFE_FILE_NAME.test(requireString(output.packageManifestFileName, "output.packageManifestFileName")), "Package manifest output file name is unsafe.");
  assert(SAFE_FILE_NAME.test(requireString(output.checksumManifestFileName, "output.checksumManifestFileName")), "Checksum manifest output file name is unsafe.");
  const boundary = requireObject(contract.complianceBoundary, "complianceBoundary");
  requireBoolean(boundary.technicalPackagingOnly, true, "complianceBoundary.technicalPackagingOnly");
  requireBoolean(boundary.communityBetaDistributionAuthorized, false, "complianceBoundary.communityBetaDistributionAuthorized");
  requireBoolean(boundary.publicReleaseApproved, false, "complianceBoundary.publicReleaseApproved");
  requireBoolean(boundary.modifiesPublicReleasePolicy, false, "complianceBoundary.modifiesPublicReleasePolicy");
  requireBoolean(boundary.requiresReleaseOwnerApprovalBeforeDistribution, true, "complianceBoundary.requiresReleaseOwnerApprovalBeforeDistribution");
  requireBoolean(boundary.requiresCodecPatentReviewBeforeStrictPublicRelease, true, "complianceBoundary.requiresCodecPatentReviewBeforeStrictPublicRelease");
  requireBoolean(boundary.requiresToolchainRuntimeLicenseReviewBeforeStrictPublicRelease, true, "complianceBoundary.requiresToolchainRuntimeLicenseReviewBeforeStrictPublicRelease");
  requireString(boundary.notice, "complianceBoundary.notice");
}

function validateCandidate({ artifactRoot, candidate, contract }) {
  const requiredInput = contract.requiredInput;
  assert(candidate.schemaVersion === 1 && candidate.status === requiredInput.candidateStatus, "Candidate manifest schema or status is invalid.");
  assert(candidate.platform === contract.platform && candidate.architecture === contract.architecture, "Candidate manifest target does not match the beta contract.");
  assert(candidate.source?.commit === contract.sourceCommit, "Candidate source commit does not match the beta contract.");
  assert(candidate.build?.licenseExpression === "LGPL-3.0-or-later", "Candidate license expression is invalid.");
  assert(Array.isArray(candidate.build.externalLibraries) && candidate.build.externalLibraries.length === 0, "Candidate must not enable FFmpeg external libraries.");
  const flags = requireStringArray(candidate.build.configureFlags, "candidate configureFlags");
  for (const required of [
    "--disable-autodetect",
    "--disable-everything",
    "--disable-network",
    "--enable-protocol=file",
    "--enable-demuxer=mov",
    "--enable-parser=h264",
    "--enable-decoder=h264",
    "--enable-filter=scale",
    "--enable-encoder=mjpeg",
    "--enable-muxer=image2",
    "--enable-version3",
  ]) {
    assert(flags.includes(required), `Candidate configure flags are missing '${required}'.`);
  }
  assert(!flags.includes("--enable-gpl") && !flags.includes("--enable-nonfree"), "Candidate enables GPL or nonfree code.");
  assert(!flags.some((flag) => flag.startsWith("--enable-lib")), "Candidate enables an external FFmpeg library.");

  const expectedRootNames = [
    ...requiredInput.artifactFiles,
    requiredInput.checksumManifest,
    requiredInput.windowsVerificationFile,
  ];
  const rootEntries = readdirSync(artifactRoot, { withFileTypes: true });
  assertExactSet(rootEntries.map((entry) => entry.name), expectedRootNames, "candidate artifact root");
  for (const entry of rootEntries) {
    assert(entry.isFile() && !entry.isSymbolicLink(), `Candidate artifact contains a directory or link: '${entry.name}'.`);
  }

  const artifactPaths = new Map();
  for (const name of requiredInput.artifactFiles) {
    artifactPaths.set(name, canonicalFile(join(artifactRoot, name), `candidate artifact '${name}'`));
  }
  const checksumPath = canonicalFile(join(artifactRoot, requiredInput.checksumManifest), "candidate checksum manifest");
  const checksums = parseChecksums(checksumPath, requiredInput.artifactFiles);
  for (const [name, expectedHash] of checksums) {
    assert(sha256File(artifactPaths.get(name)) === expectedHash, `Candidate artifact '${name}' does not match SHA256SUMS.txt.`);
  }

  const ffmpegPath = artifactPaths.get("ffmpeg.exe");
  const sourceArchivePath = artifactPaths.get("ffmpeg-corresponding-source.tar.xz");
  assert(ffmpegPath && sourceArchivePath, "Candidate artifact omits ffmpeg.exe or corresponding source.");
  const executableSize = statSync(ffmpegPath).size;
  const sourceArchiveSize = statSync(sourceArchivePath).size;
  assert(executableSize <= requiredInput.maximumExecutableBytes, "ffmpeg.exe exceeds the beta contract size limit.");
  assert(sourceArchiveSize <= requiredInput.maximumSourceArchiveBytes, "Corresponding-source archive exceeds the beta contract size limit.");
  assertX64Pe(ffmpegPath);
  assertXz(sourceArchivePath);

  const metadata = readJson(artifactPaths.get("BUILD-METADATA.json"), "candidate build metadata");
  assert(metadata.schemaVersion === 1 && metadata.status === requiredInput.buildMetadataStatus, "Candidate build metadata schema or status is invalid.");
  assert(metadata.sourceCommit === contract.sourceCommit, "Candidate build metadata source commit is invalid.");
  assert(metadata.targetTriple === candidate.build.targetTriple, "Candidate build metadata target triple is invalid.");
  assert(metadata.licenseExpression === candidate.build.licenseExpression, "Candidate build metadata license is invalid.");
  assertExactArray(metadata.configureFlags, flags, "candidate build metadata configure flags");
  assert(Array.isArray(metadata.externalLibraries) && metadata.externalLibraries.length === 0, "Candidate build metadata declares external libraries.");
  requirePositiveInteger(metadata.sourceDateEpoch, "candidate build metadata sourceDateEpoch");
  assert(metadata.executable?.fileName === "ffmpeg.exe", "Candidate build metadata executable name is invalid.");
  assert(metadata.executable.sizeBytes === executableSize, "Candidate build metadata executable size is invalid.");
  assert(metadata.executable.sha256 === checksums.get("ffmpeg.exe"), "Candidate build metadata executable hash is invalid.");
  const recordedFlags = readFileSync(artifactPaths.get("configure-flags.txt"), "utf8").trimEnd().split(/\r?\n/u);
  assertExactArray(recordedFlags, flags, "configure-flags.txt");
  for (const name of ["compiler-version.txt", "config.h", "config.mak", "pe-imports.txt"]) {
    assert(readFileSync(artifactPaths.get(name)).length > 0, `Candidate evidence '${name}' is empty.`);
  }

  const verificationPath = canonicalFile(join(artifactRoot, requiredInput.windowsVerificationFile), "Windows candidate verification");
  const verification = readJson(verificationPath, "Windows candidate verification");
  assert(verification.schemaVersion === 1 && verification.status === requiredInput.windowsVerificationStatus, "Windows verification schema or status is invalid.");
  requireBoolean(verification.promotionAuthorized, false, "Windows verification promotionAuthorized");
  assert(verification.sourceCommit === contract.sourceCommit, "Windows verification source commit is invalid.");
  assert(verification.licenseExpression === candidate.build.licenseExpression, "Windows verification license is invalid.");
  assert(Array.isArray(verification.externalLibraries) && verification.externalLibraries.length === 0, "Windows verification declares external libraries.");
  assert(verification.executable?.sizeBytes === executableSize && verification.executable?.sha256 === checksums.get("ffmpeg.exe"), "Windows verification executable record is invalid.");
  assert(Number.isFinite(Date.parse(verification.checkedAtUtc)), "Windows verification checkedAtUtc is invalid.");

  const artifactEvidence = requireObject(verification.artifactEvidence, "Windows verification artifactEvidence");
  assert(artifactEvidence.checksumManifestSha256 === sha256File(checksumPath), "Windows verification checksum manifest hash is invalid.");
  assert(artifactEvidence.sourceArchiveSha256 === checksums.get("ffmpeg-corresponding-source.tar.xz"), "Windows verification source archive hash is invalid.");
  const verificationFiles = requireObject(artifactEvidence.files, "Windows verification artifactEvidence.files");
  assertExactSet(Object.keys(verificationFiles), requiredInput.artifactFiles, "Windows verification evidence file set");
  for (const name of requiredInput.artifactFiles) {
    assert(verificationFiles[name] === checksums.get(name), `Windows verification evidence hash is invalid for '${name}'.`);
  }
  const requiredCapabilities = requireObject(verification.requiredCapabilities, "Windows verification requiredCapabilities");
  for (const [kind, names] of Object.entries(candidate.runtimeContract.requiredCapabilities)) {
    assertExactArray(requiredCapabilities[kind], names, `Windows verification ${kind} capabilities`);
  }
  assert(verification.smoke?.fixtureSha256 === candidate.runtimeContract.smokeFixture.sha256, "Windows verification smoke fixture is invalid.");
  requirePositiveInteger(verification.smoke?.outputSizeBytes, "Windows verification smoke outputSizeBytes");
  assert(verification.smoke.outputSizeBytes <= candidate.runtimeContract.maximumOutputBytes, "Windows verification smoke output is too large.");
  requireHash(verification.smoke?.outputSha256, "Windows verification smoke outputSha256");

  return {
    artifactPaths,
    candidateFlags: flags,
    checksums,
    checksumPath,
    executableSize,
    ffmpegPath,
    metadata,
    sourceArchivePath,
    sourceArchiveSize,
    verification,
    verificationPath,
  };
}

function collectFiles(root) {
  const files = [];
  function walk(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      assert(!entry.isSymbolicLink(), `Generated package contains a link: '${full}'.`);
      if (entry.isDirectory()) walk(full);
      else {
        assert(entry.isFile(), `Generated package contains a non-file entry: '${full}'.`);
        files.push(full);
      }
    }
  }
  walk(root);
  return files.sort((left, right) => left.localeCompare(right, "en"));
}

function portableRelative(root, pathValue) {
  return relative(root, pathValue).split(sep).join("/");
}

function makeFileRecord(root, pathValue) {
  return {
    path: portableRelative(root, pathValue),
    sizeBytes: statSync(pathValue).size,
    sha256: sha256File(pathValue),
  };
}

function writeUtf8(pathValue, text) {
  mkdirSync(dirname(pathValue), { recursive: true });
  writeFileSync(pathValue, text.endsWith("\n") ? text : `${text}\n`, { encoding: "utf8", flag: "wx" });
}

function copyPayload(source, destination) {
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination, 1);
}

function buildPackage({ candidateManifestHash, contract, contractHash, contractRelativePath, outputPath, repositoryRoot, validated }) {
  const requestedOutputPath = resolve(outputPath);
  const outputParent = canonicalDirectory(dirname(requestedOutputPath), "package output parent");
  const outputName = parse(requestedOutputPath).base;
  assert(outputName && outputName !== "." && outputName !== "..", "Package output must name a child of its existing parent.");
  const finalPath = join(outputParent, outputName);
  assert(!existsSync(finalPath), `Package output already exists: ${finalPath}`);
  const temporaryPath = join(outputParent, `.${outputName}.tmp-${randomUUID()}`);
  assert(!existsSync(temporaryPath), "Temporary package path unexpectedly exists.");
  mkdirSync(temporaryPath);
  try {
    const output = contract.output;
    const runtimePath = resolveInside(temporaryPath, output.runtimeRelativePath, "runtime output path");
    const licenseRoot = resolveInside(temporaryPath, output.licenseRootRelativePath, "license output root");
    const releaseAssetsRoot = resolveInside(temporaryPath, output.releaseAssetsRootRelativePath, "release assets root");
    const sourceOutputPath = resolveInside(temporaryPath, output.sourceArchiveRelativePath, "source archive output path");
    const buildEvidenceRoot = resolveInside(temporaryPath, output.buildEvidenceRootRelativePath, "build evidence output root");
    const verificationOutputPath = resolveInside(temporaryPath, output.windowsVerificationRelativePath, "Windows verification output path");
    assert(SAFE_FILE_NAME.test(output.packageManifestFileName), "Package manifest output file name is unsafe.");
    assert(SAFE_FILE_NAME.test(output.checksumManifestFileName), "Checksum manifest output file name is unsafe.");

    copyPayload(validated.ffmpegPath, runtimePath);
    copyPayload(validated.sourceArchivePath, sourceOutputPath);
    for (const name of contract.requiredInput.artifactFiles) {
      if (name === "ffmpeg.exe" || name === "ffmpeg-corresponding-source.tar.xz") continue;
      copyPayload(validated.artifactPaths.get(name), join(buildEvidenceRoot, name));
    }
    copyPayload(validated.checksumPath, join(buildEvidenceRoot, contract.requiredInput.checksumManifest));
    copyPayload(validated.verificationPath, verificationOutputPath);

    for (const material of contract.licenseMaterials) {
      const source = canonicalFile(resolveInside(repositoryRoot, material.sourcePath, `license source '${material.outputFileName}'`), `license source '${material.outputFileName}'`);
      assert(statSync(source).size === material.sizeBytes, `License source '${material.outputFileName}' size does not match the contract.`);
      assert(sha256File(source) === material.sha256, `License source '${material.outputFileName}' hash does not match the contract.`);
      copyPayload(source, join(licenseRoot, material.outputFileName));
    }

    const sourceRelative = output.sourceArchiveRelativePath;
    const sourceHash = validated.checksums.get("ffmpeg-corresponding-source.tar.xz");
    const executableHash = validated.checksums.get("ffmpeg.exe");
    const buildInfo = {
      schemaVersion: 1,
      status: "community-beta-technical-candidate",
      channel: contract.channel,
      component: "FFmpeg",
      sourceCommit: contract.sourceCommit,
      sourceDateEpoch: validated.metadata.sourceDateEpoch,
      targetTriple: validated.metadata.targetTriple,
      licenseExpression: validated.metadata.licenseExpression,
      configureFlags: validated.candidateFlags,
      ffmpegExternalLibraries: [],
      executable: {
        fileName: "ffmpeg.exe",
        sizeBytes: validated.executableSize,
        sha256: executableHash,
      },
      correspondingSource: {
        fileName: "ffmpeg-corresponding-source.tar.xz",
        packageRelativePath: sourceRelative,
        sizeBytes: validated.sourceArchiveSize,
        sha256: sourceHash,
      },
      windowsVerification: {
        status: validated.verification.status,
        checkedAtUtc: validated.verification.checkedAtUtc,
        sha256: sha256File(validated.verificationPath),
      },
      complianceBoundary: contract.complianceBoundary,
    };
    writeUtf8(join(licenseRoot, "BUILD-INFO.json"), `${JSON.stringify(buildInfo, null, 2)}\n`);

    const sourceOffer = [
      "# FFmpeg corresponding source for the community-beta candidate",
      "",
      "This package contains an unmodified, separately executed FFmpeg command-line program built from:",
      "",
      `- FFmpeg commit: \`${contract.sourceCommit}\``,
      `- License: \`${validated.metadata.licenseExpression}\``,
      `- Executable SHA-256: \`${executableHash}\``,
      `- Corresponding-source archive: \`${sourceRelative}\``,
      `- Corresponding-source SHA-256: \`${sourceHash}\``,
      "",
      "Any channel that distributes an installer containing this executable must make the exact corresponding-source archive available beside that installer at no additional charge and keep a clear source link next to the binary download.",
      "",
      "This generated notice records technical provenance only. Community-beta distribution additionally requires the repository release-owner decision. Codec-patent and toolchain-runtime-license review remain gates for the strict public-release channel.",
      "",
    ].join("\n");
    writeUtf8(join(licenseRoot, "SOURCE-OFFER.md"), sourceOffer);

    const notice = [
      "# FFmpeg community-beta component notice",
      "",
      "This package uses FFmpeg under the GNU Lesser General Public License version 3 or later. FFmpeg is executed as a separate command-line program. The accompanying LGPLv3 and GPLv3 texts and exact corresponding source are included in this package.",
      "",
      "This software is based in part on the work of the Independent JPEG Group.",
      "",
      "FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project. The FFmpeg project does not endorse this application.",
      "",
      contract.complianceBoundary.notice,
      "",
    ].join("\n");
    writeUtf8(join(licenseRoot, "THIRD-PARTY-NOTICE.md"), notice);

    const manifestPath = join(temporaryPath, output.packageManifestFileName);
    const checksumOutputPath = join(temporaryPath, output.checksumManifestFileName);
    const payloadFiles = collectFiles(temporaryPath).map((pathValue) => makeFileRecord(temporaryPath, pathValue));
    const packageManifest = {
      schemaVersion: 1,
      status: output.status,
      channel: contract.channel,
      createdAtUtc: new Date().toISOString(),
      sourceCommit: contract.sourceCommit,
      contract: {
        path: contractRelativePath,
        sha256: contractHash,
      },
      technicalPromotion: {
        installerResourceOverlayPrepared: true,
        correspondingSourcePackaged: true,
        licensesPackaged: true,
        buildEvidencePackaged: true,
        windowsRuntimeVerificationPassed: true,
        communityBetaDistributionAuthorized: false,
        publicReleaseApproved: false,
      },
      executable: {
        path: output.runtimeRelativePath,
        sizeBytes: validated.executableSize,
        sha256: executableHash,
      },
      correspondingSource: {
        path: sourceRelative,
        sizeBytes: validated.sourceArchiveSize,
        sha256: sourceHash,
      },
      inputEvidence: {
        candidateManifestSha256: candidateManifestHash,
        buildMetadataSha256: validated.checksums.get("BUILD-METADATA.json"),
        candidateChecksumManifestSha256: sha256File(validated.checksumPath),
        windowsVerificationSha256: sha256File(validated.verificationPath),
      },
      payloadFiles,
      complianceBoundary: contract.complianceBoundary,
    };
    writeUtf8(manifestPath, `${JSON.stringify(packageManifest, null, 2)}\n`);

    const filesForChecksum = collectFiles(temporaryPath)
      .filter((pathValue) => pathValue !== checksumOutputPath)
      .map((pathValue) => makeFileRecord(temporaryPath, pathValue));
    const checksumText = filesForChecksum
      .map((file) => `${file.sha256}  ${file.path}`)
      .join("\n");
    writeUtf8(checksumOutputPath, `${checksumText}\n`);
    renameSync(temporaryPath, finalPath);
    return {
      output: finalPath,
      packageManifest: join(finalPath, output.packageManifestFileName),
      checksumManifest: join(finalPath, output.checksumManifestFileName),
      status: output.status,
      executableSha256: executableHash,
      correspondingSourceSha256: sourceHash,
      communityBetaDistributionAuthorized: false,
      publicReleaseApproved: false,
    };
  } catch (error) {
    if (existsSync(temporaryPath)) rmSync(temporaryPath, { recursive: true, force: true });
    throw error;
  }
}

function main() {
  const args = parseArguments(process.argv.slice(2));
  if (args.help) {
    printHelp();
    return;
  }
  const repositoryRoot = canonicalDirectory(args.repositoryRoot, "repository root");
  assert(!isAbsolute(args.contract), "--contract must be repository-relative.");
  const contractPath = canonicalFile(resolveInside(repositoryRoot, args.contract.replaceAll("\\", "/"), "community-beta contract"), "community-beta contract");
  const contract = readJson(contractPath, "community-beta contract");
  validateContract(contract);
  const candidateManifestPath = canonicalFile(resolveInside(repositoryRoot, contract.candidateManifest, "candidate manifest"), "candidate manifest");
  const candidate = readJson(candidateManifestPath, "candidate manifest");
  const artifactRoot = canonicalDirectory(args.candidateRoot, "verified candidate artifact root");
  const validated = validateCandidate({ artifactRoot, candidate, contract });
  const summary = buildPackage({
    candidateManifestHash: sha256File(candidateManifestPath),
    contract,
    contractHash: sha256File(contractPath),
    contractRelativePath: args.contract.replaceAll("\\", "/"),
    outputPath: resolve(args.output),
    repositoryRoot,
    validated,
  });
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`Community-beta FFmpeg packaging failed: ${error.message}\n`);
  process.exitCode = 1;
}
