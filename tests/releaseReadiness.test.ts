import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

type JsonObject = Record<string, unknown>;

const packageJson = readJson("../package.json");
const packageLock = readJson("../package-lock.json");
const tauriConfig = readJson("../src-tauri/tauri.conf.json");
const cargoManifest = readFileSync(
  new URL("../src-tauri/Cargo.toml", import.meta.url),
  "utf8",
);
const projectLicenseText = readFileSync(
  new URL("../LICENSE", import.meta.url),
  "utf8",
);
const projectLicenseScopeText = readFileSync(
  new URL("../LICENSE-SCOPE.txt", import.meta.url),
  "utf8",
);
const licensingDecision = readFileSync(
  new URL("../docs/LICENSING.md", import.meta.url),
  "utf8",
);
const ffmpegManifest = readJson("../third_party/ffmpeg/windows-x64.json");
const licenseTextOverrides = readJson(
  "../third_party/licenses/license-text-overrides.json",
);
const licenseTextOverrideApprovals = readJson(
  "../third_party/licenses/license-text-override-approvals.json",
);
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
  assert.deepEqual(bundle.resources, {
    "resources/": "",
    "../LICENSE": "licenses/project/LICENSE.txt",
    "../LICENSE-SCOPE.txt": "licenses/project/LICENSE-SCOPE.txt",
  });
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

test("missing dependency license texts use locked and reviewable overrides", () => {
  assert.equal(licenseTextOverrides.schemaVersion, 1);
  const policy = objectAt(licenseTextOverrides, "policy");
  for (const field of [
    "offlineOnly",
    "exactComponentMatchRequired",
    "unusedOverrideIsError",
    "localPackageLicenseFilesTakePrecedence",
    "reviewDoesNotConstituteLegalApproval",
  ]) {
    assert.equal(policy[field], true, `${field} must remain fail-closed`);
  }

  const texts = licenseTextOverrides.texts as JsonObject[];
  const overrides = licenseTextOverrides.overrides as JsonObject[];
  assert.equal(texts.length, 9);
  assert.equal(overrides.length, 12);

  const textIds = new Set<string>();
  for (const text of texts) {
    const id = String(text.id);
    const path = String(text.path);
    assert.equal(textIds.has(id), false, `duplicate override text id: ${id}`);
    textIds.add(id);
    assert.match(path, /^third_party\/licenses\/texts\/[A-Za-z0-9._-]+$/);
    const bytes = readFileSync(new URL(`../${path}`, import.meta.url));
    assert.equal(bytes.byteLength, text.sizeBytes);
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      text.sha256,
    );
    const source = objectAt(text, "source");
    assert.match(String(source.url), /^https:\/\//);
    assert.equal(typeof source.relationship, "string");
    assert.equal(
      text.spdxLicenseId === null || typeof text.spdxLicenseId === "string",
      true,
    );
    assert.equal(Array.isArray(text.equivalentSources), true);
    for (const equivalent of text.equivalentSources as JsonObject[]) {
      assert.equal(equivalent.relationship, "byte-identical-upstream-file");
      assert.match(String(equivalent.revision), /^[a-f0-9]{40}$/);
      assert.match(String(equivalent.url), /^https:\/\//);
    }
  }

  const expectedComponents = new Set([
    "npm:react-remove-scroll-bar@2.3.8",
    "cargo:alloc-stdlib@0.2.4",
    "cargo:selectors@0.36.1",
    "cargo:tauri-plugin@2.6.3",
    "cargo:unic-char-property@0.9.0",
    "cargo:unic-char-range@0.9.0",
    "cargo:unic-common@0.9.0",
    "cargo:unic-ucd-ident@0.9.0",
    "cargo:unic-ucd-version@0.9.0",
    "cargo:webview2-com-macros@0.8.1",
    "cargo:webview2-com-sys@0.38.2",
    "cargo:webview2-com@0.38.2",
  ]);
  for (const override of overrides) {
    const component = `${String(override.ecosystem)}:${String(override.name)}@${String(override.version)}`;
    assert.equal(expectedComponents.delete(component), true, component);
    if (override.ecosystem === "cargo") {
      assert.match(String(override.vcsRevision), /^[a-f0-9]{40}$/);
    } else {
      assert.match(String(override.registryGitHead), /^[a-f0-9]{40}$/);
    }
    assert.equal(Array.isArray(override.textIds), true);
    for (const textId of override.textIds as string[]) {
      assert.equal(textIds.has(textId), true, `${component}: ${textId}`);
    }
    assert.equal("review" in override, false);
  }
  assert.equal(expectedComponents.size, 0);

  assert.equal(licenseTextOverrideApprovals.schemaVersion, 1);
  assert.equal(Array.isArray(licenseTextOverrideApprovals.approvals), true);
  assert.equal((licenseTextOverrideApprovals.approvals as JsonObject[]).length, 0);

  const npmOverride = overrides.find(
    (entry) => entry.ecosystem === "npm",
  ) as JsonObject;
  const lockedNpmPackage = objectAt(
    objectAt(packageLock, "packages"),
    "node_modules/react-remove-scroll-bar",
  );
  assert.equal(npmOverride.lockIntegrity, lockedNpmPackage.integrity);
  assert.equal(npmOverride.resolved, lockedNpmPackage.resolved);
  assert.match(String(npmOverride.registryTarballSha1), /^[a-f0-9]{40}$/);
  assert.match(String(npmOverride.registryGitHead), /^[a-f0-9]{40}$/);

  const selectorsOverride = overrides.find(
    (entry) => entry.name === "selectors",
  ) as JsonObject;
  assert.deepEqual(selectorsOverride.obligations, [
    "mpl-2.0-source-code-form-review-required",
  ]);

  assert.match(complianceGenerator, /tracked-license-override/);
  assert.match(complianceGenerator, /\.cargo_vcs_info\.json/);
  assert.match(complianceGenerator, /License override is stale because the package now includes local text/);
  assert.match(complianceGenerator, /does not match Cargo\.lock/);
  assert.match(complianceGenerator, /LICENSE_OVERRIDE_REVIEW_PENDING/);
  assert.match(complianceGenerator, /License override text SPDX coverage mismatch/);
  assert.match(complianceGenerator, /must be tracked in the Git index and match its indexed bytes/);
  assert.match(complianceGenerator, /Locked npm tarball does not match its SHA-512\/SHA-1 provenance/);
  assert.match(complianceGenerator, /approval manifest contains unknown or stale components/);
  assert.match(bundleGateScript, /tracked license-text override manifest/);
  assert.match(bundleGateScript, /structured license-text override approval manifest/);
  assert.match(bundleGateScript, /License-text override manifest contains an unsafe text path/);
  assert.match(bundleGateScript, /does not match its manifest size and SHA-256/);
  assert.match(bundleGateScript, /does not have exact SPDX text coverage/);
  assert.match(bundleGateScript, /text records that are not bound to a component/);
  assert.match(bundleGateScript, /pending blockers do not match the structured license override approvals/);
});

test("first-party licensing is consistently declared as MIT", () => {
  assert.equal(packageJson.license, "MIT");
  assert.equal(
    objectAt(objectAt(packageLock, "packages"), "").license,
    "MIT",
  );
  assert.match(cargoManifest, /^license = "MIT"$/m);
  assert.match(projectLicenseText, /^MIT License$/m);
  assert.match(projectLicenseText, /Copyright \(c\) 2026 VALOFRAME Contributors/);
  assert.match(projectLicenseText, /Permission is hereby granted, free of charge/);

  const projectLicense = objectAt(publicReleasePolicy, "projectLicense");
  assert.equal(projectLicense.approved, true);
  assert.equal(projectLicense.spdxExpression, "MIT");
  assert.equal(projectLicense.file, "LICENSE");
  assert.equal(
    createHash("sha256").update(projectLicenseText).digest("hex"),
    projectLicense.sha256,
  );
  assert.equal(projectLicense.scopeFile, "LICENSE-SCOPE.txt");
  assert.equal(
    createHash("sha256").update(projectLicenseScopeText).digest("hex"),
    projectLicense.scopeSha256,
  );
  assert.equal(
    projectLicense.approvalReference,
    "docs/LICENSING.md#项目许可决定",
  );

  const eula = objectAt(publicReleasePolicy, "eula");
  assert.equal(eula.required, false);
  assert.equal(eula.approved, true);
  assert.equal(eula.file, null);
  assert.match(licensingDecision, /当前公开发布决定是不再另设一份重复的最终用户许可协议/);
  assert.match(licensingDecision, /MIT 不会改变仓库中第三方材料原有的许可或权利归属/);
  assert.match(projectLicenseScopeText, /are not licensed under the\nMIT License/);
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
    "PROJECT_LICENSE_BUNDLE_MISSING",
    "PROJECT_LICENSE_HASH_MISMATCH",
    "PROJECT_LICENSE_SCOPE_MISSING",
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
  assert.match(publicReleasePreflight, /separate-EULA requirement must be an explicit JSON Boolean decision/);
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
  assert.match(bundleGateScript, /Bundled project MIT license must exactly match the repository root LICENSE/);
  assert.match(bundleGateScript, /Bundled project license scope must exactly match the repository root LICENSE-SCOPE\.txt/);
  assert.match(bundleGateScript, /Bundled project license does not contain the approved MIT license markers/);
  assert.match(bundleGateScript, /Project license or scope bytes do not match the approved public release policy SHA-256/);
  assert.match(bundleGateScript, /VerifiedPayloadOutputDirectory/);
  assert.match(bundleGateScript, /Assert-VerifiedPayloadMatchesReport/);
  assert.match(bundleGateScript, /rootAndParentChainRecheckedAfterMove/);
  assert.equal(
    bundleGateScript.includes('File\\s+"\\$\\{MAINBINARYSRCPATH\\}"'),
    true,
  );

  for (const destination of [
    "bin\\ffmpeg.exe",
    "licenses\\project\\LICENSE.txt",
    "licenses\\project\\LICENSE-SCOPE.txt",
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
  assert.match(releaseWorkflow, /licenses\\project\\LICENSE-SCOPE\.txt/);
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
