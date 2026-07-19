import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

type JsonObject = Record<string, unknown>;

const packageJson = readJson("../package.json");
const tauriConfig = readJson("../src-tauri/tauri.conf.json");
const ffmpegManifest = readJson("../third_party/ffmpeg/windows-x64.json");
const ffmpegBuildInfo = readJson(
  "../src-tauri/resources/licenses/ffmpeg/BUILD-INFO.json",
);
const bundleGateScript = readFileSync(
  new URL("../scripts/release/check-bundle.ps1", import.meta.url),
  "utf8",
);
const releaseWorkflow = readFileSync(
  new URL("../.github/workflows/windows-release-readiness.yml", import.meta.url),
  "utf8",
);
const releaseSmokeScript = readFileSync(
  new URL("../scripts/release/windows-release-smoke.ps1", import.meta.url),
  "utf8",
);
const complianceGenerator = readFileSync(
  new URL(
    "../scripts/release/generate-compliance-evidence.mjs",
    import.meta.url,
  ),
  "utf8",
);
const publicReleasePolicy = readJson("../release/public-release-policy.json");
const publicReleasePreflight = readFileSync(
  new URL(
    "../scripts/release/public-release-preflight.ps1",
    import.meta.url,
  ),
  "utf8",
);
const minimalFfmpegCandidate = readJson(
  "../third_party/ffmpeg/minimal-windows-x64-candidate.json",
);
const minimalFfmpegBuildScript = readFileSync(
  new URL("../scripts/release/build-minimal-ffmpeg.sh", import.meta.url),
  "utf8",
);
const minimalFfmpegVerifyScript = readFileSync(
  new URL(
    "../scripts/release/verify-minimal-ffmpeg-candidate.ps1",
    import.meta.url,
  ),
  "utf8",
);
const minimalFfmpegWorkflow = readFileSync(
  new URL(
    "../.github/workflows/ffmpeg-minimal-candidate.yml",
    import.meta.url,
  ),
  "utf8",
);

test("Windows bundle policy is explicit and downgrade-safe", () => {
  const app = objectAt(tauriConfig, "app");
  const bundle = objectAt(tauriConfig, "bundle");
  const windows = objectAt(bundle, "windows");
  const nsis = objectAt(windows, "nsis");
  const webviewInstallMode = objectAt(windows, "webviewInstallMode");

  assert.deepEqual(bundle.targets, ["nsis"]);
  assert.deepEqual(bundle.resources, { "resources/": "" });
  assert.equal(windows.allowDowngrades, false);
  assert.deepEqual(webviewInstallMode, {
    type: "downloadBootstrapper",
    silent: true,
  });
  assert.equal(nsis.installMode, "currentUser");
  assert.deepEqual(nsis.languages, ["SimpChinese", "English"]);
  const windowsConfig = app.windows;
  assert.equal(Array.isArray(windowsConfig), true);
  assert.equal(
    (windowsConfig as JsonObject[])[0]?.create,
    false,
    "the main window must be created only after the release-smoke root is validated",
  );
});

test("package, Tauri, and Cargo versions remain aligned", () => {
  const cargoToml = readFileSync(
    new URL("../src-tauri/Cargo.toml", import.meta.url),
    "utf8",
  );
  const cargoVersion = cargoToml.match(
    /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
  )?.[1];

  assert.equal(packageJson.version, tauriConfig.version);
  assert.equal(packageJson.version, cargoVersion);
});

test("FFmpeg input is immutable, hashed, and follows the redistribution state machine", () => {
  const provider = objectAt(ffmpegManifest, "provider");
  const artifact = objectAt(ffmpegManifest, "artifact");
  const sourceCompliance = objectAt(ffmpegManifest, "sourceCompliance");
  const sourceBundle = objectAt(sourceCompliance, "correspondingSourceBundle");

  assert.match(String(provider.releaseTag), /^autobuild-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}$/);
  assert.doesNotMatch(String(artifact.url), /\/latest(?:\/|$)/i);
  assert.match(String(artifact.sha256), /^[a-f0-9]{64}$/);
  assert.match(String(artifact.executableSha256), /^[a-f0-9]{64}$/);
  assert.equal(typeof sourceCompliance.redistributionReady, "boolean");
  assert.equal(
    ffmpegBuildInfo.redistributionStatus,
    sourceCompliance.status,
    "BUILD-INFO and manifest redistribution states must remain identical",
  );

  if (sourceCompliance.redistributionReady === true) {
    assert.equal(sourceCompliance.status, "ready-for-redistribution");
    assert.match(String(artifact.projectMirrorUrl), /^https:\/\//);
    assert.equal(artifact.projectMirrorUrl, sourceCompliance.binaryMirrorUrl);
    assert.match(String(sourceBundle.url), /^https:\/\//);
    assert.equal(
      typeof sourceBundle.sizeBytes === "number" && sourceBundle.sizeBytes > 0,
      true,
    );
    assert.match(String(sourceBundle.sha256), /^[a-f0-9]{64}$/);
    assert.equal(sourceCompliance.thirdPartyLicenseAuditComplete, true);
    assert.equal(sourceCompliance.ijgAttributionIncluded, true);
    assert.match(String(sourceCompliance.patentReviewStatus), /^(approved|not-required)$/);
    assert.equal(
      typeof sourceCompliance.legalApprovalReference === "string" &&
        sourceCompliance.legalApprovalReference.trim().length > 0,
      true,
    );
  } else {
    assert.notEqual(
      sourceCompliance.status,
      "ready-for-redistribution",
      "a blocked manifest must not use the ready state label",
    );
  }

  for (const path of [
    "../src-tauri/resources/licenses/ffmpeg/COPYING.LGPLv3.txt",
    "../src-tauri/resources/licenses/ffmpeg/COPYING.GPLv3.txt",
    "../src-tauri/resources/licenses/ffmpeg/BUILD-INFO.json",
    "../src-tauri/resources/licenses/ffmpeg/SOURCE-OFFER.md",
  ]) {
    assert.equal(existsSync(new URL(path, import.meta.url)), true, `${path} must exist`);
  }
});

test("public bundle gate mirrors the FFmpeg redistribution contract", () => {
  for (const field of [
    "redistributionReady",
    "status",
    "projectMirrorUrl",
    "binaryMirrorUrl",
    "correspondingSourceBundle",
    "thirdPartyLicenseAuditComplete",
    "ijgAttributionIncluded",
    "patentReviewStatus",
    "legalApprovalReference",
    "redistributionStatus",
  ]) {
    assert.match(bundleGateScript, new RegExp(`\\b${field}\\b`));
  }
  assert.match(bundleGateScript, /ready-for-redistribution/);
  assert.match(bundleGateScript, /approved.*not-required/s);
  assert.match(
    bundleGateScript,
    /buildRedistributionStatus[\s\S]*sourceComplianceStatus[\s\S]*StringComparison\]::Ordinal/,
  );
});

test("compliance evidence covers locked npm, Cargo, and FFmpeg inputs", () => {
  for (const output of [
    "npm-runtime.spdx.json",
    "npm-build.spdx.json",
    "cargo-windows-x64.spdx.json",
    "ffmpeg-component.json",
    "LICENSE-TEXTS-INDEX.json",
    "THIRD-PARTY-LICENSES.txt",
    "THIRD-PARTY-NOTICES.md",
    "COMPLIANCE-SUMMARY.json",
    "COMPLIANCE-MANIFEST.json",
  ]) {
    assert.equal(complianceGenerator.includes(`\"${output}\"`), true);
  }
  assert.match(complianceGenerator, /--package-lock-only/);
  assert.match(complianceGenerator, /--sbom-format[\s\S]*spdx/);
  assert.match(complianceGenerator, /--filter-platform/);
  assert.match(complianceGenerator, /Output directory must not already exist/);
  assert.match(complianceGenerator, /Third-party packages without a license declaration/);
  assert.match(releaseWorkflow, /generate-compliance-evidence\.mjs/);
  assert.match(releaseWorkflow, /src-tauri\/resources\/licenses\/third-party/);
  const scripts = objectAt(packageJson, "scripts");
  assert.match(
    String(scripts["release:compliance:generate"]),
    /--output src-tauri\/resources\/licenses\/third-party --offline/,
  );
  assert.match(bundleGateScript, /target must be x86_64-pc-windows-msvc/);
  assert.match(bundleGateScript, /approved release generator/);
});

test("public release preflight covers every non-bundle approval domain", () => {
  assert.equal(publicReleasePolicy.schemaVersion, 1);
  assert.equal(publicReleasePolicy.releaseMode, "public");
  assert.equal(
    objectAt(publicReleasePolicy, "identity").identifier,
    tauriConfig.identifier,
  );

  for (const code of [
    "PROJECT_LICENSE_APPROVAL_MISSING",
    "EULA_APPROVAL_MISSING",
    "THIRD_PARTY_APPROVAL_MISSING",
    "BRAND_APPROVAL_MISSING",
    "PUBLISHER_APPROVAL_MISSING",
    "IDENTIFIER_APPROVAL_MISSING",
    "ICON_DISTRIBUTION_RIGHTS_MISSING",
    "DISCLAIMER_APPROVAL_MISSING",
    "FFMPEG_REDISTRIBUTION_BLOCKED",
    "CODE_SIGNING_NOT_READY",
    "TIMESTAMP_NOT_READY",
    "SIGNTOOL_VERIFICATION_NOT_READY",
    "CLEAN_VM_EVIDENCE_MISSING",
    "CLEAN_VM_EVIDENCE_INVALID",
    "UPDATER_DECISION_PENDING",
    "DATA_SAFETY_APPROVAL_MISSING",
    "DATA_SAFETY_EVIDENCE_INVALID",
  ]) {
    assert.equal(publicReleasePreflight.includes(`'${code}'`), true);
  }
  assert.match(publicReleasePreflight, /-RequireReady/);
  assert.match(publicReleasePreflight, /-ExpectBlocked/);
  assert.match(publicReleasePreflight, /Get-EvidenceValidationError/);
  assert.match(publicReleasePreflight, /evidence artifact.*does not match its SHA-256/);
  assert.match(publicReleasePreflight, /sourceCommit does not match the release commit/);
  assert.equal(
    objectAt(publicReleasePolicy, "authenticode").signtoolVerificationRequired,
    true,
  );
  assert.match(releaseWorkflow, /public-release-preflight\.ps1/);
  assert.match(releaseWorkflow, /public-release-preflight\.json/);
});

test("minimal FFmpeg candidate is narrow, self-built, and cannot self-promote", () => {
  assert.equal(minimalFfmpegCandidate.status, "candidate-not-promoted");
  const build = objectAt(minimalFfmpegCandidate, "build");
  assert.deepEqual(build.externalLibraries, []);
  const flags = build.configureFlags as string[];
  for (const requiredFlag of [
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
  ]) {
    assert.equal(flags.includes(requiredFlag), true, `${requiredFlag} must be pinned`);
  }
  assert.equal(flags.some((flag) => flag === "--enable-gpl"), false);
  assert.equal(flags.some((flag) => flag === "--enable-nonfree"), false);
  assert.equal(flags.some((flag) => flag.startsWith("--enable-lib")), false);
  assert.equal(
    flags.some((flag) => /--extra-version=.*ce3c09c101c8/.test(flag)),
    true,
  );
  const runtimeContract = objectAt(minimalFfmpegCandidate, "runtimeContract");
  assert.equal(
    (runtimeContract.allowedSystemDllImports as string[]).includes("KERNEL32.dll"),
    true,
  );
  const smokeFixture = objectAt(runtimeContract, "smokeFixture");
  const smokeBytes = Buffer.from(String(smokeFixture.base64), "base64");
  assert.equal(smokeBytes.length, smokeFixture.sizeBytes);
  assert.equal(
    createHash("sha256").update(smokeBytes).digest("hex"),
    smokeFixture.sha256,
  );
  assert.equal(smokeBytes.includes(Buffer.from("avc1", "ascii")), true);
  assert.equal(smokeBytes.includes(Buffer.from("mp4v", "ascii")), false);
  assert.match(minimalFfmpegBuildScript, /source checkout must be clean/);
  assert.match(minimalFfmpegBuildScript, /SOURCE_DATE_EPOCH/);
  assert.match(minimalFfmpegBuildScript, /Output directory must not already exist/);
  assert.match(minimalFfmpegVerifyScript, /passed-candidate-not-promoted/);
  assert.match(minimalFfmpegVerifyScript, /promotionAuthorized = \$false/);
  assert.match(minimalFfmpegVerifyScript, /Get-PeImportedDlls/);
  assert.match(minimalFfmpegVerifyScript, /SHA256SUMS\.txt/);
  assert.match(minimalFfmpegVerifyScript, /objdump imports do not match the Windows PE parser/);
  assert.doesNotMatch(
    minimalFfmpegVerifyScript,
    /^\s*\$outputPath\s*=/im,
    "PowerShell variables are case-insensitive; a local $outputPath must not overwrite the -OutputPath parameter",
  );
  assert.match(minimalFfmpegWorkflow, /ffmpeg-corresponding-source\.tar\.xz/);
  assert.match(minimalFfmpegWorkflow, /mingw-w64 nasm xz-utils/);
  assert.match(minimalFfmpegWorkflow, /verify-minimal-ffmpeg-candidate\.ps1/);
});

test("bundle gate proves NSIS format and all shipped compliance payload files", () => {
  assert.match(bundleGateScript, /Get-NsisHeaderReport/);
  assert.match(bundleGateScript, /DEADBEEF\/NullsoftInst mismatch/);
  assert.match(bundleGateScript, /Type = Nsis/);
  assert.match(bundleGateScript, /SubType = NSIS-3 Unicode/);
  assert.match(bundleGateScript, /7-Zip controlled NSIS extraction/);
  assert.match(bundleGateScript, /tauri-nsis-marker-normalized/);
  assert.match(bundleGateScript, /__TAURI_BUNDLE_TYPE_VAR_UNK/);
  assert.match(bundleGateScript, /__TAURI_BUNDLE_TYPE_VAR_NSS/);
  assert.match(bundleGateScript, /normalizedMatchesExternal/);
  assert.match(bundleGateScript, /embeddedMainSignature/);
  assert.match(bundleGateScript, /strict-unsigned/);
  assert.match(bundleGateScript, /authenticode-aware/);
  assert.match(bundleGateScript, /Get-WinCertificateTableReport/);
  assert.match(bundleGateScript, /checksumOffset/);
  assert.match(bundleGateScript, /securityDirectoryOffset/);
  assert.match(bundleGateScript, /Align8\(external staging length\)/);
  assert.match(
    bundleGateScript,
    /Public release external UNK staging main executable must be NotSigned/,
  );
  assert.match(
    bundleGateScript,
    /-Description 'NSIS embedded main executable'[\s\S]*?-PermitUnsigned \$PermitUnsignedApplicationArtifacts/,
  );
  assert.match(
    bundleGateScript,
    /\$nsisSignature = Get-SignatureReport[\s\S]*?-PermitUnsigned \$permitUnsigned[\s\S]*?-SigningRequirements \$signingRequirements/,
  );
  assert.match(bundleGateScript, /expectedCertificateThumbprint/);
  assert.match(bundleGateScript, /TimeStamperCertificate/);
  assert.match(bundleGateScript, /signtool verify \/pa \/all \/v/);
  assert.match(bundleGateScript, /VerifiedPayloadOutputDirectory/);
  assert.match(bundleGateScript, /Assert-VerifiedPayloadMatchesReport/);
  assert.match(bundleGateScript, /rootAndParentChainRecheckedAfterMove/);
  assert.equal(
    bundleGateScript.includes('File\\s+"\\$\\{MAINBINARYSRCPATH\\}"'),
    true,
  );

  for (const destination of [
    "bin\\ffmpeg.exe",
    "licenses\\ffmpeg\\COPYING.LGPLv3.txt",
    "licenses\\ffmpeg\\COPYING.GPLv3.txt",
    "licenses\\ffmpeg\\BUILD-INFO.json",
    "licenses\\ffmpeg\\SOURCE-OFFER.md",
    "licenses\\third-party",
    "COMPLIANCE-MANIFEST.json",
    "COMPLIANCE-SUMMARY.json",
    "npm-runtime.spdx.json",
    "cargo-windows-x64.spdx.json",
    "THIRD-PARTY-LICENSES.txt",
    "THIRD-PARTY-NOTICES.md",
  ]) {
    assert.equal(bundleGateScript.includes(`'${destination}'`), true);
  }

  assert.match(releaseWorkflow, /-NsisScriptPath \$nsisScripts\[0\]\.FullName/);
  assert.match(releaseWorkflow, /-NsisExtractorPath \$sevenZip/);
  assert.match(
    releaseWorkflow,
    /-VerifiedPayloadOutputDirectory \$verifiedPayload/,
  );
  assert.match(releaseWorkflow, /Get-Command 7z\.exe -CommandType Application/);
  assert.match(
    releaseWorkflow,
    /\$expectedPayloadRoot = .*'vhm-verified-installer-payload'/,
  );
  assert.match(releaseWorkflow, /vhm-internal-rc-report\.json/);
  assert.match(releaseWorkflow, /ConvertFrom-Json -Depth 20/);
  assert.match(releaseWorkflow, /verifiedPayloadOutputVerification\.entriesMatchedReport/);
  assert.match(releaseWorkflow, /\[string\] \$entry\.sha256/);
  assert.match(
    releaseWorkflow,
    /normalization\.rawEmbeddedSha256/,
  );
  assert.match(
    releaseWorkflow,
    /-ExpectedExecutableSha256 \$expectedMainHash/,
  );
  assert.match(releaseWorkflow, /-ResourceDirectory \$payloadRoot/);
  assert.match(releaseWorkflow, /-FfmpegMode ResourceOverride/);
});

test("Windows startup smoke proves schema v13 and single-instance handoff", () => {
  assert.match(releaseSmokeScript, /\$ExpectedSchemaVersion\s*=\s*13/);
  assert.match(releaseSmokeScript, /clip_trash_snapshots/);
  assert.match(releaseSmokeScript, /trashSnapshotCount/);
  assert.match(releaseSmokeScript, /'clip_delete_intents'/);
  assert.match(releaseSmokeScript, /deleteIntentCount/);
  assert.match(releaseSmokeScript, /secondInstanceProcess\.WaitForRootExit/);
  assert.match(releaseSmokeScript, /onlyPrimaryNamedRootAfterHandoff/);
  assert.match(releaseSmokeScript, /primaryWindowHandlePreserved/);
  assert.match(releaseSmokeScript, /primaryWindowVisibleAfterHandoff/);
  assert.match(releaseSmokeScript, /primaryWindowMinimizedBeforeHandoff/);
  assert.match(releaseSmokeScript, /primaryWindowMinimizedAfterHandoff/);
  assert.match(releaseSmokeScript, /ClassName\s+-ceq\s+'Tauri Window'/);
  assert.match(releaseSmokeScript, /foregroundMatchesMainWindow/);
  assert.match(
    releaseSmokeScript,
    /sharedLaunchConfiguration[\s\S]*environmentOverrides[\s\S]*VHM_RELEASE_SMOKE_ROOT/,
  );
  assert.match(
    releaseWorkflow,
    /singleInstance\.sharedLaunchConfiguration\.environmentOverrides\.VHM_RELEASE_SMOKE_ROOT/,
  );
  assert.doesNotMatch(releaseWorkflow, /singleInstance\.sharedSmokeRoot/);
});

function readJson(path: string): JsonObject {
  return JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8")) as JsonObject;
}

function objectAt(value: JsonObject, key: string): JsonObject {
  const nested = value[key];
  assert.equal(
    typeof nested,
    "object",
    `expected ${key} to be an object`,
  );
  assert.notEqual(nested, null, `expected ${key} to be non-null`);
  assert.equal(Array.isArray(nested), false, `expected ${key} not to be an array`);
  return nested as JsonObject;
}
