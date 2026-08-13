import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");
const workflow = read(".github/workflows/stable-release.yml");
const stableConfig = JSON.parse(read("src-tauri/tauri.stable.conf.json"));
const publicPolicy = JSON.parse(read("release/public-release-policy.json"));

test("stable release is triggered only by canonical-looking version tags", () => {
  assert.match(
    workflow,
    /on:\r?\n\s+push:\r?\n\s+tags:\r?\n\s+- "v\*\.\*\.\*"/u,
  );
  assert.doesNotMatch(workflow, /workflow_dispatch:|pull_request:|schedule:/u);
  assert.doesNotMatch(workflow, /inputs\.|environment: stable-release/u);
  assert.match(workflow, /RELEASE_TAG: \$\{\{ github\.ref_name \}\}/u);
  assert.match(workflow, /\^v\(0\|\[1-9\]\[0-9\]\*\)\\\./u);
  assert.match(
    workflow,
    /\$env:GITHUB_REF -cne "refs\/tags\/\$env:RELEASE_TAG"/u,
  );
  assert.match(workflow, /2424521842\/valoframe/u);
});

test("tag checkout is bound to one source commit and all application versions", () => {
  assert.match(workflow, /ref: \$\{\{ github\.ref \}\}/u);
  assert.match(workflow, /git rev-parse HEAD/u);
  assert.match(workflow, /git cat-file -t "refs\/tags\/\$env:RELEASE_TAG"/u);
  assert.match(workflow, /git rev-list -n 1 "refs\/tags\/\$env:RELEASE_TAG"/u);
  assert.match(workflow, /pushed tag must be a lightweight tag/u);
  assert.match(workflow, /SOURCE_COMMIT=\$head/u);
  assert.match(
    workflow,
    /package, Tauri, Cargo, and stable tag versions must agree/u,
  );
  assert.match(workflow, /package\.json/u);
  assert.match(workflow, /src-tauri\/tauri\.conf\.json/u);
  assert.match(workflow, /src-tauri\/Cargo\.toml/u);
});

test("repository updater signing remains mandatory for the personal community channel", () => {
  assert.equal(stableConfig.bundle.createUpdaterArtifacts, true);
  assert.match(
    workflow,
    /TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/u,
  );
  assert.match(
    workflow,
    /TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/u,
  );
  assert.match(
    workflow,
    /VALOFRAME_UPDATER_PUBLIC_KEY: \$\{\{ vars\.VALOFRAME_UPDATER_PUBLIC_KEY \}\}/u,
  );
  for (const name of [
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "VALOFRAME_UPDATER_PUBLIC_KEY",
  ]) {
    assert.match(
      workflow,
      new RegExp(`IsNullOrWhiteSpace\\(\\$env:${name}\\)`, "u"),
    );
  }
  assert.doesNotMatch(
    workflow,
    /VALOFRAME_RELEASE_EVIDENCE|public-release-preflight|RequireReady|WINDOWS_CERTIFICATE|signtool/u,
  );
  assert.match(workflow, /check-bundle\.ps1/u);
  assert.match(workflow, /-AllowPersonalCommunityStable/u);
  assert.doesNotMatch(workflow, /-AllowUnsignedInternalRc|-AllowUnsignedCommunityBeta/u);
  assert.doesNotMatch(workflow, /public-release-policy\.json/u);
  assert.equal(publicPolicy.enforcement, "optional-future-hardening");
  assert.equal(publicPolicy.updater.decision, "enabled");
  assert.equal(publicPolicy.updater.tauriSignatureRequired, true);
  assert.equal(
    publicPolicy.updater.publicKeyReference,
    "repository-variable:VALOFRAME_UPDATER_PUBLIC_KEY",
  );
  assert.equal(
    publicPolicy.authenticode.requiredForPersonalUpdaterRelease,
    false,
  );
});

test("complete automated checks run before the signed build", () => {
  const baselineIndex = workflow.indexOf(
    "name: Run complete automated baseline",
  );
  const buildIndex = workflow.indexOf(
    "name: Build Tauri-signed updater artifacts",
  );
  assert.ok(baselineIndex > 0 && buildIndex > baselineIndex);
  assert.match(workflow, /npm test/u);
  assert.match(workflow, /npm run build/u);
  assert.match(workflow, /cargo fmt[\s\S]*?--check/u);
  assert.match(workflow, /cargo clippy[\s\S]*?-D warnings/u);
  assert.match(workflow, /cargo test/u);
  assert.match(workflow, /git diff --check/u);
  assert.doesNotMatch(workflow, /prepare-ffmpeg\.ps1|verify-ffmpeg\.ps1|-ValidationOnly/u);
  assert.match(workflow, /verify-minimal-ffmpeg-candidate\.ps1/u);
  assert.match(workflow, /windows-release-smoke\.ps1/u);
});

test("a pinned Ubuntu job builds exact minimal FFmpeg and transfers one run-bound artifact", () => {
  assert.match(workflow, /ffmpeg:\r?\n\s+name: Build exact minimal FFmpeg and corresponding source/u);
  assert.match(workflow, /runs-on: ubuntu-24\.04/u);
  assert.match(workflow, /FFMPEG_COMMIT: ce3c09c101c83add623774d414a9f9498caf5c25/u);
  assert.match(workflow, /git -C "\$RUNNER_TEMP\/ffmpeg-source" rev-parse HEAD/u);
  assert.match(workflow, /build-minimal-ffmpeg\.sh/u);
  assert.match(workflow, /minimal-windows-x64-candidate\.json/u);
  assert.match(workflow, /--sort=name[\s\S]*?ffmpeg-corresponding-source\.tar\.xz/u);
  assert.match(workflow, /sha256sum[\s\S]*?> SHA256SUMS\.txt/u);
  assert.match(
    workflow,
    /stable-ffmpeg-candidate-\$\{\{ github\.run_id \}\}-\$\{\{ github\.run_attempt \}\}/u,
  );
  assert.match(workflow, /release:\r?\n[\s\S]*?needs: ffmpeg/u);
});

test("Windows verifies, packages, stages, and installs personal community compliance before Tauri build", () => {
  const downloadIndex = workflow.indexOf(
    "name: Download the run-pinned minimal FFmpeg candidate",
  );
  const nativeVerifyIndex = workflow.indexOf(
    "name: Verify minimal FFmpeg on native Windows x64",
  );
  const stageIndex = workflow.indexOf(
    "name: Package and stage owner-authorized personal community FFmpeg",
  );
  const complianceIndex = workflow.indexOf(
    "name: Generate and install personal community compliance evidence",
  );
  const buildIndex = workflow.indexOf(
    "name: Build Tauri-signed updater artifacts",
  );
  assert.ok(
    downloadIndex > 0 &&
      nativeVerifyIndex > downloadIndex &&
      stageIndex > nativeVerifyIndex &&
      complianceIndex > stageIndex &&
      buildIndex > complianceIndex,
  );
  assert.match(
    workflow,
    /package-minimal-ffmpeg-community-beta\.mjs[\s\S]*?--contract third_party\/ffmpeg\/personal-community-stable-minimal-windows-x64\.json/u,
  );
  assert.match(workflow, /stage-personal-community-stable-ffmpeg\.ps1/u);
  assert.match(
    workflow,
    /-DecisionPath release\/approvals\/personal-community-stable-v0\.2\.1\.json/u,
  );
  assert.match(workflow, /--release-profile personal-community-stable/u);
  assert.match(workflow, /channelDistributionReady -ne \$true/u);
  assert.match(workflow, /Move-Item -LiteralPath \$source -Destination \$destination/u);
});

test("the personal community bundle gate owns the payload used by startup smoke", () => {
  const gateIndex = workflow.indexOf("check-bundle.ps1");
  const smokeIndex = workflow.indexOf("windows-release-smoke.ps1");
  assert.ok(gateIndex > 0 && smokeIndex > gateIndex);
  for (const parameter of [
    "-AllowPersonalCommunityStable",
    "-PersonalCommunityStableDecisionPath",
    "-FFmpegArchivePath",
    "-SourceBundlePath",
    "-VerifiedPayloadOutputDirectory",
  ]) {
    assert.match(workflow, new RegExp(parameter, "u"));
  }
  assert.match(workflow, /releaseMode -cne 'personal-community-stable'/u);
  assert.match(workflow, /nsisPayload\.verifiedPayloadOutputDirectory/u);
  assert.match(workflow, /normalization\.rawEmbeddedSha256/u);
  assert.match(workflow, /-ExecutablePath \$verifiedMain/u);
  assert.match(workflow, /-ResourceDirectory \$payloadRoot/u);
});

test("updater package shape, size, signature, tampering, and wrong key are verified", () => {
  assert.match(workflow, /\.nsis\.zip/u);
  assert.match(workflow, /System\.IO\.Compression\.ZipFile/u);
  assert.match(workflow, /瓦刻_\$env:APP_VERSION`_x64-setup\.exe/u);
  assert.match(workflow, /536870912/u);
  assert.match(
    workflow,
    /Updater ZIP does not contain exactly the bounded root installer expected by the runtime/u,
  );
  assert.match(workflow, /example verify_updater_signature/u);
  assert.match(workflow, /Valid updater signature was rejected/u);
  assert.match(
    workflow,
    /one-byte updater-package mutation unexpectedly verified/u,
  );
  assert.match(workflow, /wrong updater public key unexpectedly verified/u);
  assert.match(workflow, /create-updater-manifest\.mjs/u);
});

test("release notes are optional and receive a safe default", () => {
  assert.match(workflow, /release\\notes\\\$env:RELEASE_TAG\.md/u);
  assert.match(workflow, /vhm-default-release-notes\.md/u);
  assert.match(workflow, /本版本包含功能改进、错误修复与自动更新支持/u);
  assert.match(workflow, /--notes-file \$env:STABLE_PUBLISH_NOTES/u);
  assert.match(workflow, /RELEASE-NOTES\.md/u);
  assert.match(workflow, /个人社区发行说明/u);
  assert.match(workflow, /free of charge/u);
  assert.match(workflow, /not affiliated with, sponsored by, or endorsed by Riot Games, Tencent, FFmpeg/u);
  assert.match(workflow, /Unknown Publisher/u);
  assert.match(workflow, /Tauri updater packages remain cryptographically signed/u);
});

test("the release carries FFmpeg source, notices, manifest, and compliance beside the installer", () => {
  for (const asset of [
    "valoframe-ffmpeg-minimal-windows-x64.zip",
    "ffmpeg-corresponding-source.tar.xz",
    "FFMPEG-MANIFEST.json",
    "FFMPEG-SOURCE-OFFER.md",
    "personal-community-stable-compliance.zip",
    "PERSONAL-COMMUNITY-STABLE-NOTICE.txt",
  ]) {
    assert.match(workflow, new RegExp(asset.replaceAll(".", "\\."), "u"));
  }
  const assemblyIndex = workflow.indexOf(
    "name: Assemble and independently verify stable release assets",
  );
  const hashIndex = workflow.indexOf("SHA256SUMS.txt", assemblyIndex);
  const draftIndex = workflow.indexOf("gh release create $env:RELEASE_TAG");
  assert.ok(assemblyIndex > 0 && hashIndex > assemblyIndex && draftIndex > hashIndex);
  assert.match(
    workflow,
    /SHA256SUMS\.txt must cover every other release asset exactly once/u,
  );
  assert.match(workflow, /\$declaredNames\.Add\(\$name\)/u);
  assert.match(workflow, /Set-Content[^\r\n]+-Encoding utf8NoBOM/u);
  assert.doesNotMatch(workflow, /SHA256SUMS[^\r\n]*-Encoding ascii/u);
  assert.match(
    workflow,
    /SHA256SUMS\.txt does not match release asset/u,
  );
});

test("stable release verification uses numeric GitHub database IDs", () => {
  assert.match(workflow, /--json databaseId,isDraft,isPrerelease,tagName/u);
  assert.match(workflow, /\[long\] \$published\.databaseId/u);
  assert.match(workflow, /\[long\] \$release\.databaseId/u);
  assert.match(workflow, /\[long\] \$rolledBack\.databaseId/u);
  assert.doesNotMatch(workflow, /\[long\] \$(?:published|release|rolledBack)\.id/u);
});

test("stable releases serialize globally and expose write credentials only to gh steps", () => {
  assert.match(
    workflow,
    /concurrency:\r?\n\s+group: stable-release\r?\n\s+cancel-in-progress: false/u,
  );
  const jobEnvironment = workflow.slice(
    workflow.indexOf("    env:"),
    workflow.indexOf("    steps:"),
  );
  assert.doesNotMatch(jobEnvironment, /GH_TOKEN/u);

  const ghSteps = workflow
    .split(/\r?\n(?= {6}- name:)/u)
    .filter((step) => /\bgh (?:api|release) /u.test(step));
  assert.ok(ghSteps.length >= 5);
  for (const step of ghSteps) {
    assert.match(step, /env:\r?\n\s+GH_TOKEN: \$\{\{ github\.token \}\}/u);
  }
});

test("GitHub actions are pinned to immutable revisions", () => {
  const actionUses = [
    ...workflow.matchAll(/uses: (actions\/[a-z-]+)@([^\s#]+)/gu),
  ];
  assert.equal(actionUses.length, 7);
  for (const [, action, revision] of actionUses) {
    assert.match(
      action,
      /^actions\/(?:checkout|setup-node|setup-python|upload-artifact|download-artifact)$/u,
    );
    assert.match(revision, /^[0-9a-f]{40}$/u);
  }
  assert.match(
    workflow,
    /actions\/download-artifact@018cc2cf5baa6db3ef3c5f8a56943fffe632ef53/u,
  );
});

test("publication rejects duplicates and non-increasing SemVer before draft creation", () => {
  assert.match(workflow, /Refusing to overwrite existing release/u);
  assert.doesNotMatch(
    workflow,
    /Refusing to reuse existing tag|stable tag .* is absent/u,
  );
  assert.match(
    workflow,
    /gh api --paginate "repos\/\$env:GITHUB_REPOSITORY\/releases\?per_page=100"/u,
  );
  assert.match(workflow, /Compare-StableReleaseVersion/u);
  const monotonicIndex = workflow.indexOf(
    "must be newer than highest published stable release",
  );
  const buildIndex = workflow.indexOf(
    "name: Build Tauri-signed updater artifacts",
  );
  const draftIndex = workflow.indexOf("gh release create $env:RELEASE_TAG");
  assert.ok(
    monotonicIndex > 0 &&
      buildIndex > monotonicIndex &&
      draftIndex > buildIndex,
  );
  assert.match(workflow, /--verify-tag/u);
  assert.doesNotMatch(workflow, /--target/u);
  assert.match(workflow, /--draft/u);
});

test("draft assets and updater signatures are reverified before publication", () => {
  const remoteVerifyIndex = workflow.indexOf(
    "name: Redownload and verify every remote draft asset before publication",
  );
  const publishIndex = workflow.indexOf(
    "name: Publish the complete draft as the stable latest release",
  );
  assert.ok(remoteVerifyIndex > 0 && publishIndex > remoteVerifyIndex);
  assert.match(workflow, /gh release download/u);
  assert.match(
    workflow,
    /Remote draft asset differs from the verified local file/u,
  );
  assert.match(workflow, /Remote updater signature verification failed/u);
  assert.match(
    workflow,
    /Stable lightweight tag does not point to the checked-out source commit/u,
  );
  assert.match(workflow, /--draft=false/u);
  assert.match(workflow, /--prerelease=false/u);
  assert.match(workflow, /--latest/u);
});

test("public latest is verified and a failed verification rolls back to draft", () => {
  const steps = workflow.split(/\r?\n(?= {6}- name:)/u);
  const publishStep = steps.find((step) =>
    step.includes("id: publish_stable_release"),
  );
  const verifyStep = steps.find((step) =>
    step.includes("id: verify_public_release"),
  );
  const rollbackStep = steps.find((step) =>
    step.includes(
      "name: Restore the release to draft if public verification fails",
    ),
  );
  assert.ok(publishStep && verifyStep && rollbackStep);
  assert.match(workflow, /releases\/latest\/download\/latest\.json/u);
  assert.match(
    workflow,
    /Public updater package signature verification failed/u,
  );
  assert.match(
    rollbackStep,
    /failure\(\) && steps\.publish_stable_release\.outcome == 'success' && steps\.verify_public_release\.outcome == 'failure'/u,
  );
  assert.match(rollbackStep, /--draft=true[\s\S]*?--latest=false/u);
  assert.match(rollbackStep, /vhm-stable-public-rollback\.json/u);
});

test("each updater manifest command fails closed", () => {
  const callCount = workflow.match(
    /node \.\/scripts\/release\/create-updater-manifest\.mjs/gu,
  )?.length;
  const guardedCount = workflow.match(
    /node \.\/scripts\/release\/create-updater-manifest\.mjs[\s\S]*?\r?\n\s+--(?:output|verify) [^\r\n]+\r?\n\s+if \(\$LASTEXITCODE -ne 0\) \{ throw '[^']*manifest[^']*failed\.' \}/gu,
  )?.length;
  assert.equal(callCount, 4);
  assert.equal(guardedCount, callCount);
});
