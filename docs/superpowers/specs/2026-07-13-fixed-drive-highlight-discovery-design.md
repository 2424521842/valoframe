# Fixed-Drive Highlight Discovery Design

**Date:** 2026-07-13

## Goal

Add an explicit “全电脑发现” action that searches the current Windows computer's fixed drives for standard ACLOS `wonderfulVideos*` source directories, validates them, and imports their clips through the existing read-only scanner. The feature must not depend on Everything, Windows Search indexing, a background indexing service, administrator privileges, or prior knowledge of the clip location.

## Scope

The feature searches local fixed drives only. It excludes removable drives, optical drives, network shares, mapped network drives, and paths reached through symbolic links, directory junctions, or other reparse points.

A directory is a discovery candidate when its name starts with `wonderfulVideos`, matched case-insensitively. A candidate is valid when at least one of its direct child directories contains a direct `.mp4` file, also matched case-insensitively. Loose MP4 files outside that structure are not discovered.

Discovery is user-triggered through a separate button. It does not run during application startup or ordinary “重新扫描”, and it does not create a persistent filesystem index.

## Non-Goals

- Do not integrate or redistribute Everything.
- Do not use the Windows Search index.
- Do not scan removable or network storage.
- Do not import loose MP4 files or guess whether arbitrary videos came from ACLOS.
- Do not modify, move, rename, or delete source files.
- Do not change the behavior or scope of default and custom directory scans.
- Do not add a background monitor or recurring full-drive search.

## Architecture

### Drive discovery module

Create a focused Rust module, `src-tauri/src/drive_discovery.rs`, separate from the existing large `scanner.rs` module. Its public responsibilities are:

1. Enumerate Windows drive roots and retain only roots reported as `DRIVE_FIXED`.
2. Traverse supplied roots iteratively rather than recursively.
3. Identify and validate `wonderfulVideos*` candidates.
4. Return deduplicated scan roots, traversal metrics, and aggregated warnings.
5. Report discovery progress without depending on Tauri or the database.

The production drive enumerator uses Windows APIs solely to classify logical drive types. Directory discovery itself uses Rust filesystem APIs and does not invoke Windows Search.

The core traversal accepts an explicit list of roots. This keeps the algorithm testable with temporary directory fixtures and prevents automated tests from scanning the developer's real disks.

### Traversal rules

Traversal uses an explicit work queue. For each queued directory it reads immediate entries, counts the directory as visited, and queues eligible child directories.

An eligible child directory:

- is not a symbolic link;
- does not have the Windows reparse-point attribute;
- is readable; and
- has not already been accepted as a `wonderfulVideos*` candidate.

When a child directory name starts with `wonderfulVideos`, discovery validates only that candidate's direct child directories and their direct files. If one of those group directories contains an `.mp4`, the candidate's parent directory becomes a scan root. Discovery does not descend further into the accepted candidate. If validation fails, traversal also does not reinterpret the candidate's descendants as unrelated scan roots.

All returned scan roots are normalized with the same path normalization used by the database scanner and deduplicated case-insensitively. Multiple valid candidates with one parent therefore produce one scan root.

### Scan orchestration

Add a Tauri command dedicated to the complete operation:

1. Emit an indeterminate discovery progress event.
2. Enumerate fixed drives.
3. Discover and validate candidates.
4. If no valid candidates exist, return a successful empty result without running an import scan.
5. Pass the discovered parent roots to the existing multi-root scan orchestration.
6. Merge discovery metrics and warnings with the final result.
7. Return one response to the frontend and let it refresh the library once.

The existing default scan remains based on the default ACLOS root and `videocut` log hints. The existing custom scan remains based on the user-supplied root.

## Data Contracts and Progress

The command returns a discovery-and-scan result containing:

- discovered fixed-drive count;
- visited directory count;
- validated `wonderfulVideos*` count;
- deduplicated scan-root count;
- skipped unreadable directory count;
- a bounded list of representative discovery warnings; and
- the existing `ScanSummary` for the import phase, or an empty scan summary when no candidates were found.

Discovery progress uses the existing scan progress event channel with a distinct discovery phase. Because the total number of directories is not known in advance, discovery progress is indeterminate. Its message includes the current drive, visited directory count, and valid candidate count. Once discovery finishes, the same command emits the existing source and clip scan phases.

## User Interface

Add a secondary “全电脑发现” button to the sidebar scan card. It remains visually and behaviorally separate from “重新扫描” and “添加目录”.

On activation:

- all scan entry points become disabled to prevent concurrent SQLite writes;
- the scan card displays the discovery phase and live metrics;
- the backend automatically imports validated results without a confirmation step;
- the frontend refreshes the clip list once after the command completes; and
- the final activity message reports discovered source directories and imported videos.

No valid candidate is a normal result. The UI displays “未发现标准无畏时刻素材” and leaves the existing library unchanged.

## Error Handling

Filesystem traversal is best-effort. An unreadable directory, a drive that becomes unavailable, a malformed entry, or metadata failure skips only the affected branch. These conditions do not abort discovery on other drives.

Warnings are bounded to prevent thousands of access-denied messages from overwhelming memory, the database, or the diagnostic UI. The result preserves a small representative sample and a total skipped-directory count.

If fixed-drive enumeration itself fails, the command fails. If one fixed-drive root cannot be opened, discovery records a warning and continues with the remaining roots; the command fails only when none of the enumerated fixed-drive roots can be opened. A database or import error uses the existing scanner error behavior. Source-file operations remain read-only throughout discovery and import.

## Performance and Safety

Discovery compares directory names during traversal and does not read ordinary file contents. File enumeration below a candidate is limited to the minimum direct-child validation needed to find one MP4. Accepted candidates are not traversed recursively.

The implementation does not follow reparse points, preventing cycles and accidental traversal onto excluded volumes. It performs discovery on the existing blocking task runtime so filesystem I/O does not block the Tauri UI thread.

The first search may take from seconds to several minutes depending on directory count and disk speed. No persistent index or background process is created.

## Testing

### Rust unit tests

Temporary directory fixtures cover:

- a valid deeply nested `wonderfulVideos*` candidate;
- case-insensitive candidate and `.mp4` matching;
- rejection of a similarly named directory that does not start with `wonderfulVideos`;
- rejection when no direct group directory contains a direct MP4;
- deduplication when multiple valid candidates share a parent;
- separate roots when candidates have different parents;
- no descent through symbolic links or reparse points;
- continuation after an unreadable branch;
- bounded warning samples with an accurate total skip count; and
- progress metrics increasing during discovery.

Windows drive enumeration has a narrow Windows-only test that verifies returned roots are classified as fixed. Core traversal tests inject fixture roots and never enumerate real drives.

### Rust integration tests

An integration fixture verifies the full pipeline from discovered parent root through the existing scanner to SQLite clip rows. It also verifies that no-result discovery preserves existing indexed clips and that ordinary default/custom scan behavior is unchanged.

### Frontend tests

Frontend tests verify:

- the “全电脑发现” button is present;
- invoking it calls the dedicated backend API;
- all scan buttons are disabled while discovery or import is active;
- discovery progress is presented as indeterminate with live metrics;
- the no-result state is non-error;
- the completion message includes discovered source and imported clip counts; and
- bounded discovery diagnostics remain accessible.

### Verification commands

The implementation is accepted only after the relevant focused tests, the full frontend test suite, the full Rust test suite, the TypeScript build, and `cargo check` pass.

## Acceptance Criteria

- A user can click one independent button to discover and immediately import standard ACLOS sources from all local fixed drives.
- The user does not need Everything, Windows Search indexing, administrator privileges, or a preconfigured source path.
- Only validated `wonderfulVideos*` directories are imported.
- Removable and network storage are not searched.
- Inaccessible branches do not prevent accessible drives and directories from completing.
- Discovery and import expose visible progress and a clear final result.
- The feature never writes to ACLOS source directories.
- Existing default scan, custom scan, incremental indexing, and user metadata preservation continue to behave as before.
