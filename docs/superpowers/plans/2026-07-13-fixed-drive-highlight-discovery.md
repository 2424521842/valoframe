# Fixed-Drive Highlight Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a user-triggered “全电脑发现” action that natively finds valid ACLOS `wonderfulVideos*` sources on local fixed drives and immediately imports them through the existing scanner.

**Architecture:** A new Rust `drive_discovery` module enumerates Windows fixed drives, iteratively traverses directory names, validates candidate source directories, and returns deduplicated parent scan roots plus bounded diagnostics. A new Tauri command composes discovery with a discovered-root variant of the existing multi-root scanner, while React reuses the current scan progress channel and scan card.

**Tech Stack:** Rust 2021, `windows-sys` 0.61.2, Tauri 2, rusqlite, React 19, TypeScript 5.8, Node test runner via `tsx`.

## Global Constraints

- Search local fixed drives only; exclude removable, optical, mapped network, and network storage.
- Match directory names starting with `wonderfulVideos` and `.mp4` extensions case-insensitively.
- Validate only direct child group directories containing direct MP4 files.
- Do not follow symbolic links, directory junctions, or other Windows reparse points.
- Do not use Everything, Windows Search, a persistent index, a background service, or administrator privileges.
- Do not modify, move, rename, or delete source files.
- Do not change ordinary default scan or custom scan behavior.
- Automated tests must use fixture roots and must never scan the developer's real fixed drives.

---

## File Structure

- Create `src-tauri/src/drive_discovery.rs`: fixed-drive enumeration, iterative discovery, candidate validation, bounded warnings, and unit fixtures.
- Modify `src-tauri/Cargo.toml`: add a Windows-only `windows-sys` dependency for drive classification and reparse attributes.
- Modify `src-tauri/src/lib.rs`: register the discovery module and Tauri command.
- Modify `src-tauri/src/scanner.rs`: expose an empty summary constructor and add discovered custom-root multi-scan orchestration without changing default-root semantics.
- Modify `src-tauri/src/commands.rs`: compose discovery, scan, progress events, and the serialized result contract.
- Modify `src/types.ts` and `src/api/backend.ts`: add the frontend result type and command wrapper.
- Modify `src/lib/scanSummary.ts`: produce accurate no-result and completed discovery activity messages.
- Modify `src/App.tsx`: add the full-drive handler and connect it to shared scan state.
- Modify `src/components/SourceSidebar.tsx` and `src/App.css`: render the independent action and discovery-specific progress metadata.
- Modify `tests/scanSummary.test.ts` and `tests/sourceSidebarLayout.test.ts`: verify messages, button state, and indeterminate discovery progress.
- Create `tests/fullDriveDiscovery.test.ts`: verify command wiring without invoking a real Tauri backend.
- Modify `README.md` and `docs/ARCHITECTURE.md`: document the new explicit discovery path and its constraints.

---

### Task 1: Native fixed-drive discovery engine

**Files:**
- Create: `src-tauri/src/drive_discovery.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `fixed_drive_roots() -> Result<Vec<PathBuf>, String>`.
- Produces: `discover_scan_roots<F>(roots: &[PathBuf], progress: F) -> DiscoveryResult where F: Fn(DiscoveryProgress)`.
- Produces: `DiscoveryResult { fixed_drive_count, opened_drive_count, visited_directory_count, validated_source_dir_count, scan_roots, skipped_directory_count, warnings }`.
- Produces: `DiscoveryProgress { current_drive, visited_directory_count, validated_source_dir_count, message }`.

- [ ] **Step 1: Add failing discovery unit tests**

Add an internal `#[cfg(test)]` module to the new file with fixture tests that express the desired API:

```rust
#[test]
fn discovers_nested_valid_source_and_returns_its_parent_root() {
    let fixture = TestFixture::new("nested-valid");
    let scan_root = fixture.path().join("Archive");
    let group = scan_root
        .join("WonderfulVideos1001")
        .join("match-a");
    fs::create_dir_all(&group).unwrap();
    fs::write(group.join("ACE.MP4"), b"video").unwrap();

    let result = discover_scan_roots(&[fixture.path().to_path_buf()], |_| {});

    assert_eq!(result.scan_roots, vec![scan_root]);
    assert_eq!(result.validated_source_dir_count, 1);
    assert_eq!(result.opened_drive_count, 1);
}

#[test]
fn rejects_candidate_without_direct_group_mp4() {
    let fixture = TestFixture::new("invalid-candidate");
    let nested = fixture
        .path()
        .join("wonderfulVideos1001")
        .join("match-a")
        .join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("clip.mp4"), b"video").unwrap();

    let result = discover_scan_roots(&[fixture.path().to_path_buf()], |_| {});

    assert!(result.scan_roots.is_empty());
    assert_eq!(result.validated_source_dir_count, 0);
}

#[test]
fn deduplicates_shared_parent_and_bounds_read_warnings() {
    let fixture = TestFixture::new("dedupe-warnings");
    for suffix in ["1001", "1002"] {
        let group = fixture
            .path()
            .join(format!("wonderfulVideos{suffix}"))
            .join("match-a");
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join("clip.mp4"), b"video").unwrap();
    }
    let invalid_roots = (0..12)
        .map(|index| {
            let path = fixture.path().join(format!("not-a-directory-{index}"));
            fs::write(&path, b"file").unwrap();
            path
        })
        .collect::<Vec<_>>();
    let mut roots = vec![fixture.path().to_path_buf()];
    roots.extend(invalid_roots);

    let result = discover_scan_roots(&roots, |_| {});

    assert_eq!(result.scan_roots, vec![fixture.path().to_path_buf()]);
    assert_eq!(result.validated_source_dir_count, 2);
    assert_eq!(result.skipped_directory_count, 12);
    assert_eq!(result.warnings.len(), MAX_WARNING_SAMPLES);
}
```

Also test the pure `has_reparse_attribute(attributes: u32)` helper with `FILE_ATTRIBUTE_REPARSE_POINT`, and record progress events to assert that `visited_directory_count` and `validated_source_dir_count` increase.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml drive_discovery --lib
```

Expected: compilation fails because `drive_discovery` types and functions do not exist.

- [ ] **Step 3: Add the Windows dependency and minimal module registration**

Add to `src-tauri/Cargo.toml`:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61.2", features = [
  "Win32_Storage_FileSystem",
  "Win32_System_WindowsProgramming",
] }
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod drive_discovery;
```

- [ ] **Step 4: Implement the discovery contracts and traversal**

Implement these concrete types and constants in `drive_discovery.rs`:

```rust
pub const MAX_WARNING_SAMPLES: usize = 8;
const PROGRESS_INTERVAL_DIRECTORIES: u64 = 250;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryProgress {
    pub current_drive: String,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    pub fixed_drive_count: u64,
    pub opened_drive_count: u64,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub scan_roots: Vec<PathBuf>,
    pub skipped_directory_count: u64,
    pub warnings: Vec<String>,
}
```

Use `GetLogicalDriveStringsW` and `GetDriveTypeW`; keep only `DRIVE_FIXED`. Parse the returned UTF-16 multi-string without lossy path construction. On non-Windows targets, return `Err("全电脑发现仅支持 Windows".to_string())`.

Use a `VecDeque<(PathBuf, String)>` work queue. Read directory entries with `fs::read_dir`; on error increment `skipped_directory_count`, append at most `MAX_WARNING_SAMPLES`, and continue. Call `fs::symlink_metadata` before queueing child directories and skip `file_type().is_symlink()` or metadata whose Windows attributes include `FILE_ATTRIBUTE_REPARSE_POINT`.

Candidate validation must implement exactly:

```rust
fn is_candidate_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("wonderfulvideos"))
}

fn candidate_has_direct_group_mp4(path: &Path) -> Result<bool, String> {
    for group in read_child_directories(path)? {
        if read_entries(&group)?.into_iter().any(|entry| {
            entry.is_file()
                && entry.extension().and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}
```

Normalize and deduplicate accepted parent roots with `crate::db::normalize_path`. Sort returned roots case-insensitively for deterministic tests. Emit progress on each drive start, every 250 visited directories, every accepted candidate, and completion.

- [ ] **Step 5: Run discovery tests and verify GREEN**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml drive_discovery --lib
```

Expected: all `drive_discovery` tests pass; no real fixed-drive enumeration occurs in traversal tests.

- [ ] **Step 6: Format and commit the discovery engine**

Run:

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/drive_discovery.rs
git commit -m "feat: discover ACLOS sources on fixed drives"
```

Expected: commit contains only the discovery engine and dependency registration.

---

### Task 2: Reuse the scanner for discovered custom roots

**Files:**
- Modify: `src-tauri/src/scanner.rs`

**Interfaces:**
- Consumes: ordered, deduplicated parent roots from `DiscoveryResult.scan_roots`.
- Produces: `ScanSummary::empty(root_path: String) -> ScanSummary` as a public constructor.
- Produces: `scan_discovered_aclos_roots_with_progress<F>(connection: &Connection, roots: &[PathBuf], progress: F) -> DbResult<ScanSummary>`.

- [ ] **Step 1: Write the failing scanner test**

Add to the existing scanner test module:

```rust
#[test]
fn scan_discovered_roots_indexes_each_root_as_external() {
    let _env_guard = ENV_LOCK.lock().unwrap();
    let fixture = TestFixture::new("discovered-roots");
    let appdata = fixture.path().join("AppData");
    fs::create_dir_all(&appdata).unwrap();
    let original_appdata = std::env::var_os("APPDATA");
    std::env::set_var("APPDATA", &appdata);

    let roots = [fixture.path().join("ArchiveA"), fixture.path().join("ArchiveB")];
    for (index, root) in roots.iter().enumerate() {
        let group = root
            .join(format!("wonderfulVideos{index}"))
            .join(format!("match-{index}"));
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join(format!("clip-{index}.mp4")), b"video").unwrap();
    }

    let connection = Connection::open_in_memory().unwrap();
    db::initialize_schema(&connection).unwrap();
    let summary = scan_discovered_aclos_roots_with_progress(&connection, &roots, |_| {})
        .unwrap();

    match original_appdata {
        Some(value) => std::env::set_var("APPDATA", value),
        None => std::env::remove_var("APPDATA"),
    }
    assert_eq!(summary.source_dir_count, 2);
    assert_eq!(summary.clip_group_count, 2);
    assert_eq!(db::list_clips(&connection).unwrap().len(), 2);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml scan_discovered_roots_indexes_each_root_as_external --lib
```

Expected: compilation fails because `scan_discovered_aclos_roots_with_progress` does not exist.

- [ ] **Step 3: Refactor the private multi-root helper minimally**

Change the private helper to:

```rust
fn scan_library_roots(
    connection: &Connection,
    roots: &[PathBuf],
    all_roots_external: bool,
    progress: Option<ScanProgressReporter<'_>>,
) -> DbResult<ScanSummary>
```

Compute `allow_external` as:

```rust
let allow_external = all_roots_external || index > 0;
```

Pass `false` from both default-library entry points. Add:

```rust
pub fn scan_discovered_aclos_roots_with_progress<F>(
    connection: &Connection,
    roots: &[PathBuf],
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    scan_library_roots(connection, roots, true, Some(&progress))
}
```

Make only the existing constructor visible:

```rust
impl ScanSummary {
    pub fn empty(root_path: String) -> Self { /* existing fields unchanged */ }
}
```

- [ ] **Step 4: Run focused and existing scanner tests**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml scan_discovered_roots_indexes_each_root_as_external --lib
cargo test --manifest-path src-tauri/Cargo.toml scan_default_aclos_library --lib
```

Expected: discovered roots and existing default-library tests pass.

- [ ] **Step 5: Commit the scanner orchestration**

```powershell
git add src-tauri/src/scanner.rs
git commit -m "feat: scan discovered ACLOS roots"
```

---

### Task 3: Tauri discovery-and-scan command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `fixed_drive_roots`, `discover_scan_roots`, and `scan_discovered_aclos_roots_with_progress`.
- Produces: Tauri command `discover_and_scan_fixed_drives`.
- Produces serialized `FullDriveScanResult` with discovery metrics, bounded warnings, `scanned_clip_count`, and `scan_summary`.

- [ ] **Step 1: Add a failing command-module test for fixture orchestration**

Add a non-Tauri helper inside private `commands.rs`:

```rust
fn discover_and_scan_roots<F, G>(
    connection: &Connection,
    roots: &[PathBuf],
    discovery_progress: F,
    scan_progress: G,
) -> Result<FullDriveScanResult, String>
where
    F: Fn(DiscoveryProgress),
    G: Fn(ScanProgress),
```

In a `#[cfg(test)] mod tests` inside `commands.rs`, create a fixture containing one valid source and assert:

```rust
let result = discover_and_scan_roots(&connection, &[fixture.path().to_path_buf()], |_| {}, |_| {})?;
assert_eq!(result.validated_source_dir_count, 1);
assert_eq!(result.scan_root_count, 1);
assert_eq!(result.scanned_clip_count, 1);
assert_eq!(result.scan_summary.new_clip_count, 1);
```

Also seed an existing clip, run a no-candidate fixture, and assert the clip remains available and `result.scan_summary.message == Some("未发现标准无畏时刻素材".to_string())`.

- [ ] **Step 2: Run the integration test and verify RED**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml commands::tests::discover_and_scan_roots
```

Expected: compilation fails because the orchestration helper and result contract do not exist.

- [ ] **Step 3: Implement the serialized result and pure orchestration helper**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDriveScanResult {
    pub fixed_drive_count: u64,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub scan_root_count: u64,
    pub skipped_directory_count: u64,
    pub discovery_warnings: Vec<String>,
    pub scanned_clip_count: i64,
    pub scan_summary: scanner::ScanSummary,
}
```

The helper must fail when `opened_drive_count == 0`. For no candidates, return `ScanSummary::empty(joined_roots)` with the exact message `未发现标准无畏时刻素材`. For candidates, run the discovered-root scanner. Capture the maximum `clip_file_count` seen in scan progress with `Cell<i64>` and return it as `scanned_clip_count`. Prefix the scan summary errors with the bounded discovery warnings so the existing diagnostics UI remains useful.

- [ ] **Step 4: Implement the Tauri wrapper and progress mapping**

Add command:

```rust
#[tauri::command]
pub async fn discover_and_scan_fixed_drives(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDriveScanResult, String>
```

Inside `spawn_blocking`, call `fixed_drive_roots()`, then the pure helper. Map discovery events to `scanner::ScanProgress` using phase `drive-discovery`, `current = visited_directory_count`, `total = 0`, `source_dir_count = validated_source_dir_count`, and the discovery message. Forward scanner events unchanged on `scan-progress`.

Register `commands::discover_and_scan_fixed_drives` in `tauri::generate_handler!`.

- [ ] **Step 5: Run integration and command compilation checks**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml commands::tests::discover_and_scan_roots
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: integration test passes and the Tauri command compiles.

- [ ] **Step 6: Commit the backend command**

```powershell
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: expose full-drive highlight discovery"
```

---

### Task 4: Frontend contract, action, and progress presentation

**Files:**
- Modify: `src/types.ts`
- Modify: `src/api/backend.ts`
- Modify: `src/lib/scanSummary.ts`
- Modify: `src/App.tsx`
- Modify: `src/components/SourceSidebar.tsx`
- Modify: `src/App.css`
- Modify: `tests/scanSummary.test.ts`
- Modify: `tests/sourceSidebarLayout.test.ts`
- Create: `tests/fullDriveDiscovery.test.ts`

**Interfaces:**
- Produces: TypeScript `FullDriveScanResult` matching Rust camelCase serialization.
- Produces: `discoverAndScanFixedDrives(): Promise<FullDriveScanResult>`.
- Produces: `fullDriveDiscoveryActivityMessage(result: FullDriveScanResult): string`.
- Extends: `SourceSidebarProps` with `onDiscoverAll: () => void`.

- [ ] **Step 1: Write failing formatter and sidebar tests**

Add formatter assertions:

```typescript
test("summarizes empty full-drive discovery as a normal result", () => {
  assert.equal(
    fullDriveDiscoveryActivityMessage({
      ...baseDiscoveryResult,
      validatedSourceDirCount: 0,
      scanRootCount: 0,
      scannedClipCount: 0,
    }),
    "未发现标准无畏时刻素材",
  );
});

test("summarizes discovered sources and scanned videos", () => {
  assert.equal(
    fullDriveDiscoveryActivityMessage({
      ...baseDiscoveryResult,
      validatedSourceDirCount: 3,
      scanRootCount: 2,
      scannedClipCount: 41,
    }),
    "全电脑发现完成：3 个素材目录，扫描 41 个视频",
  );
});
```

Update the sidebar fixture with `onDiscoverAll: noop`, then assert the idle markup contains `全电脑发现`. Add a `phase: "drive-discovery"` progress fixture with `current: 1250`, `total: 0`, `sourceDirCount: 2`, and assert an indeterminate progressbar plus `1,250 个目录 · 2 个候选`.

Create `tests/fullDriveDiscovery.test.ts` to read `src/api/backend.ts` and `src/App.tsx`, asserting the command name `discover_and_scan_fixed_drives`, the handler call, and the `onDiscoverAll={handleDiscoverAll}` prop wiring.

- [ ] **Step 2: Run frontend tests and verify RED**

```powershell
npm test -- --test-name-pattern "full-drive|全电脑|discovery"
```

Expected: tests fail because the formatter, API wrapper, button, and handler are absent.

- [ ] **Step 3: Add the TypeScript result contract and API wrapper**

Add to `src/types.ts`:

```typescript
export type FullDriveScanResult = {
  fixedDriveCount: number;
  visitedDirectoryCount: number;
  validatedSourceDirCount: number;
  scanRootCount: number;
  skippedDirectoryCount: number;
  discoveryWarnings: string[];
  scannedClipCount: number;
  scanSummary: ScanSummary;
};
```

Add to `src/api/backend.ts`:

```typescript
export async function discoverAndScanFixedDrives(): Promise<FullDriveScanResult> {
  return invoke<FullDriveScanResult>("discover_and_scan_fixed_drives");
}
```

Add the exact formatter behavior tested above to `src/lib/scanSummary.ts`.

- [ ] **Step 4: Add the App handler**

Follow the existing scan handlers:

```typescript
const handleDiscoverAll = async () => {
  if (isScanning) return;
  setIsScanning(true);
  setScanProgress(initialScanProgress("正在搜索本机固定磁盘"));
  setScanError(null);
  setScanSummary(null);
  setActivityMessage("正在全电脑发现无畏时刻素材");
  try {
    const result = await discoverAndScanFixedDrives();
    setScanSummary(result.scanSummary);
    setScanProgress(null);
    const refreshed = await loadClipList({ preserveActivity: true });
    setActivityMessage(
      refreshed
        ? fullDriveDiscoveryActivityMessage(result)
        : "发现完成，但刷新素材列表失败",
    );
  } catch (error) {
    setScanError(commandErrorMessage(error));
    setScanProgress(null);
    setActivityMessage("全电脑发现失败");
  } finally {
    setIsScanning(false);
  }
};
```

Pass `onDiscoverAll={handleDiscoverAll}` to `SourceSidebar`.

- [ ] **Step 5: Render the independent action and discovery progress**

Add `onDiscoverAll` to the sidebar prop type and destructuring. Replace the single scan button with:

```tsx
<div className="scan-actions">
  <button disabled={isScanning || isLoading} type="button" onClick={onRescan}>
    {isScanning ? "扫描中..." : "重新扫描"}
  </button>
  <button
    className="scan-discover-button"
    disabled={isScanning || isLoading}
    type="button"
    onClick={onDiscoverAll}
  >
    全电脑发现
  </button>
</div>
```

Replace the three-part hardcoded progress metadata with `scanProgressMeta(progress)`. For `drive-discovery`, return `${current} 个目录 · ${sourceDirCount} 个候选`; otherwise preserve the current source/group/video string exactly. Add a two-column `.scan-actions` layout that collapses to one column at the existing narrow sidebar breakpoint.

- [ ] **Step 6: Run focused tests, full frontend tests, and build**

```powershell
npm test -- --test-name-pattern "full-drive|全电脑|discovery"
npm test
npm run build
```

Expected: focused and full tests pass; TypeScript and Vite build succeed.

- [ ] **Step 7: Commit the frontend feature**

```powershell
git add src/types.ts src/api/backend.ts src/lib/scanSummary.ts src/App.tsx src/components/SourceSidebar.tsx src/App.css tests/scanSummary.test.ts tests/sourceSidebarLayout.test.ts tests/fullDriveDiscovery.test.ts
git commit -m "feat: add full-computer discovery action"
```

---

### Task 5: Documentation and final verification

**Files:**
- Modify: `README.md`
- Modify: `docs/ARCHITECTURE.md`

**Interfaces:**
- Documents: user-visible trigger, fixed-drive-only scope, native traversal, validation rules, and read-only behavior.

- [ ] **Step 1: Update product and architecture documentation**

In `README.md`, add a current-capability bullet stating that “全电脑发现” is an explicit action that searches local fixed drives for validated `wonderfulVideos*` sources without Everything or Windows Search.

In `docs/ARCHITECTURE.md`, add `drive_discovery.rs` to the backend module list and document the data flow:

```text
全电脑发现按钮 → discover_and_scan_fixed_drives
→ 固定磁盘枚举 → wonderfulVideos* 候选验证
→ 去重后的父目录 → 现有只读多根扫描器 → SQLite
```

- [ ] **Step 2: Run formatting and repository-wide verification**

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm test
npm run build
git diff --check
```

Expected: every command exits 0 with no new warnings attributable to this feature. Do not invoke the production full-drive command during automated verification because it would scan real disks and mutate the user's application database.

- [ ] **Step 3: Review scope and commit docs**

```powershell
git status --short
git diff --stat
git add README.md docs/ARCHITECTURE.md
git commit -m "docs: document fixed-drive discovery"
```

Expected: only intentional feature and documentation files remain changed; no generated output or real scan database is added.
