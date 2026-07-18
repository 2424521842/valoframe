import assert from "node:assert/strict";
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

test("FFmpeg input is immutable, hashed, and not marked redistribution-ready", () => {
  const provider = objectAt(ffmpegManifest, "provider");
  const artifact = objectAt(ffmpegManifest, "artifact");
  const sourceCompliance = objectAt(ffmpegManifest, "sourceCompliance");

  assert.match(String(provider.releaseTag), /^autobuild-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}$/);
  assert.doesNotMatch(String(artifact.url), /\/latest(?:\/|$)/i);
  assert.match(String(artifact.sha256), /^[a-f0-9]{64}$/);
  assert.match(String(artifact.executableSha256), /^[a-f0-9]{64}$/);
  assert.equal(sourceCompliance.redistributionReady, false);
  assert.equal(
    ffmpegBuildInfo.redistributionStatus,
    sourceCompliance.status,
    "internal blocked BUILD-INFO and manifest states must remain identical",
  );
  assert.equal(
    objectAt(sourceCompliance, "correspondingSourceBundle").url,
    null,
  );

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

test("bundle gate proves NSIS format and the six shipped payload files", () => {
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
    /\$nsisSignature = Get-SignatureReport[^\n]*-PermitUnsigned \$permitUnsigned/,
  );
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
