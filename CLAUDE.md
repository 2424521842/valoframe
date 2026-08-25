# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

VALOFRAME (瓦刻) is a Windows-only Tauri 2 desktop app that indexes local VALORANT (无畏契约) highlights into a searchable, previewable library grouped by account and match. Rust does all filesystem/database work; React 19 + TypeScript renders the UI. It does not record gameplay, read game memory, or upload anything automatically. Network egress is limited to: the user-consented issue-feedback upload (opt-in, sanitized diagnostics — currently save-to-file only, since the endpoint configuration is hidden in the UI; contract in `docs/FEEDBACK_UPLOAD_SPEC.md`), app update checks, and the opt-in ad slot's creative download (off by default; contract in `docs/AD_INTEGRATION_SPEC.md`). The webview itself never reaches an external origin — CSP forbids it, and ad creatives are proxied through the `clip-media` protocol.

Authoritative docs (in Chinese): `docs/ARCHITECTURE.md` (module map, command contract, security boundaries), `docs/DATA_MODEL.md`, `docs/PRD.md`, `docs/GIT_WORKFLOW.md`, `docs/RELEASE.md`.

## Commands

Requires Node `>=24 <25`, npm `>=11 <12` (npm is the only frontend package manager — do not introduce others), Rust 1.96.1 MSVC (`rust-toolchain.toml`).

### Run

```powershell
npm run dev            # Browser-only UI preview at http://localhost:1420, mock data, no real backend
npm run tauri -- dev   # Desktop dev mode (real Rust backend + Vite on port 1420, strictPort)
.\start-dev.bat        # Desktop dev wrapper: CARGO_INCREMENTAL=0 + loads updater pubkey from gitignored release-secrets/
```

The UI switches on `isBrowserPreviewRuntime()` in `src/api/backend.ts` (`window.__TAURI_INTERNALS__` absent → mock `browserPreview*` implementations). Browser preview exercises UI only — no scanning, playback, dialogs, or export.

### Test

```powershell
npm test    # = npm run assets:verify + npm run test:node + npm run test:ui
```

Two distinct frontend test layers:

- `npm run test:node` — `tsx --test tests/*.test.ts`: pure-logic tests on Node's test runner (no DOM).
- `npm run test:ui` — `vitest run` over `tests/ui/**/*.test.tsx`: jsdom + Testing Library component/controller tests. `vitest.setup.ts` installs the shared DOM shims (matchMedia, ResizeObserver, pointer capture).
- `npm run assets:verify` — asserts local agent/map images match the pinned manifest byte-for-byte.

Single tests:

```powershell
npx tsx --test tests/maps.test.ts
npx vitest run tests/ui/MatchLibrary.test.tsx
cargo test --manifest-path src-tauri/Cargo.toml --locked <test-name-filter>
```

### Build & lint

```powershell
npm run build   # tsc && vite build && assets:verify on dist output
# Rust gates (CI runs these from src-tauri/; cargo must be scoped there or via --manifest-path)
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
```

CI (`.github/workflows/ci.yml`) runs exactly these: frontend on ubuntu (`npm ci && npm test && npm run build`), Rust on windows-latest with the pinned toolchain.

## Architecture

### Frontend (`src/`)

- Six workspaces lazy-loaded from `App.tsx` (`Library`, `Preview`, `Review`, `Scan`, `TagManagement`, `Settings`), plus the cinematic sidebar shell.
- **Controller pattern**: each workspace's data access goes through a controller hook (`src/hooks/use*Controller.ts`) → single API layer `src/api/backend.ts` (`invoke`/`listen` wrappers) → Tauri commands. Stateful page logic (pagination with generation isolation, LRU detail cache, scan-event dedup, updater state machine) lives in these hooks.
- `src/lib/` is pure, well-tested logic (filters, grouping, dates, video types, tags).
- `src/data/valorantAssets.json` is the pinned game-asset manifest; `mockData.ts` feeds the browser preview.

### Rust backend (`src-tauri/`)

- `lib.rs` bootstraps: single-instance guard → DB migration → settle orphaned scans / restore delete intents → thumbnail worker → register commands. `db.rs` is the facade over per-entity repositories (`src-tauri/src/db/repositories/`) with plain-DTO models in `db/models.rs`.
- Scanning: `scan_coordinator.rs` (single global job, cancel, terminal states) → `scanner.rs` / `scanner/` (enumerate, metadata ingest, missing detection, batch commits). Source types: `aclos-structured` (wonderfulVideos dirs + official metadata) and `recursive-mp4` (NVIDIA/Tracker/generic plain .mp4). `drive_discovery.rs` does whole-PC fixed-drive discovery.
- Metadata ingestion (priority order, per field): **WonderfulDb clip record** (in-memory AES-256-CBC decrypt, `wonderful_db.rs`) > **video export JSON** (`metadata.rs`) > **highlight.log match fields incl. gzip payloads** (`highlight_log_parser.rs`) > **LevelDB battle summary** (`leveldb_reader.rs`) > filename inference.
- Thumbnails: persistent SQLite queue, single worker in `thumbnail.rs`. FFmpeg resolves ONLY from app resources `bin/ffmpeg(.exe)` or env `VHM_FFMPEG_PATH` — never PATH, never shell; missing FFmpeg degrades to placeholder covers. Do not add un-audited FFmpeg binaries to the repo.
- Media: `clip-media` custom protocol (`commands/media_protocol.rs`) streams by clip ID; large files capped at 1 MiB segments.
- Updater: `app_updates.rs` — fixed endpoint, embedded pubkey, gated state machine; Community Beta builds without pubkey refuse update checks.

### Database

SQLite via rusqlite, schema v21, file `highlight-index.sqlite3`. WAL + `synchronous=FULL`. Migrations run only at startup in one IMMEDIATE transaction with verified pre-migration backups; read/write connections never initialize schema. `Permanent delete` requires the file to be `trashed`, writes an immutable intent, then verifies Windows file identity (volume serial + file index) on the same handle before deleting.

### Core invariants (enforced by code + tests)

- Source files are read-only for scan/preview/organize; only explicit permanent-delete touches originals. User state (favorites, tags, notes, trash) lives only in SQLite.
- File statuses: `available` / `missing` / `trashed`. `missing` is only set when a source was fully enumerable, not cancelled.
- Cross-page grouping keyed on stable account + match key; facet queries aggregate the whole index, never the loaded page.
- CSP (in `src-tauri/tauri.conf.json`) is strict in production; only dev CSP allows eval/inline. Window capability exposes only event listen + dialog.

## Conventions

- **Git** (`docs/GIT_WORKFLOW.md`): conventional commits (`feat:`, `fix:`, …), branch prefixes `feature/ fix/ docs/ chore/ codex/`, work off `main`. `package-lock.json` AND `src-tauri/Cargo.lock` are committed; dependency bumps are their own commit.
- **Privacy**: never commit real game logs, WonderfulDb files, player names/OpenIDs, local paths, or databases. Test samples go in `tests/fixtures/` and must be sanitized. `.codex/`, `.claude/`, `.agents/`, `.superpowers/`, `.worktrees/`, `release-secrets/`, `.tmp*` are personal tool state — never commit.
- **Tests**: new pure logic goes in `tests/*.test.ts` (node runner); anything touching React/DOM goes in `tests/ui/*.test.tsx` (vitest + jsdom). Rust tests live in `src-tauri/tests/` (note: `real_scan.rs` uses env-supplied `VHM_REAL_SCAN_*` fixtures and is not self-contained).
- **Release** (Windows NSIS, Chinese product): `npm run release:bundle:windows:internal` builds unsigned bundles; the full gated release pipeline (FFmpeg evidence, compliance report, SHA-256 re-verification) is documented in `docs/RELEASE.md` with workflow files in `.github/workflows/`.
