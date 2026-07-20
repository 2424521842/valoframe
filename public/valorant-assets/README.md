# Bundled VALORANT artwork

VALOFRAME bundles 29 agent display icons and 13 map list-view images so the match library can render without a runtime network dependency.

- Runtime/source manifest: `src/data/valorantAssets.json`
- Owner attestation and review state: `release/approvals/game-content-rights.json`
- Offline verifier: `scripts/assets/verify-valorant-assets.mjs`
- Retrieval/repair script: `scripts/assets/fetch-valorant-assets.ps1`
- Upstream asset service: `https://valorant-api.com/`
- Collection fingerprint: `26c4c77a5a13d3ca1a84f4616b0cba1f251462882a0e86f9592d5fc8ef2e1c13`
- Rights status: **owner-attested; source evidence and exact scopes not yet reviewed**

On 2026-07-20, the repository owner stated that authorization had been obtained. No source authorization document, verifiable external evidence identifier, rights-holder/licensee identity, territory, term, or exact permitted-use clauses were provided to repository automation. Public-source, in-app, controlled-test, internal Windows-build, and GitHub-marketing uses are therefore conservative repository operating assumptions—not verified legal facts—and publication or distribution must wait for manual comparison with the source authorization.

Repository automation verifies every asset's path, exact source URL, dimensions, byte length, PNG structure, and SHA-256; it cannot establish legal scope. Until manual review is recorded, do **not** publish these changes or a GitHub marketing image, create public Windows installers or Release downloads, use the files commercially, sublicense them, permit third-party reuse, or create standalone derived artwork. Run `npm run assets:verify` before local controlled use; `fetch-valorant-assets.ps1 -Refresh` is intentionally explicit and accepts only bytes pinned by the manifest.

These game-content assets are not licensed under VALOFRAME's MIT License. Recipients do not gain reuse rights merely by cloning the repository. VALORANT and related game content belong to their respective owners; inclusion does not imply official affiliation, sponsorship, or endorsement by Riot Games or Tencent.
