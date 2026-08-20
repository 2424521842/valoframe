import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function read(path: string): string {
  return readFileSync(resolve(repositoryRoot, path), "utf8");
}

function readJson(path: string): Record<string, any> {
  return JSON.parse(read(path)) as Record<string, any>;
}

const workflow = read(".github/workflows/community-beta.yml");
const communityBetaDocumentation = read("docs/COMMUNITY_BETA.md");
const bundleGate = read("scripts/release/check-bundle.ps1");
const stageFfmpeg = read("scripts/release/stage-community-beta-ffmpeg.ps1");
const packageAssets = read("scripts/release/package-community-beta-assets.ps1");
const complianceGenerator = read(
  "scripts/release/generate-compliance-evidence.mjs",
);
const packageJson = readJson("package.json");
const decision = readJson(
  "release/approvals/community-beta-v0.1.0.json",
);

test("community beta workflow can only create an acknowledged prerelease", () => {
  assert.match(workflow, /workflow_dispatch:/u);
  assert.doesNotMatch(workflow, /\n\s+(?:push|pull_request|schedule):/u);
  assert.match(workflow, /approved_source_commit:/u);
  for (const mirrorInput of [
    "mirror_url",
    "mirror_password",
    "mirror_file_name",
    "mirror_sha256",
  ]) {
    assert.match(workflow, new RegExp(`${mirrorInput}:`, "u"));
  }
  assert.match(
    workflow,
    /expected_confirmation="UNSIGNED-COMMUNITY-BETA \$RELEASE_TAG \$APPROVED_SOURCE_COMMIT"/u,
  );
  assert.match(
    workflow,
    /test "\$APPROVED_SOURCE_COMMIT" = "\$GITHUB_SHA"/u,
  );
  assert.match(
    workflow,
    /test "\$GITHUB_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/u,
  );
  assert.match(workflow, /--prerelease/u);
  assert.match(workflow, /--latest=false/u);
  assert.match(workflow, /Refusing to overwrite existing release/u);
  assert.match(workflow, /HTTP 404\(\[\^0-9\]\|\$\)/u);
  assert.match(workflow, /Existing tag .* points to .* expected/u);
  assert.match(workflow, /Reusing exact retry tag/u);
  assert.match(workflow, /--verify-tag/u);
  assert.match(
    workflow,
    /--title "瓦刻（VALOFRAME）\$RELEASE_TAG · Community Beta"/u,
  );
  assert.doesNotMatch(workflow, /gh release create[\s\S]*?--target "\$GITHUB_SHA"/u);
  assert.match(workflow, /environment: community-beta-publish/u);
  assert.match(
    workflow,
    /publish:[\s\S]*?permissions:\s*\n\s+contents: write/u,
  );
  assert.match(workflow, /permissions:\s*\n\s+contents: read/u);
});

test("community beta artifacts survive failed-job and full-run retries", () => {
  assert.match(
    workflow,
    /name: community-beta-ffmpeg-candidate-\$\{\{ github\.run_id \}\}/u,
  );
  assert.match(
    workflow,
    /name: community-beta-release-assets-\$\{\{ github\.run_id \}\}/u,
  );
  assert.doesNotMatch(
    workflow,
    /name: community-beta-(?:ffmpeg-candidate|release-assets)-[^\n]*run_attempt/u,
  );
  assert.equal(workflow.match(/\n\s+overwrite: true/gu)?.length, 2);
});

test("community beta workflow keeps the updater runtime dormant and is visibly labeled", () => {
  assert.match(workflow, /release:bundle:windows:community-beta/u);
  assert.match(
    String(packageJson.scripts["release:bundle:windows:community-beta"]),
    /--no-sign/u,
  );
  assert.doesNotMatch(
    workflow,
    /TAURI_SIGNING_PRIVATE_KEY(?:_PASSWORD)?:\s*\$\{\{/u,
  );
  assert.match(workflow, /VALOFRAME_UPDATER_PUBLIC_KEY/u);
  assert.match(workflow, /Community Beta must not receive \$name/u);
  assert.doesNotMatch(workflow, /certificateThumbprint/u);
  assert.match(workflow, /NotSigned/u);
  assert.match(
    workflow,
    /UNSIGNED COMMUNITY BETA — NOT A FORMAL PUBLIC RELEASE/u,
  );
  assert.match(workflow, /-name 'latest\.json'/u);
  assert.match(workflow, /must not produce latest\.json or updater signatures/u);
  assert.doesNotMatch(workflow, /createUpdaterArtifacts:\s*true/u);
});

test("release notes lead users to the installer and explain technical attachments", () => {
  for (const marker of [
    "## 下载",
    "GitHub（推荐）",
    "普通用户只需下载本节列出的",
    "Source code",
    "SmartScreen",
    "更多信息",
    "主要功能",
    "技术合规附件",
  ]) {
    assert.match(packageAssets, new RegExp(marker, "u"));
  }
  assert.match(
    packageAssets,
    /releases\/download\/\$ReleaseTag\/\$installerOutputName/u,
  );
  assert.match(packageAssets, /Mirror metadata must be entirely empty or fully specified/u);
  assert.match(packageAssets, /MirrorSha256 -cmatch '\^\[0-9a-f\]\{64\}\$'/u);
});

test("minimal FFmpeg binary, source, licenses, and hashes travel together", () => {
  const releaseImplementation = `${workflow}\n${stageFfmpeg}\n${packageAssets}`;
  for (const marker of [
    "build-minimal-ffmpeg.sh",
    "verify-minimal-ffmpeg-candidate.ps1",
    "package-minimal-ffmpeg-community-beta.mjs",
    "stage-community-beta-ffmpeg.ps1",
    "valoframe-ffmpeg-minimal-windows-x64.zip",
    "ffmpeg-corresponding-source.tar.xz",
    "community-beta-compliance.zip",
    "SHA256SUMS.txt",
  ]) {
    assert.match(
      releaseImplementation,
      new RegExp(marker.replaceAll(".", "\\."), "u"),
    );
  }
  assert.match(stageFfmpeg, /externalLibraries = @\(\)/u);
  assert.match(workflow, /-BinaryArchiveOutputDirectory \$env:FFMPEG_ARCHIVE_OUTPUT_DIR/u);
  assert.match(workflow, /-FfmpegArchivePath \$env:BETA_FFMPEG_ARCHIVE/u);
  assert.match(
    stageFfmpeg,
    /Binary archive output directory must remain outside the hash-bound technical package root/u,
  );
  assert.match(
    packageAssets,
    /Technical FFmpeg package checksum coverage is incomplete/u,
  );
  assert.match(stageFfmpeg, /ffmpegExternalLibraryAuditComplete = \$true/u);
  assert.match(stageFfmpeg, /thirdPartyLicenseAuditComplete = \$false/u);
  assert.match(
    stageFfmpeg,
    /toolchainRuntimeLicenseReviewStatus = 'pending-for-strict-public-release'/u,
  );
  assert.match(stageFfmpeg, /ijgAttributionRequired = \$true/u);
  assert.match(stageFfmpeg, /ijgAttributionIncluded = \$true/u);
  assert.match(
    stageFfmpeg,
    /patentReviewStatus = 'pending-for-strict-public-release'/u,
  );
  assert.match(stageFfmpeg, /legalApprovalReference = \$null/u);
  assert.match(stageFfmpeg, /community-beta-source-bundled-formal-review-pending/u);
  assert.match(packageAssets, /Get-AuthenticodeSignature/u);
  assert.match(packageAssets, /correspondingSourceBundle/u);
  assert.match(packageAssets, /SHA256SUMS\.txt/u);
  assert.match(
    packageAssets,
    /FFmpeg manifest is not bound to the approved application source commit/u,
  );
  assert.match(
    packageAssets,
    /Startup smoke executable is not hash-bound to the bundle-gate main payload/u,
  );
});

test("Community Beta has its own gate and cannot weaken strict public release", () => {
  assert.match(bundleGate, /AllowUnsignedCommunityBeta/u);
  assert.match(
    bundleGate,
    /AllowUnsignedInternalRc and -AllowUnsignedCommunityBeta are mutually exclusive/u,
  );
  assert.match(bundleGate, /unsigned-community-beta/u);
  assert.match(bundleGate, /strictPublicReleaseApproved/u);
  assert.match(bundleGate, /Community Beta FFmpeg corresponding-source archive/u);
  assert.match(workflow, /public-release-preflight\.ps1 -ExpectBlocked/u);
  assert.match(workflow, /strict-public-bundle-block\.json/u);
  assert.match(
    workflow,
    /Strict public bundle gate did not fail for the expected beta-only FFmpeg state/u,
  );
  assert.equal(decision.strictPublicReleaseApproval, false);
  assert.equal(
    decision.releaseOwnerConfirmations.ffmpegMinimalBuildMayBeDistributedInThisChannel,
    true,
  );
  assert.equal(
    decision.releaseOwnerConfirmations.codecPatentReviewDeferredToStrictRelease,
    true,
  );
});

test("compliance generator supports an isolated beta manifest without changing defaults", () => {
  assert.match(complianceGenerator, /--release-profile/u);
  assert.match(complianceGenerator, /--ffmpeg-manifest/u);
  assert.match(
    complianceGenerator,
    /options\.releaseProfile \?\? "public"/u,
  );
  assert.match(
    complianceGenerator,
    /options\.ffmpegManifest \?\? "third_party\/ffmpeg\/windows-x64\.json"/u,
  );
  assert.match(workflow, /--release-profile community-beta/u);
  assert.match(
    workflow,
    /--ffmpeg-manifest \.tmp\/community-beta\/ffmpeg-windows-x64\.json/u,
  );
  assert.match(
    workflow,
    /--output \$env:BETA_COMPLIANCE_DIR/u,
  );
  assert.doesNotMatch(
    workflow,
    /--output src-tauri\/resources\/licenses\/third-party/u,
  );
  assert.match(
    workflow,
    /vhm-community-beta-third-party-compliance/u,
  );
  assert.match(workflow, /Third-party compliance destination escaped/u);
  assert.match(workflow, /if \(Test-Path -LiteralPath \$destination\)/u);
  assert.match(workflow, /Remove-Item -LiteralPath \$destination -Recurse -Force/u);
  assert.match(workflow, /Move-Item -LiteralPath \$actualSource -Destination \$destination/u);
});

test("startup smoke consumes only the exact bundle-gate payload", () => {
  for (const marker of [
    "verifiedPayloadOutputVerification.entriesMatchedReport",
    "verifiedPayloadOutputVerification.fileCount",
    "verifiedPayloadOutputDirectory",
    "System.Collections.Generic.HashSet[string]",
    "Verified Community Beta startup payload hash mismatch",
    "Verified Community Beta startup payload files do not exactly match",
    "normalization.rawEmbeddedSha256",
    "-ExpectedExecutableSha256 $expectedMainHash",
    "smoke.database.schemaVersion -ne 18",
    "fresh schema-v18 database",
    "windows-release-smoke.ps1 exited with code",
    "singleInstance.sharedLaunchConfiguration.environmentOverrides.WEBVIEW2_USER_DATA_FOLDER",
    "smoke.runtime.webView2UserDataPath",
  ]) {
    assert.match(
      workflow,
      new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&"), "u"),
    );
  }
});

test("publishing documentation requires a solo-maintainer-compatible environment approval", () => {
  assert.match(communityBetaDocumentation, /approved_source_commit/u);
  assert.match(
    communityBetaDocumentation,
    /UNSIGNED-COMMUNITY-BETA v0\.1\.0-beta\.1 <完整 SHA>/u,
  );
  assert.match(communityBetaDocumentation, /required reviewer/iu);
  assert.match(communityBetaDocumentation, /Prevent self-review/iu);
  assert.match(communityBetaDocumentation, /个人仓库可以把自己设为 reviewer/u);
  assert.match(communityBetaDocumentation, /deployment branches/iu);
  assert.match(communityBetaDocumentation, /默认分支/u);
  assert.match(
    communityBetaDocumentation,
    /required reviewer 和默认分支 deployment rule 未配置时，不得运行/u,
  );
});
