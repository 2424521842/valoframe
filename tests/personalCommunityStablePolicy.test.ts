import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

type JsonObject = Record<string, any>;

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");
const readJson = (path: string): JsonObject => JSON.parse(read(path));

const decision = readJson(
  "release/approvals/personal-community-stable-v0.2.1.json",
);
const policy = readJson("release/personal-community-stable-policy.json");
const ffmpegContract = readJson(
  "third_party/ffmpeg/personal-community-stable-minimal-windows-x64.json",
);
const generator = read("scripts/release/generate-compliance-evidence.mjs");
const licensing = read("docs/LICENSING.md");
const notes = read("release/notes/v0.2.1.md");
const bundleGate = read("scripts/release/check-bundle.ps1");

test("v0.2.1 has a narrowly scoped repository-owner community decision", () => {
  assert.equal(decision.version, "0.2.1");
  assert.equal(decision.tag, "v0.2.1");
  assert.equal(decision.channel, "personal-community-stable");
  assert.equal(decision.decision, "approved-by-repository-release-owner");
  assert.equal(decision.distributionPurpose, "free-personal-community");
  assert.equal(decision.strictPublicReleaseApproval, false);
  assert.equal(decision.independentLegalReviewCompleted, false);
  assert.equal(
    decision.releaseOwnerConfirmations.tauriUpdaterSignatureRemainsRequired,
    true,
  );
  assert.equal(
    decision.distributionRequirements.ffmpegCorrespondingSourceMustAccompanyRelease,
    true,
  );
  assert.equal(decision.assetSet.assetCount, 42);
  assert.match(decision.assetSet.manifestSha256, /^[0-9a-f]{64}$/u);
  const assetManifestBytes = readFileSync(resolve(root, decision.assetSet.manifest));
  const assetManifest = JSON.parse(assetManifestBytes.toString("utf8"));
  assert.equal(
    createHash("sha256").update(assetManifestBytes).digest("hex"),
    decision.assetSet.manifestSha256,
  );
  assert.equal(
    assetManifest.agents.length + assetManifest.maps.length,
    decision.assetSet.assetCount,
  );
  assert.equal(
    assetManifest.collectionFingerprint,
    decision.assetSet.collectionFingerprint,
  );
  assert.equal(decision.assetSet.sourceAssetBytesMustRemainManifestExact, true);
  assert.match(bundleGate, /game asset manifest SHA-256 does not match the owner decision/u);
  assert.match(bundleGate, /game asset collection fingerprint does not match the owner decision/u);
  assert.ok(
    decision.nonAssertions.some((entry: string) =>
      entry.includes("does not waive third-party license obligations"),
    ),
  );
});

test("personal stable keeps technical distribution gates while deferring enterprise gates", () => {
  assert.equal(policy.releaseProfile, "personal-community-stable");
  for (const mandatory of [
    "tauriUpdaterSignature",
    "minimalSourceBuiltFfmpeg",
    "ffmpegCorrespondingSourceOnSameRelease",
    "ffmpegLicenseAndBuildEvidence",
    "thirdPartyLicenseTextCoverage",
    "mplSourceAvailability",
    "bundlePayloadVerification",
    "isolatedStartupSmoke",
    "draftAssetRedownloadVerification",
    "sha256ForEveryOtherAsset",
  ]) {
    assert.equal(policy.mandatory[mandatory], true, mandatory);
  }
  assert.equal(policy.mandatory.ffmpegExternalLibraries, 0);
  assert.ok(
    policy.futureHardeningNotBlockingThisProfile.includes(
      "authenticode-and-trusted-timestamp",
    ),
  );
  assert.ok(
    policy.futureHardeningNotBlockingThisProfile.includes(
      "independent-legal-review",
    ),
  );
});

test("personal stable packages only the pinned minimal LGPL FFmpeg", () => {
  assert.equal(
    ffmpegContract.status,
    "personal-community-stable-technical-packaging-contract",
  );
  assert.equal(ffmpegContract.channel, "personal-community-stable");
  assert.equal(
    ffmpegContract.sourceCommit,
    "ce3c09c101c83add623774d414a9f9498caf5c25",
  );
  assert.equal(
    ffmpegContract.output.sourceArchiveRelativePath,
    "release-assets/ffmpeg-corresponding-source.tar.xz",
  );
  assert.equal(
    ffmpegContract.complianceBoundary.ownerAuthorizedForThisChannel,
    true,
  );
  assert.equal(
    ffmpegContract.complianceBoundary.strictPublicReleaseApproved,
    false,
  );
});

test("compliance generator preserves strict truth while allowing the owner channel", () => {
  assert.match(generator, /personal-community-stable/u);
  assert.match(generator, /channelDistributionReady/u);
  assert.match(generator, /publicRedistributionReady: !personalCommunityStable/u);
  assert.match(generator, /validatePersonalCommunityStableFfmpeg/u);
  assert.match(generator, /--enable-gpl is forbidden/u);
  assert.match(generator, /external --enable-lib\* integrations are forbidden/u);
  assert.match(generator, /MPL_SOURCE_FORM_AVAILABLE/u);
  assert.match(generator, /635e1a19d02960588a00e189bd4bd5bdb150ec3d/u);
});

test("documentation describes intent without adding a non-commercial MIT restriction", () => {
  assert.match(licensing, /第一方 MIT 代码.*不会.*增加.*禁止商业使用/u);
  assert.match(notes, /非商业.*不会限制项目第一方 MIT 代码/u);
  assert.match(notes, /LGPL-3\.0-or-later/u);
  assert.match(notes, /未知发布者/u);
  assert.match(notes, /Tauri\/Minisign 签名/u);
});

test("the bound stable decision parameter survives until the channel check", () => {
  // PowerShell variable names are case-insensitive: an assignment to
  // $personalCommunityStableDecisionPath before the parameter check erases the
  // bound -PersonalCommunityStableDecisionPath value and fails every run.
  const checkIndex = bundleGate.indexOf(
    "IsNullOrWhiteSpace($PersonalCommunityStableDecisionPath)",
  );
  assert.ok(checkIndex > 0, "stable decision parameter check must exist");
  const assignIndex = bundleGate.indexOf(
    "$personalCommunityStableDecisionPath =",
  );
  assert.ok(
    assignIndex < 0 || assignIndex > checkIndex,
    "no assignment to the decision variable may precede the parameter check",
  );
});
