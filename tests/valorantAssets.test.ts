import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { verifyValorantAssets } from "../scripts/assets/verify-valorant-assets.mjs";
import {
  bundledValorantAssetUrls,
  valorantAgentDisplayIconUrl,
  valorantMapListViewIconUrl,
} from "../src/lib/valorantAssets.ts";

const assetRefreshScript = readFileSync(
  new URL("../scripts/assets/fetch-valorant-assets.ps1", import.meta.url),
  "utf8",
);

test("map artwork resolves Chinese, English, and legacy aliases to local files", () => {
  const splitArtwork = "/valorant-assets/maps/d960549e-485c-e861-8d71-aa9d1aed12a2.png";

  assert.equal(valorantMapListViewIconUrl("霓虹町"), splitArtwork);
  assert.equal(valorantMapListViewIconUrl(" Split "), splitArtwork);
  assert.equal(valorantMapListViewIconUrl("双塔迷城"), splitArtwork);
  assert.equal(valorantMapListViewIconUrl("未知地图"), "");
});

test("agent artwork resolves current Chinese names and English aliases locally", () => {
  const chamberIcon =
    "/valorant-assets/agents/22697a3d-45bf-8dd7-4fec-84a9e28c69d7.png";

  assert.equal(valorantAgentDisplayIconUrl("尚勃勒"), chamberIcon);
  assert.equal(valorantAgentDisplayIconUrl("尚勒"), chamberIcon);
  assert.equal(valorantAgentDisplayIconUrl("Chamber"), chamberIcon);
  assert.match(valorantAgentDisplayIconUrl("迷核"), /^\/valorant-assets\/agents\//);
  assert.equal(valorantAgentDisplayIconUrl("未知英雄"), "");
});

test("every pinned artwork byte and PNG structure matches the manifest", () => {
  const report = verifyValorantAssets();

  assert.equal(report.assetCount, 42);
  assert.equal(report.totalBytes, 2_354_318);
  assert.equal(
    report.collectionFingerprint,
    "26c4c77a5a13d3ca1a84f4616b0cba1f251462882a0e86f9592d5fc8ef2e1c13",
  );
  assert.equal(bundledValorantAssetUrls.length, 42);

  for (const assetUrl of bundledValorantAssetUrls) {
    assert.doesNotMatch(assetUrl, /^https?:/);
  }
});

test("owner attestation remains pending review and narrower than public release", () => {
  const authorization = JSON.parse(
    readFileSync(
      new URL(
        "../release/approvals/game-content-rights.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    status: string;
    ownerAttestationReceived: boolean;
    sourceDocumentReviewed: boolean;
    legalReviewApproved: boolean;
    manualReviewRequired: boolean;
    repositoryOperationalAssumptionScopes: string[];
    notAttestedOrApprovedForRepositoryUse: string[];
  };

  assert.equal(
    authorization.status,
    "owner-attested-pending-source-evidence-review",
  );
  assert.equal(authorization.ownerAttestationReceived, true);
  assert.equal(authorization.sourceDocumentReviewed, false);
  assert.equal(authorization.legalReviewApproved, false);
  assert.equal(authorization.manualReviewRequired, true);
  for (const scope of [
    "public-source-repository",
    "in-app-display",
    "internal-controlled-testing",
    "windows-internal-test-build",
    "github-project-marketing",
  ]) {
    assert.equal(
      authorization.repositoryOperationalAssumptionScopes.includes(scope),
      true,
      scope,
    );
  }
  for (const scope of [
    "public-release-artifact-download",
    "public-windows-installer",
  ]) {
    assert.equal(
      authorization.notAttestedOrApprovedForRepositoryUse.includes(scope),
      true,
      scope,
    );
    assert.equal(
      authorization.repositoryOperationalAssumptionScopes.includes(scope),
      false,
      scope,
    );
  }
});

test("GitHub preview is synthetic, hash-pinned, and explicitly approved for publication", () => {
  const imageManifest = JSON.parse(
    readFileSync(
      new URL("../docs/images/manifest.json", import.meta.url),
      "utf8",
    ),
  ) as {
    schemaVersion: number;
    images: Array<{
      path: string;
      sha256: string;
      byteLength: number;
      width: number;
      height: number;
      dataClassification: string;
      gameAssetManifestSha256: string;
      gameAssetCollectionFingerprint: string;
      authorizationReference: string;
      publicationDecisionReference: string;
      operationalAssumptionScope: string;
      publicationApproved: boolean;
      manualScopeReviewRequired: boolean;
      renderedGameAssets: string[];
      renderingOperations: string[];
    }>;
  };
  const assetManifestBytes = readFileSync(
    new URL("../src/data/valorantAssets.json", import.meta.url),
  );
  const assetManifest = JSON.parse(assetManifestBytes.toString("utf8")) as {
    collectionFingerprint: string;
    authorizationReference: string;
    agents: Array<{ relativePath: string }>;
    maps: Array<{ relativePath: string }>;
  };
  const authorization = JSON.parse(
    readFileSync(
      new URL(
        "../release/approvals/game-content-rights.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    manualReviewRequired: boolean;
    repositoryOperationalAssumptionScopes: string[];
  };
  const channelAuthorization = JSON.parse(
    readFileSync(
      new URL(
        "../release/approvals/community-beta-v0.1.0-game-content-scope.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    status: string;
    distributionScopes: string[];
  };
  const releaseDecision = JSON.parse(
    readFileSync(
      new URL(
        "../release/approvals/community-beta-v0.1.0.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    releaseOwnerConfirmations: {
      gameContentScreenshotMayBePublishedInRepositoryReadme: boolean;
    };
  };

  assert.equal(imageManifest.schemaVersion, 2);
  assert.equal(imageManifest.images.length, 1);
  const image = imageManifest.images[0];
  const imageBytes = readFileSync(new URL(`../${image.path}`, import.meta.url));
  assert.equal(imageBytes.length, image.byteLength);
  assert.equal(
    createHash("sha256").update(imageBytes).digest("hex"),
    image.sha256,
  );
  assert.deepEqual(
    [...imageBytes.subarray(0, 8)],
    [137, 80, 78, 71, 13, 10, 26, 10],
  );
  assert.equal(imageBytes.readUInt32BE(16), image.width);
  assert.equal(imageBytes.readUInt32BE(20), image.height);
  assert.equal(image.dataClassification, "repository-synthetic-fixtures-only");
  assert.equal(
    image.gameAssetManifestSha256,
    createHash("sha256").update(assetManifestBytes).digest("hex"),
  );
  assert.equal(
    image.gameAssetCollectionFingerprint,
    assetManifest.collectionFingerprint,
  );
  assert.equal(
    image.authorizationReference,
    "release/approvals/community-beta-v0.1.0-game-content-scope.json",
  );
  assert.equal(
    image.publicationDecisionReference,
    "release/approvals/community-beta-v0.1.0.json",
  );
  assert.equal(
    authorization.repositoryOperationalAssumptionScopes.includes(
      image.operationalAssumptionScope,
    ),
    true,
  );
  assert.equal(authorization.manualReviewRequired, true);
  assert.equal(
    channelAuthorization.status,
    "approved-by-release-owner-for-community-beta",
  );
  assert.equal(
    channelAuthorization.distributionScopes.includes(
      image.operationalAssumptionScope,
    ),
    true,
  );
  assert.equal(
    releaseDecision.releaseOwnerConfirmations
      .gameContentScreenshotMayBePublishedInRepositoryReadme,
    true,
  );
  assert.equal(image.publicationApproved, true);
  assert.equal(image.manualScopeReviewRequired, false);
  assert.deepEqual(image.renderingOperations, [
    "display-scaling",
    "css-cropping-and-masking",
    "interface-composition",
  ]);

  const declaredPaths = new Set(
    [...assetManifest.agents, ...assetManifest.maps].map(
      ({ relativePath }) => relativePath,
    ),
  );
  assert.equal(image.renderedGameAssets.length, 4);
  for (const relativePath of image.renderedGameAssets) {
    assert.equal(declaredPaths.has(relativePath), true, relativePath);
  }
});

test("artwork verifier fails closed on scope, extra-file, and byte changes", () => {
  const temporaryRoot = mkdtempSync(join(tmpdir(), "valoframe-artwork-"));
  try {
    mkdirSync(join(temporaryRoot, "src", "data"), { recursive: true });
    mkdirSync(join(temporaryRoot, "release", "approvals"), {
      recursive: true,
    });
    cpSync(
      new URL("../public/valorant-assets", import.meta.url),
      join(temporaryRoot, "public", "valorant-assets"),
      { recursive: true },
    );
    copyFileSync(
      new URL("../src/data/valorantAssets.json", import.meta.url),
      join(temporaryRoot, "src", "data", "valorantAssets.json"),
    );
    const authorizationPath = join(
      temporaryRoot,
      "release",
      "approvals",
      "game-content-rights.json",
    );
    copyFileSync(
      new URL(
        "../release/approvals/game-content-rights.json",
        import.meta.url,
      ),
      authorizationPath,
    );

    const authorization = JSON.parse(
      readFileSync(authorizationPath, "utf8"),
    ) as { repositoryOperationalAssumptionScopes: string[] };
    authorization.repositoryOperationalAssumptionScopes.pop();
    writeFileSync(
      authorizationPath,
      `${JSON.stringify(authorization, null, 2)}\n`,
      "utf8",
    );
    assert.throws(
      () =>
        verifyValorantAssets({
          repositoryRoot: temporaryRoot,
          metadataOnly: true,
        }),
      /repositoryOperationalAssumptionScopes must match the recorded scope set/,
    );

    copyFileSync(
      new URL(
        "../release/approvals/game-content-rights.json",
        import.meta.url,
      ),
      authorizationPath,
    );
    const extraPath = join(
      temporaryRoot,
      "public",
      "valorant-assets",
      "undeclared.svg",
    );
    writeFileSync(extraPath, "<svg/>", "utf8");
    assert.throws(
      () => verifyValorantAssets({ repositoryRoot: temporaryRoot }),
      /asset tree contains undeclared files/,
    );
    rmSync(extraPath);

    const changedPath = join(
      temporaryRoot,
      "public",
      "valorant-assets",
      "agents",
      "e370fa57-4757-3604-3648-499e1f642d3f.png",
    );
    const changedBytes = readFileSync(changedPath);
    changedBytes[changedBytes.length - 13] ^= 1;
    writeFileSync(changedPath, changedBytes);
    assert.throws(
      () => verifyValorantAssets({ repositoryRoot: temporaryRoot }),
      /asset SHA-256 mismatch/,
    );
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("asset refresh validates metadata and a complete staging set before install", () => {
  const metadataGate = assetRefreshScript.indexOf("--metadata-only");
  const firstDownload = assetRefreshScript.indexOf("Receive-PinnedAsset `");
  const stagingGate = assetRefreshScript.indexOf(
    "The complete staged VALORANT artwork set failed verification",
  );
  const firstInstall = assetRefreshScript.indexOf(
    "-Description 'installed asset'",
  );

  assert.ok(metadataGate >= 0 && metadataGate < firstDownload);
  assert.ok(stagingGate >= 0 && stagingGate < firstInstall);
  assert.match(assetRefreshScript, /AllowAutoRedirect = \$false/);
  assert.match(assetRefreshScript, /ResponseHeadersRead/);
  assert.match(assetRefreshScript, /-not \$Uri\.IsDefaultPort/);
  assert.match(assetRefreshScript, /\$mediaType -ine 'image\/png'/);
  assert.match(assetRefreshScript, /\$total \+ \$read -gt \$ExpectedBytes/);
  assert.match(assetRefreshScript, /CancellationTokenSource/);
  assert.match(assetRefreshScript, /ReadAsync/);
  assert.match(assetRefreshScript, /Assert-NoReparseAncestors/);
  assert.match(assetRefreshScript, /reparse-point root/);
  assert.match(
    assetRefreshScript,
    /Remove-Item -LiteralPath \$stagingFull -Recurse -Force/,
  );
});
