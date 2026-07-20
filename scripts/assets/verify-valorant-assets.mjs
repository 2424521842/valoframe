import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const SCRIPT_REPOSITORY_ROOT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const REQUIRED_PUBLIC_STATEMENTS = [
  "not-official",
  "not-affiliated",
  "not-sponsored",
  "not-endorsed",
  "game-content-not-covered-by-mit",
];
const REPOSITORY_OPERATIONAL_ASSUMPTION_SCOPES = [
  "public-source-repository",
  "in-app-display",
  "internal-controlled-testing",
  "windows-internal-test-build",
  "github-project-marketing",
];
const NOT_ATTESTED_OR_APPROVED_SCOPES = [
  "public-release-artifact-download",
  "public-windows-installer",
  "commercial-distribution",
  "third-party-reuse",
  "official-affiliation",
  "official-sponsorship",
  "official-endorsement",
];
const MANUAL_REVIEW_REQUIREMENTS = [
  "source-authorization-document-or-verifiable-evidence-reference",
  "rights-holder-and-authority-chain",
  "licensee-legal-identity",
  "effective-date-expiration-and-revocation",
  "territory-and-language",
  "repository-source-and-github-marketing",
  "in-app-display-and-rendering-operations",
  "internal-test-build-and-participant-scope",
  "attribution-and-disclaimer-obligations",
];
const CURRENT_UI_RENDERING_OPERATIONS = [
  "display-scaling",
  "css-cropping-and-masking",
  "interface-composition",
];
const CRC_TABLE = buildCrcTable();

export function verifyValorantAssets(options = {}) {
  const repositoryRoot = realpathSync(
    resolve(options.repositoryRoot ?? SCRIPT_REPOSITORY_ROOT),
  );
  const manifestPath = resolveInside(
    repositoryRoot,
    options.manifest ?? "src/data/valorantAssets.json",
    "asset manifest",
  );
  const manifestBytes = readRegularFile(manifestPath, "asset manifest");
  const manifest = parseJson(manifestBytes, "asset manifest");

  assert(manifest.schemaVersion === 2, "asset manifest schemaVersion must be 2");
  assert(manifest.sourceService === "https://valorant-api.com/", "unexpected source service");
  assert(manifest.sourceBase === "https://media.valorant-api.com", "unexpected source base");
  assertDate(manifest.retrievedAt, "manifest retrievedAt");
  assertSafeRelativePath(manifest.assetRoot, "manifest assetRoot");
  assertSafeRelativePath(
    manifest.authorizationReference,
    "manifest authorizationReference",
  );
  assert(
    manifest.collectionFingerprintAlgorithm ===
      "sha256(sorted(relativePath + TAB + byteLength + TAB + sha256 + LF))",
    "unexpected collection fingerprint algorithm",
  );

  const entries = [];
  const uuidSet = new Set();
  const nameIndex = new Map();
  for (const [category, expectedCount] of [
    ["agents", 29],
    ["maps", 13],
  ]) {
    const categoryEntries = manifest[category];
    assert(Array.isArray(categoryEntries), `manifest ${category} must be an array`);
    assert(
      categoryEntries.length === expectedCount,
      `manifest ${category} must contain ${expectedCount} entries`,
    );
    for (const entry of categoryEntries) {
      verifyEntryShape(entry, category, manifest);
      assert(!uuidSet.has(entry.uuid), `duplicate asset UUID: ${entry.uuid}`);
      uuidSet.add(entry.uuid);

      for (const candidate of [entry.displayName, ...entry.aliases]) {
        const normalized = normalizeAssetName(candidate);
        assert(normalized.length > 0, `empty normalized asset name for ${entry.uuid}`);
        const existing = nameIndex.get(normalized);
        assert(
          existing === undefined || existing === entry.uuid,
          `asset alias collision: ${candidate}`,
        );
        nameIndex.set(normalized, entry.uuid);
      }

      entries.push(entry);
    }
  }

  const fingerprintPayload = entries
    .map(
      ({ relativePath, byteLength, sha256: digest }) =>
        `${relativePath}\t${byteLength}\t${digest}\n`,
    )
    .sort()
    .join("");
  const collectionFingerprint = sha256(Buffer.from(fingerprintPayload, "utf8"));
  assert(
    collectionFingerprint === manifest.collectionFingerprint,
    "asset collection fingerprint mismatch",
  );

  const totalBytes = entries.reduce((sum, entry) => sum + entry.byteLength, 0);
  const authorizationPath = resolveInside(
    repositoryRoot,
    manifest.authorizationReference,
    "game-content authorization record",
  );
  const authorization = parseJson(
    readRegularFile(authorizationPath, "game-content authorization record"),
    "game-content authorization record",
  );
  verifyAuthorization({
    authorization,
    manifest,
    manifestPath,
    repositoryRoot,
    entryCount: entries.length,
    totalBytes,
    collectionFingerprint,
  });

  const baseReport = {
    manifestPath,
    authorizationPath,
    assetCount: entries.length,
    totalBytes,
    collectionFingerprint,
    manifestSha256: sha256(manifestBytes),
  };
  if (options.metadataOnly === true) {
    return Object.freeze({ ...baseReport, assetRoot: null });
  }

  const configuredAssetRoot =
    options.assetRoot === undefined
      ? manifest.assetRoot
      : options.assetRoot;
  const assetRoot = resolveInside(
    repositoryRoot,
    configuredAssetRoot,
    "asset root",
  );
  assertDirectory(assetRoot, "asset root");
  for (const category of ["agents", "maps"]) {
    assertDirectory(resolveInside(assetRoot, category, `${category} asset directory`), `${category} asset directory`);
  }

  for (const entry of entries) {
    const filePath = resolveInside(assetRoot, entry.relativePath, "asset file");
    const bytes = readRegularFile(filePath, `asset file ${entry.relativePath}`);
    assert(
      bytes.length === entry.byteLength,
      `asset byte length mismatch: ${entry.relativePath}`,
    );
    assert(
      sha256(bytes) === entry.sha256,
      `asset SHA-256 mismatch: ${entry.relativePath}`,
    );
    const png = inspectPng(bytes, entry.relativePath);
    assert(
      png.width === entry.width && png.height === entry.height,
      `asset dimensions mismatch: ${entry.relativePath}`,
    );
  }

  const declaredPaths = entries.map(({ relativePath }) => relativePath).sort();
  const actualFiles = listAssetFiles(assetRoot).sort();
  const actualPaths = actualFiles
    .filter((path) => path.toLowerCase().endsWith(".png"))
    .sort();
  assert(
    JSON.stringify(actualPaths) === JSON.stringify(declaredPaths),
    `asset file set differs from manifest; declared=${declaredPaths.length}, actual=${actualPaths.length}`,
  );
  const unexpectedFiles = actualFiles.filter(
    (path) => path !== "README.md" && !declaredPaths.includes(path),
  );
  assert(
    unexpectedFiles.length === 0,
    `asset tree contains undeclared files: ${unexpectedFiles.join(", ")}`,
  );

  return Object.freeze({
    ...baseReport,
    assetRoot,
  });
}

function verifyEntryShape(entry, category, manifest) {
  assert(entry && typeof entry === "object", `invalid ${category} manifest entry`);
  assert(
    typeof entry.displayName === "string" && entry.displayName.trim().length > 0,
    `invalid displayName in ${category}`,
  );
  assert(
    typeof entry.uuid === "string" && UUID_PATTERN.test(entry.uuid),
    `invalid UUID in ${category}`,
  );
  assert(
    Array.isArray(entry.aliases) &&
      entry.aliases.every(
        (alias) => typeof alias === "string" && alias.trim().length > 0,
      ),
    `invalid aliases for ${entry.uuid}`,
  );
  assert(
    new Set(entry.aliases).size === entry.aliases.length,
    `duplicate aliases for ${entry.uuid}`,
  );

  const leaf = category === "agents" ? "displayicon.png" : "listviewicon.png";
  const expectedRelativePath = `${category}/${entry.uuid}.png`;
  const expectedSourceUrl = `${manifest.sourceBase}/${category}/${entry.uuid}/${leaf}`;
  assert(
    entry.relativePath === expectedRelativePath,
    `unexpected relativePath for ${entry.uuid}`,
  );
  assert(entry.sourceUrl === expectedSourceUrl, `unexpected sourceUrl for ${entry.uuid}`);
  assert(entry.mimeType === "image/png", `unexpected MIME type for ${entry.uuid}`);
  assert(
    Number.isSafeInteger(entry.byteLength) && entry.byteLength > PNG_SIGNATURE.length,
    `invalid byteLength for ${entry.uuid}`,
  );
  assert(
    Number.isSafeInteger(entry.width) &&
      Number.isSafeInteger(entry.height) &&
      entry.width > 0 &&
      entry.height > 0,
    `invalid dimensions for ${entry.uuid}`,
  );
  assert(
    typeof entry.sha256 === "string" && SHA256_PATTERN.test(entry.sha256),
    `invalid SHA-256 for ${entry.uuid}`,
  );
}

function verifyAuthorization({
  authorization,
  manifest,
  manifestPath,
  repositoryRoot,
  entryCount,
  totalBytes,
  collectionFingerprint,
}) {
  assert(authorization.schemaVersion === 1, "authorization schemaVersion must be 1");
  assert(
    !Object.hasOwn(authorization, "approved"),
    "owner attestation must not be represented as legal approval",
  );
  assert(
    authorization.status === "owner-attested-pending-source-evidence-review",
    "unexpected game-content authorization status",
  );
  assert(
    authorization.ownerAttestationReceived === true,
    "repository owner attestation is missing",
  );
  assertDate(
    authorization.ownerStatementReceivedOn,
    "authorization ownerStatementReceivedOn",
  );
  assert(
    authorization.sourceDocumentReviewed === false,
    "source-document review must remain pending",
  );
  assert(
    authorization.legalReviewApproved === false,
    "legal review must remain pending",
  );
  assert(
    authorization.manualReviewRequired === true,
    "manual source-evidence review must be required",
  );
  assert(
    typeof authorization.evidenceReference === "string" &&
      authorization.evidenceReference.trim().length > 0,
    "authorization evidenceReference is missing",
  );
  assert(
    authorization.evidenceDocumentSha256 === null,
    "source authorization document hash must remain unrecorded until evidence is provided",
  );
  assert(
    authorization.rightsHolderIdentity === "not-provided" &&
      authorization.licensee === "not-provided",
    "unprovided legal-party identities must not be inferred",
  );
  assertExactStringSet(
    authorization.repositoryOperationalAssumptionScopes,
    REPOSITORY_OPERATIONAL_ASSUMPTION_SCOPES,
    "authorization repositoryOperationalAssumptionScopes",
  );
  assertExactStringSet(
    authorization.notAttestedOrApprovedForRepositoryUse,
    NOT_ATTESTED_OR_APPROVED_SCOPES,
    "authorization notAttestedOrApprovedForRepositoryUse",
  );
  assertExactStringSet(
    authorization.manualReviewRequirements,
    MANUAL_REVIEW_REQUIREMENTS,
    "authorization manualReviewRequirements",
  );
  const restrictions = authorization.repositoryOperationalRestrictions;
  assert(
    restrictions && typeof restrictions === "object",
    "authorization repositoryOperationalRestrictions is missing",
  );
  assert(
    restrictions.nonCommercialOnly === true &&
      restrictions.sourceAssetBytesMustRemainManifestExact === true &&
      restrictions.noSublicensing === true &&
      restrictions.noStandaloneDerivativeAssetFiles === true &&
      restrictions.publicationOrDistributionBeforeManualScopeReview === false,
    "authorization operational restrictions are incomplete",
  );
  assertExactStringSet(
    restrictions.renderingOperationsUsedByCurrentUi,
    CURRENT_UI_RENDERING_OPERATIONS,
    "authorization renderingOperationsUsedByCurrentUi",
  );
  assertExactStringSet(
    authorization.requiredPublicStatements,
    REQUIRED_PUBLIC_STATEMENTS,
    "authorization requiredPublicStatements",
  );

  const assetSet = authorization.assetSet;
  assert(assetSet && typeof assetSet === "object", "authorization assetSet is missing");
  assert(
    assetSet.manifest ===
      relative(repositoryRoot, manifestPath).split(sep).join("/"),
    "authorization manifest path mismatch",
  );
  assert(
    assetSet.manifestSha256 === sha256(readFileSync(manifestPath)),
    "authorization manifest SHA-256 mismatch",
  );
  assert(assetSet.assetRoot === manifest.assetRoot, "authorization asset root mismatch");
  assert(assetSet.assetCount === entryCount, "authorization asset count mismatch");
  assert(assetSet.totalBytes === totalBytes, "authorization total byte count mismatch");
  assert(
    assetSet.collectionFingerprint === collectionFingerprint &&
      assetSet.collectionFingerprint === manifest.collectionFingerprint,
    "authorization collection fingerprint mismatch",
  );
  assert(
    assetSet.sourceService === manifest.sourceService &&
      assetSet.retrievedAt === manifest.retrievedAt,
    "authorization source snapshot mismatch",
  );
  assert(
    assetSet.technicalVerificationStatus === "manifest-bytes-verified" &&
      assetSet.legalScopeStatus === "pending-manual-source-evidence-review",
    "authorization asset-set review status is invalid",
  );
}

function inspectPng(bytes, description) {
  assert(
    bytes.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE),
    `invalid PNG signature: ${description}`,
  );

  let offset = PNG_SIGNATURE.length;
  let width;
  let height;
  let sawIhdr = false;
  let sawIdat = false;
  let idatEnded = false;
  let sawIend = false;
  const idatParts = [];

  while (offset < bytes.length) {
    assert(offset + 12 <= bytes.length, `truncated PNG chunk: ${description}`);
    const length = bytes.readUInt32BE(offset);
    const typeStart = offset + 4;
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;
    const chunkEnd = dataEnd + 4;
    assert(chunkEnd <= bytes.length, `PNG chunk exceeds file: ${description}`);
    const type = bytes.toString("ascii", typeStart, dataStart);
    assert(/^[A-Za-z]{4}$/.test(type), `invalid PNG chunk type: ${description}`);
    assert(type[2] === type[2].toUpperCase(), `invalid PNG reserved chunk bit: ${description}`);
    const critical = type[0] === type[0].toUpperCase();
    assert(
      !critical || ["IHDR", "PLTE", "IDAT", "IEND"].includes(type),
      `unknown critical PNG chunk ${type}: ${description}`,
    );
    const expectedCrc = bytes.readUInt32BE(dataEnd);
    const actualCrc = crc32(bytes.subarray(typeStart, dataEnd));
    assert(expectedCrc === actualCrc, `PNG CRC mismatch in ${type}: ${description}`);

    if (!sawIhdr) {
      assert(type === "IHDR" && length === 13, `PNG must start with IHDR: ${description}`);
      sawIhdr = true;
      width = bytes.readUInt32BE(dataStart);
      height = bytes.readUInt32BE(dataStart + 4);
      assert(bytes[dataStart + 8] === 8, `PNG must be 8-bit: ${description}`);
      assert(bytes[dataStart + 9] === 6, `PNG must be RGBA: ${description}`);
      assert(bytes[dataStart + 10] === 0, `invalid PNG compression: ${description}`);
      assert(bytes[dataStart + 11] === 0, `invalid PNG filter method: ${description}`);
      assert(bytes[dataStart + 12] === 0, `PNG must be non-interlaced: ${description}`);
    } else if (type === "IHDR") {
      throw new Error(`duplicate PNG IHDR: ${description}`);
    } else if (type === "IDAT") {
      assert(!idatEnded, `non-contiguous PNG IDAT chunks: ${description}`);
      sawIdat = true;
      idatParts.push(bytes.subarray(dataStart, dataEnd));
    } else {
      if (sawIdat) idatEnded = true;
      if (type === "IEND") {
        assert(length === 0, `PNG IEND must be empty: ${description}`);
        assert(!sawIend, `duplicate PNG IEND: ${description}`);
        sawIend = true;
        assert(chunkEnd === bytes.length, `PNG has trailing bytes: ${description}`);
      }
    }
    offset = chunkEnd;
  }

  assert(sawIhdr && sawIdat && sawIend, `PNG is missing a required chunk: ${description}`);
  const rowLength = 1 + width * 4;
  const expectedInflatedLength = rowLength * height;
  const inflated = inflateSync(Buffer.concat(idatParts), {
    maxOutputLength: expectedInflatedLength + 1,
  });
  assert(
    inflated.length === expectedInflatedLength,
    `PNG decompressed size mismatch: ${description}`,
  );
  for (let row = 0; row < height; row += 1) {
    assert(
      inflated[row * rowLength] <= 4,
      `PNG has an invalid row filter: ${description}`,
    );
  }
  return { width, height };
}

function listAssetFiles(root) {
  const files = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolutePath = resolve(directory, entry.name);
      assert(!entry.isSymbolicLink(), `asset tree contains a symbolic link: ${absolutePath}`);
      if (entry.isDirectory()) {
        visit(absolutePath);
      } else if (entry.isFile()) {
        files.push(relative(root, absolutePath).split(sep).join("/"));
      }
    }
  };
  visit(root);
  return files;
}

function resolveInside(root, candidate, description) {
  assert(
    typeof candidate === "string" && candidate.length > 0,
    `${description} path is missing`,
  );
  const absolutePath = isAbsolute(candidate)
    ? resolve(candidate)
    : resolve(root, candidate);
  const relation = relative(root, absolutePath);
  assert(
    relation !== ".." &&
      !relation.startsWith(`..${sep}`) &&
      !isAbsolute(relation),
    `${description} escapes the repository root`,
  );
  let current = root;
  for (const segment of relation.split(sep).filter(Boolean)) {
    current = resolve(current, segment);
    assert(
      !lstatSync(current).isSymbolicLink(),
      `${description} contains a symbolic link or junction`,
    );
  }
  const canonicalPath = realpathSync(absolutePath);
  const canonicalRelation = relative(root, canonicalPath);
  assert(
    canonicalRelation !== ".." &&
      !canonicalRelation.startsWith(`..${sep}`) &&
      !isAbsolute(canonicalRelation),
    `${description} resolves outside the repository root`,
  );
  return absolutePath;
}

function assertSafeRelativePath(value, description) {
  assert(
    typeof value === "string" &&
      value.length > 0 &&
      !isAbsolute(value) &&
      !value.includes("\\") &&
      !value.includes("\0") &&
      value
        .split("/")
        .every((part) => part.length > 0 && part !== "." && part !== ".."),
    `${description} must be a safe repository-relative POSIX path`,
  );
}

function readRegularFile(path, description) {
  const item = lstatSync(path);
  assert(
    item.isFile() && !item.isSymbolicLink(),
    `${description} must be a regular file`,
  );
  return readFileSync(path);
}

function assertDirectory(path, description) {
  const item = lstatSync(path);
  assert(
    item.isDirectory() && !item.isSymbolicLink(),
    `${description} must be a regular directory`,
  );
}

function parseJson(bytes, description) {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${description} is not valid JSON: ${error.message}`);
  }
}

function assertDate(value, description) {
  assert(
    typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value),
    `${description} must be YYYY-MM-DD`,
  );
}

function assertExactStringSet(actual, expected, description) {
  assert(
    Array.isArray(actual) &&
      actual.every(
        (value) => typeof value === "string" && value.trim().length > 0,
      ) &&
      new Set(actual).size === actual.length &&
      actual.length === expected.length &&
      expected.every((value) => actual.includes(value)),
    `${description} must match the recorded scope set`,
  );
}

function normalizeAssetName(value) {
  return value.trim().toLocaleLowerCase("zh-CN").replace(/[\s_-]+/g, "");
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function buildCrcTable() {
  const table = new Uint32Array(256);
  for (let index = 0; index < table.length; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value & 1) === 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[index] = value >>> 0;
  }
  return table;
}

function crc32(bytes) {
  let value = 0xffffffff;
  for (const byte of bytes) {
    value = CRC_TABLE[(value ^ byte) & 0xff] ^ (value >>> 8);
  }
  return (value ^ 0xffffffff) >>> 0;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function parseCliArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--quiet") {
      options.quiet = true;
      continue;
    }
    if (argument === "--metadata-only") {
      options.metadataOnly = true;
      continue;
    }
    if (!["--repository-root", "--manifest", "--asset-root"].includes(argument)) {
      throw new Error(`unknown argument: ${argument}`);
    }
    const value = argv[index + 1];
    if (!value) throw new Error(`missing value for ${argument}`);
    index += 1;
    if (argument === "--repository-root") options.repositoryRoot = value;
    if (argument === "--manifest") options.manifest = value;
    if (argument === "--asset-root") options.assetRoot = value;
  }
  return options;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const options = parseCliArguments(process.argv.slice(2));
    const report = verifyValorantAssets(options);
    if (!options.quiet) {
      const subject = options.metadataOnly
        ? "Verified metadata for"
        : "Verified";
      console.log(
        `${subject} ${report.assetCount} pinned VALORANT artwork files ` +
          `(${report.totalBytes.toLocaleString("en-US")} bytes, ` +
          `${report.collectionFingerprint}).`,
      );
    }
  } catch (error) {
    console.error(`VALORANT artwork verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}
