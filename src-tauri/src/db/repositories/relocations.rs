//! Fail-closed source-root relocation planning and persistence.
//!
//! Preview and commit deliberately share the same planner.  A commit opens an IMMEDIATE
//! transaction first and then rebuilds the plan, so it never trusts a stale UI preview or a
//! database snapshot taken before the relocation lease was acquired.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, Transaction,
    TransactionBehavior,
};
use serde::Serialize;

use crate::file_identity::{read_stable_file_snapshot, StableFileIdentity};

use super::super::{
    normalize_path, readable_error, stable_path_for_storage, DbResult, SourceDir, SourceKind,
};
use super::{sources, thumbnails};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRelocationConflict {
    pub code: String,
    pub message: String,
    pub old_clip_ids: Vec<String>,
    pub candidate_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRelocationBlocker {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedRelocationSource {
    pub id: i64,
    pub display_name: String,
    pub old_source_path: String,
    pub new_source_path: String,
    pub clip_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSourceRelocationPreview {
    pub source_id: i64,
    pub old_root_path: String,
    pub new_root_path: String,
    pub affected_sources: Vec<AffectedRelocationSource>,
    pub exact_path_match_count: usize,
    pub identity_match_count: usize,
    pub legacy_fingerprint_match_count: usize,
    pub unmatched_count: usize,
    pub new_candidate_count: usize,
    pub expected_clip_update_count: usize,
    pub expected_group_update_count: usize,
    pub expected_cover_update_count: usize,
    pub expected_metadata_reference_update_count: usize,
    pub conflicts: Vec<SourceRelocationConflict>,
    pub blockers: Vec<SourceRelocationBlocker>,
    pub can_relocate: bool,
}

#[derive(Debug, Clone)]
pub struct CommittedSourceRelocation {
    pub preview: ScanSourceRelocationPreview,
    pub relocated_clip_count: usize,
    pub affected_source_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct RelocationSource {
    source: SourceDir,
    new_path: PathBuf,
    clip_count: i64,
}

#[derive(Debug, Clone)]
struct OldClip {
    id: i64,
    source_dir_id: i64,
    clip_group_id: Option<i64>,
    clip_group_key: Option<String>,
    file_path: String,
    file_name: String,
    size_bytes: i64,
    modified_at: Option<String>,
    identity: Option<StableFileIdentity>,
    cover_path: Option<String>,
}

#[derive(Debug, Clone)]
struct RelocationCandidate {
    source_dir_id: i64,
    path: PathBuf,
    file_path: String,
    normalized_path: String,
    relative_components: Vec<String>,
    relative_key: String,
    file_name: String,
    size_bytes: i64,
    modified_at: String,
    modified_time: SystemTime,
    identity: Option<StableFileIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    ExactPath,
    StableIdentity,
    LegacyFingerprint,
}

#[derive(Debug, Clone)]
struct RelocationMatch {
    old_index: usize,
    candidate_index: usize,
    kind: MatchKind,
}

#[derive(Debug, Clone)]
struct RelocationPlan {
    preview: ScanSourceRelocationPreview,
    old_root: PathBuf,
    new_root: PathBuf,
    excluded_reference_roots: Vec<PathBuf>,
    sources: Vec<RelocationSource>,
    old_clips: Vec<OldClip>,
    candidates: Vec<RelocationCandidate>,
    matches: Vec<RelocationMatch>,
}

type IdentityReader<'a> = dyn Fn(&Path) -> Option<StableFileIdentity> + 'a;

pub fn preview_scan_source_relocation(
    connection: &Connection,
    source_id: i64,
    new_root_path: &Path,
) -> DbResult<ScanSourceRelocationPreview> {
    Ok(build_relocation_plan(
        connection,
        source_id,
        new_root_path,
        &production_identity_reader,
    )?
    .preview)
}

/// Rebuilds and applies a relocation inside one IMMEDIATE transaction.
///
/// The caller owns the process-wide relocation lease. This function intentionally does not
/// modify source health/freshness and does not start a scan.
pub fn commit_scan_source_relocation(
    connection: &Connection,
    source_id: i64,
    new_root_path: &Path,
) -> DbResult<CommittedSourceRelocation> {
    commit_scan_source_relocation_with_reader(
        connection,
        source_id,
        new_root_path,
        &production_identity_reader,
    )
}

fn commit_scan_source_relocation_with_reader(
    connection: &Connection,
    source_id: i64,
    new_root_path: &Path,
    identity_reader: &IdentityReader<'_>,
) -> DbResult<CommittedSourceRelocation> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting source relocation", error))?;
    let plan = build_relocation_plan(&transaction, source_id, new_root_path, identity_reader)?;
    if !plan.preview.can_relocate {
        return Err(relocation_rejection_message(&plan.preview));
    }

    apply_relocation_plan(&transaction, &plan, identity_reader)?;
    assert_transaction_integrity(&transaction)?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing source relocation", error))?;

    Ok(CommittedSourceRelocation {
        relocated_clip_count: plan.matches.len(),
        affected_source_ids: plan.sources.iter().map(|source| source.source.id).collect(),
        preview: plan.preview,
    })
}

fn production_identity_reader(path: &Path) -> Option<StableFileIdentity> {
    read_stable_file_snapshot(path).ok()?.identity
}

fn build_relocation_plan(
    connection: &Connection,
    source_id: i64,
    requested_new_root: &Path,
    identity_reader: &IdentityReader<'_>,
) -> DbResult<RelocationPlan> {
    if source_id <= 0 {
        return Err("source relocation source id must be positive".to_string());
    }
    let all_sources = sources::list_source_dirs(connection)?;
    let requested_source = all_sources
        .iter()
        .find(|source| source.id == source_id)
        .cloned()
        .ok_or_else(|| format!("Source id {source_id} was not found"))?;
    let old_root = PathBuf::from(&requested_source.scan_root_path);
    let old_root_key = filesystem_path_key(Path::new(&requested_source.scan_root_path));
    let affected_rows = all_sources
        .iter()
        .filter(|source| filesystem_path_key(Path::new(&source.scan_root_path)) == old_root_key)
        .cloned()
        .collect::<Vec<_>>();
    let affected_ids = affected_rows
        .iter()
        .map(|source| source.id)
        .collect::<BTreeSet<_>>();
    let excluded_reference_roots =
        unaffected_reference_roots(&all_sources, &affected_ids, &old_root);

    let mut blockers = Vec::new();
    let new_root = match validate_relocation_root(requested_new_root) {
        Ok(root) => root,
        Err(message) => {
            blockers.push(SourceRelocationBlocker {
                code: "invalid-new-root".to_string(),
                message,
            });
            requested_new_root.to_path_buf()
        }
    };

    let mut relocation_sources = Vec::with_capacity(affected_rows.len());
    for source in affected_rows {
        let relative = relative_components(Path::new(&source.path), &old_root);
        let new_path = match relative {
            Some(relative) => join_relative_components(&new_root, &relative),
            None => {
                blockers.push(SourceRelocationBlocker {
                    code: "source-outside-old-root".to_string(),
                    message: format!(
                        "来源 {} 的逻辑路径不在旧扫描根 {} 内",
                        source.path,
                        old_root.display()
                    ),
                });
                new_root.clone()
            }
        };
        let clip_count = connection
            .query_row(
                "SELECT COUNT(*) FROM clips WHERE source_dir_id = ?1",
                params![source.id],
                |row| row.get(0),
            )
            .map_err(|error| readable_error("counting relocation source clips", error))?;
        if !blockers
            .iter()
            .any(|blocker| blocker.code == "invalid-new-root")
        {
            match fs::symlink_metadata(&new_path) {
                Ok(metadata)
                    if metadata.is_dir()
                        && !metadata_is_reparse_point(&metadata)
                        && new_path
                            .canonicalize()
                            .is_ok_and(|path| path.starts_with(&new_root)) => {}
                Ok(_) => blockers.push(SourceRelocationBlocker {
                    code: "invalid-logical-source-path".to_string(),
                    message: format!(
                        "来源“{}”在新根内的逻辑目录不是普通目录：{}",
                        source.name,
                        new_path.display()
                    ),
                }),
                Err(error) => blockers.push(SourceRelocationBlocker {
                    code: "missing-logical-source-path".to_string(),
                    message: format!(
                        "无法访问来源“{}”在新根内的逻辑目录 {}：{error}",
                        source.name,
                        new_path.display()
                    ),
                }),
            }
        }
        relocation_sources.push(RelocationSource {
            source,
            new_path,
            clip_count,
        });
    }

    validate_unaffected_source_overlap(
        &all_sources,
        &affected_ids,
        &old_root,
        &new_root,
        &mut blockers,
    );
    validate_final_source_paths(
        connection,
        &relocation_sources,
        &affected_ids,
        &mut blockers,
    )?;

    let old_clips = load_old_clips(connection, &affected_ids)?;
    add_protected_state_blockers(connection, &affected_ids, &mut blockers)?;

    let mut enumeration_blockers = Vec::new();
    let candidates = if blockers
        .iter()
        .any(|blocker| blocker.code == "invalid-new-root")
    {
        Vec::new()
    } else {
        let enumerated =
            enumerate_candidates(&new_root, identity_reader, &mut enumeration_blockers);
        partition_candidates(enumerated, &relocation_sources, &mut enumeration_blockers)
    };
    blockers.extend(enumeration_blockers);

    let mut conflicts = Vec::new();
    let matches = match_candidates(
        &old_root,
        &new_root,
        &old_clips,
        &candidates,
        &mut conflicts,
    );
    validate_final_clip_paths(
        connection,
        &old_clips,
        &candidates,
        &matches,
        &mut conflicts,
    )?;

    let matched_old = matches
        .iter()
        .map(|matched| matched.old_index)
        .collect::<HashSet<_>>();
    let matched_candidates = matches
        .iter()
        .map(|matched| matched.candidate_index)
        .collect::<HashSet<_>>();
    let exact_path_match_count = matches
        .iter()
        .filter(|matched| matched.kind == MatchKind::ExactPath)
        .count();
    let identity_match_count = matches
        .iter()
        .filter(|matched| matched.kind == MatchKind::StableIdentity)
        .count();
    let legacy_fingerprint_match_count = matches
        .iter()
        .filter(|matched| matched.kind == MatchKind::LegacyFingerprint)
        .count();
    let expected_group_update_count =
        expected_group_updates(&relocation_sources, &old_clips, &candidates, &matches);
    let expected_cover_update_count = matches
        .iter()
        .filter(|matched| {
            old_clips[matched.old_index]
                .cover_path
                .as_deref()
                .is_some_and(|path| relative_components(Path::new(path), &old_root).is_some())
        })
        .count();
    let expected_metadata_reference_update_count = count_metadata_reference_updates(
        connection,
        &old_root,
        &excluded_reference_roots,
        &matched_old,
        &old_clips,
    )?;

    if matches.is_empty() {
        blockers.push(SourceRelocationBlocker {
            code: "zero-trusted-matches".to_string(),
            message: "新目录中没有找到任何可信的原素材匹配，不能重新定位".to_string(),
        });
    }
    let affected_sources = relocation_sources
        .iter()
        .map(|source| AffectedRelocationSource {
            id: source.source.id,
            display_name: source.source.name.clone(),
            old_source_path: source.source.path.clone(),
            new_source_path: stable_path_for_storage(&source.new_path.to_string_lossy()),
            clip_count: source.clip_count,
        })
        .collect();
    let preview = ScanSourceRelocationPreview {
        source_id,
        old_root_path: stable_path_for_storage(&old_root.to_string_lossy()),
        new_root_path: stable_path_for_storage(&new_root.to_string_lossy()),
        affected_sources,
        exact_path_match_count,
        identity_match_count,
        legacy_fingerprint_match_count,
        unmatched_count: old_clips.len().saturating_sub(matched_old.len()),
        new_candidate_count: candidates.len().saturating_sub(matched_candidates.len()),
        expected_clip_update_count: matches.len(),
        expected_group_update_count,
        expected_cover_update_count,
        expected_metadata_reference_update_count,
        can_relocate: blockers.is_empty() && conflicts.is_empty() && !matches.is_empty(),
        conflicts,
        blockers,
    };

    Ok(RelocationPlan {
        preview,
        old_root,
        new_root,
        excluded_reference_roots,
        sources: relocation_sources,
        old_clips,
        candidates,
        matches,
    })
}

fn validate_relocation_root(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("请选择新的来源根目录".to_string());
    }
    if !path.is_absolute() {
        return Err("新来源根必须使用绝对路径".to_string());
    }
    if has_parent_traversal(path) {
        return Err("新来源根不能包含上级目录跳转（..）".to_string());
    }
    for component_path in path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        let component_metadata = fs::symlink_metadata(component_path).map_err(|error| {
            format!(
                "无法验证新来源目录路径链 {}：{error}",
                component_path.display()
            )
        })?;
        if metadata_is_reparse_point(&component_metadata) {
            return Err(format!(
                "新来源目录路径链包含符号链接或 reparse point：{}",
                component_path.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取新来源目录 {}：{error}", path.display()))?;
    if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
        return Err("新来源根必须是普通目录，不能是文件、符号链接或 reparse point".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("无法规范化新来源目录 {}：{error}", path.display()))?;
    let canonical_metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("无法复验新来源目录 {}：{error}", canonical.display()))?;
    if !canonical_metadata.is_dir() || metadata_is_reparse_point(&canonical_metadata) {
        return Err("新来源根的规范路径不是普通目录".to_string());
    }
    Ok(canonical)
}

fn validate_unaffected_source_overlap(
    all_sources: &[SourceDir],
    affected_ids: &BTreeSet<i64>,
    old_root: &Path,
    new_root: &Path,
    blockers: &mut Vec<SourceRelocationBlocker>,
) {
    let old_key = filesystem_path_key(old_root);
    let new_key = filesystem_path_key(new_root);
    for source in all_sources
        .iter()
        .filter(|source| !affected_ids.contains(&source.id))
    {
        let scan_root_key = filesystem_path_key(Path::new(&source.scan_root_path));
        let logical_path_key = filesystem_path_key(Path::new(&source.path));
        if paths_overlap(&old_key, &scan_root_key) || paths_overlap(&old_key, &logical_path_key) {
            blockers.push(SourceRelocationBlocker {
                code: "existing-source-root-overlap".to_string(),
                message: format!(
                    "旧目录与未包含在本次重新定位中的来源“{}”重叠：{}",
                    source.name, source.scan_root_path
                ),
            });
        }
        if paths_overlap(&new_key, &scan_root_key) || paths_overlap(&new_key, &logical_path_key) {
            blockers.push(SourceRelocationBlocker {
                code: "source-root-overlap".to_string(),
                message: format!(
                    "新目录与来源“{}”重叠：{}",
                    source.name, source.scan_root_path
                ),
            });
        }
    }
}

fn unaffected_reference_roots(
    all_sources: &[SourceDir],
    affected_ids: &BTreeSet<i64>,
    old_root: &Path,
) -> Vec<PathBuf> {
    let old_key = filesystem_path_key(old_root);
    let mut roots = HashMap::<String, PathBuf>::new();
    for source in all_sources
        .iter()
        .filter(|source| !affected_ids.contains(&source.id))
    {
        for path in [&source.scan_root_path, &source.path] {
            let root = PathBuf::from(path);
            let key = filesystem_path_key(&root);
            if paths_overlap(&old_key, &key) {
                roots.entry(key).or_insert(root);
            }
        }
    }
    let mut roots = roots.into_values().collect::<Vec<_>>();
    roots.sort_by_key(|root| filesystem_path_key(root));
    roots
}

fn validate_final_source_paths(
    connection: &Connection,
    sources: &[RelocationSource],
    affected_ids: &BTreeSet<i64>,
    blockers: &mut Vec<SourceRelocationBlocker>,
) -> DbResult<()> {
    let mut keys = HashSet::new();
    let all_sources = sources::list_source_dirs(connection)?;
    for source in sources {
        let final_path = source.new_path.display().to_string();
        let key = filesystem_path_key(&source.new_path);
        if !keys.insert(key.clone()) {
            blockers.push(SourceRelocationBlocker {
                code: "duplicate-final-source-path".to_string(),
                message: format!("多个受影响来源会映射到同一路径：{final_path}"),
            });
        }
        let owner = all_sources
            .iter()
            .find(|existing| filesystem_path_key(Path::new(&existing.path)) == key)
            .map(|existing| existing.id);
        if owner.is_some_and(|owner| !affected_ids.contains(&owner)) {
            blockers.push(SourceRelocationBlocker {
                code: "final-source-path-conflict".to_string(),
                message: format!("最终来源路径已被其他来源占用：{final_path}"),
            });
        }
    }
    Ok(())
}

fn load_old_clips(connection: &Connection, source_ids: &BTreeSet<i64>) -> DbResult<Vec<OldClip>> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = sql_placeholders(source_ids.len());
    let sql = format!(
        "SELECT clips.id, clips.source_dir_id, clips.clip_group_id, clip_groups.group_key, clips.file_path, clips.file_name, \
                size_bytes, modified_at, file_volume_serial, file_index_high, file_index_low, \
                cover_path \
         FROM clips
         LEFT JOIN clip_groups ON clip_groups.id = clips.clip_group_id
         WHERE clips.source_dir_id IN ({placeholders}) ORDER BY clips.id"
    );
    let values = source_ids
        .iter()
        .copied()
        .map(Value::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("preparing relocation clips", error))?;
    let clips = statement
        .query_map(params_from_iter(values), |row| {
            Ok(OldClip {
                id: row.get(0)?,
                source_dir_id: row.get(1)?,
                clip_group_id: row.get(2)?,
                clip_group_key: row.get(3)?,
                file_path: row.get(4)?,
                file_name: row.get(5)?,
                size_bytes: row.get(6)?,
                modified_at: row.get(7)?,
                identity: StableFileIdentity::from_database_parts(
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ),
                cover_path: row.get(11)?,
            })
        })
        .map_err(|error| readable_error("querying relocation clips", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading relocation clips", error))?;
    Ok(clips)
}

fn add_protected_state_blockers(
    connection: &Connection,
    source_ids: &BTreeSet<i64>,
    blockers: &mut Vec<SourceRelocationBlocker>,
) -> DbResult<()> {
    if source_ids.is_empty() {
        return Ok(());
    }
    let placeholders = sql_placeholders(source_ids.len());
    let values = source_ids
        .iter()
        .copied()
        .map(Value::Integer)
        .collect::<Vec<_>>();
    let protected = [
        (
            "trashed-clips",
            "受影响来源仍有回收站素材，请先处理回收站后再重新定位",
            format!("SELECT COUNT(*) FROM clips WHERE source_dir_id IN ({placeholders}) AND file_status = 'trashed'"),
        ),
        (
            "trash-snapshots",
            "受影响来源仍有不可改写的回收快照，请先处理回收站后再重新定位",
            format!("SELECT COUNT(*) FROM clip_trash_snapshots WHERE clip_id IN (SELECT id FROM clips WHERE source_dir_id IN ({placeholders}))"),
        ),
        (
            "delete-intents",
            "受影响来源存在永久删除 intent，不能重新定位",
            format!("SELECT COUNT(*) FROM clip_delete_intents WHERE clip_id IN (SELECT id FROM clips WHERE source_dir_id IN ({placeholders}))"),
        ),
    ];
    for (code, message, sql) in protected {
        let count: i64 = connection
            .query_row(&sql, params_from_iter(values.clone()), |row| row.get(0))
            .map_err(|error| readable_error("checking relocation protected state", error))?;
        if count > 0 {
            blockers.push(SourceRelocationBlocker {
                code: code.to_string(),
                message: message.to_string(),
            });
        }
    }
    Ok(())
}

fn enumerate_candidates(
    root: &Path,
    identity_reader: &IdentityReader<'_>,
    blockers: &mut Vec<SourceRelocationBlocker>,
) -> Vec<RelocationCandidate> {
    let mut candidates = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                blockers.push(SourceRelocationBlocker {
                    code: "unreadable-new-root".to_string(),
                    message: format!("无法枚举新目录 {}：{error}", directory.display()),
                });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "unreadable-directory-entry".to_string(),
                        message: format!("无法读取新目录条目：{error}"),
                    });
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "unreadable-candidate".to_string(),
                        message: format!("无法读取候选 {}：{error}", path.display()),
                    });
                    continue;
                }
            };
            if metadata_is_reparse_point(&metadata) {
                blockers.push(SourceRelocationBlocker {
                    code: "reparse-point".to_string(),
                    message: format!(
                        "新目录包含不允许的符号链接或 reparse point：{}",
                        path.display()
                    ),
                });
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() || !has_mp4_extension(&path) {
                continue;
            }
            let canonical = match path.canonicalize() {
                Ok(canonical) if canonical.starts_with(root) && canonical != root => canonical,
                Ok(_) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "candidate-outside-new-root".to_string(),
                        message: format!("候选规范化后越出新目录：{}", path.display()),
                    });
                    continue;
                }
                Err(error) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "candidate-canonicalize-failed".to_string(),
                        message: format!("无法规范化候选 {}：{error}", path.display()),
                    });
                    continue;
                }
            };
            let Some(relative) = relative_components(&canonical, root) else {
                blockers.push(SourceRelocationBlocker {
                    code: "candidate-outside-new-root".to_string(),
                    message: format!("候选不在新目录内：{}", canonical.display()),
                });
                continue;
            };
            let modified_time = match metadata.modified() {
                Ok(modified) => modified,
                Err(error) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "candidate-modified-time-unavailable".to_string(),
                        message: format!(
                            "无法读取候选修改时间，未纳入匹配 {}：{error}",
                            canonical.display()
                        ),
                    });
                    continue;
                }
            };
            let modified_at = format_system_time(modified_time);
            let size_bytes = match i64::try_from(metadata.len()) {
                Ok(size) => size,
                Err(_) => {
                    blockers.push(SourceRelocationBlocker {
                        code: "candidate-size-overflow".to_string(),
                        message: format!("候选文件过大，无法安全索引：{}", canonical.display()),
                    });
                    continue;
                }
            };
            let file_name = canonical
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let file_path = stable_path_for_storage(&canonical.to_string_lossy());
            let identity = identity_reader(&path);
            let stable_after_identity = fs::symlink_metadata(&path).ok().is_some_and(|current| {
                current.is_file()
                    && !metadata_is_reparse_point(&current)
                    && current.len() == metadata.len()
                    && current.modified().ok() == Some(modified_time)
            });
            if !stable_after_identity {
                blockers.push(SourceRelocationBlocker {
                    code: "candidate-changed-during-preview".to_string(),
                    message: format!(
                        "候选在读取身份期间发生变化，未纳入匹配：{}",
                        canonical.display()
                    ),
                });
                continue;
            }
            candidates.push(RelocationCandidate {
                source_dir_id: 0,
                path: canonical,
                normalized_path: normalize_path(&file_path),
                file_path,
                relative_key: relative_key(&relative),
                relative_components: relative,
                file_name,
                size_bytes,
                modified_at,
                modified_time,
                identity,
            });
        }
    }
    candidates.sort_by(|left, right| left.relative_key.cmp(&right.relative_key));
    candidates
}

fn partition_candidates(
    candidates: Vec<RelocationCandidate>,
    sources: &[RelocationSource],
    blockers: &mut Vec<SourceRelocationBlocker>,
) -> Vec<RelocationCandidate> {
    let mut partitioned = Vec::with_capacity(candidates.len());
    for mut candidate in candidates {
        let owners = sources
            .iter()
            .filter(|source| relative_components(&candidate.path, &source.new_path).is_some())
            .collect::<Vec<_>>();
        match owners.as_slice() {
            [] => {
                // Files outside every affected logical source are not authorized candidates and
                // must not influence matching or the new-candidate count.
            }
            [owner] => {
                candidate.source_dir_id = owner.source.id;
                partitioned.push(candidate);
            }
            _ => blockers.push(SourceRelocationBlocker {
                code: "candidate-source-overlap".to_string(),
                message: format!(
                    "候选同时落入多个受影响来源，无法安全归属：{}",
                    candidate.file_path
                ),
            }),
        }
    }
    partitioned
}

fn match_candidates(
    old_root: &Path,
    new_root: &Path,
    old_clips: &[OldClip],
    candidates: &[RelocationCandidate],
    conflicts: &mut Vec<SourceRelocationConflict>,
) -> Vec<RelocationMatch> {
    let mut matches = Vec::new();
    let mut claimed_old = HashSet::new();
    let mut claimed_candidates = HashSet::new();
    let mut exact_mismatches = Vec::new();
    let mut candidates_by_relative = HashMap::<(i64, String), Vec<usize>>::new();
    let mut candidates_by_normalized_path = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        candidates_by_relative
            .entry((candidate.source_dir_id, candidate.relative_key.clone()))
            .or_default()
            .push(index);
        candidates_by_normalized_path
            .entry(candidate.normalized_path.clone())
            .or_default()
            .push(index);
    }
    let mut ambiguous_candidates = HashSet::new();
    for indices in candidates_by_relative
        .values()
        .chain(candidates_by_normalized_path.values())
        .filter(|indices| indices.len() > 1)
    {
        ambiguous_candidates.extend(indices.iter().copied());
    }
    for (normalized_path, indices) in candidates_by_normalized_path
        .iter()
        .filter(|(_, indices)| indices.len() > 1)
    {
        conflicts.push(SourceRelocationConflict {
            code: "candidate-normalized-path-ambiguous".to_string(),
            message: format!("多个候选在 Windows 路径规则下等价，未自动选择：{normalized_path}"),
            old_clip_ids: Vec::new(),
            candidate_paths: indices
                .iter()
                .map(|index| candidates[*index].file_path.clone())
                .collect(),
        });
    }

    // Explicit root relocation gets first priority. Cross-volume moves usually change stable
    // identity, so size+mtime is an accepted exact-relative proof.
    for (old_index, old) in old_clips.iter().enumerate() {
        let Some(relative) = relative_components(Path::new(&old.file_path), old_root) else {
            conflicts.push(SourceRelocationConflict {
                code: "old-clip-outside-root".to_string(),
                message: format!("旧素材路径不在授权根内：{}", old.file_path),
                old_clip_ids: vec![old.id.to_string()],
                candidate_paths: Vec::new(),
            });
            continue;
        };
        let Some(candidate_indices) =
            candidates_by_relative.get(&(old.source_dir_id, relative_key(&relative)))
        else {
            continue;
        };
        if candidate_indices.len() != 1 || ambiguous_candidates.contains(&candidate_indices[0]) {
            continue;
        }
        let candidate_index = candidate_indices[0];
        let candidate = &candidates[candidate_index];
        let identity_matches = old
            .identity
            .zip(candidate.identity)
            .is_some_and(|(left, right)| left == right);
        let fingerprint_matches = old.size_bytes == candidate.size_bytes
            && old.modified_at.as_deref() == Some(candidate.modified_at.as_str());
        if identity_matches || fingerprint_matches {
            claimed_old.insert(old_index);
            claimed_candidates.insert(candidate_index);
            matches.push(RelocationMatch {
                old_index,
                candidate_index,
                kind: MatchKind::ExactPath,
            });
        } else {
            // A whole-root move may also swap/rename directories. Defer this mismatch until the
            // unique identity/fingerprint passes have had a chance to pair both sides safely.
            exact_mismatches.push((old_index, candidate_index, relative));
        }
    }

    // A shared scan root can contain multiple logical sources. Stable identity uniqueness must
    // therefore be established across the complete affected set, not independently per source:
    // a hardlink in a sibling source is still the same ambiguous filesystem object. The final
    // pairing remains source-local below so relocation never moves ownership across sources.
    let mut old_identity_counts: HashMap<(u32, u32, u32), Vec<usize>> = HashMap::new();
    let mut candidate_identity_counts: HashMap<(u32, u32, u32), Vec<usize>> = HashMap::new();
    for (index, old) in old_clips.iter().enumerate() {
        if let Some(identity) = old.identity {
            old_identity_counts
                .entry(identity_key(identity))
                .or_default()
                .push(index);
        }
    }
    for (index, candidate) in candidates.iter().enumerate() {
        if ambiguous_candidates.contains(&index) {
            continue;
        }
        if let Some(identity) = candidate.identity {
            candidate_identity_counts
                .entry(identity_key(identity))
                .or_default()
                .push(index);
        }
    }
    for (key, old_indices) in &old_identity_counts {
        let Some(candidate_indices) = candidate_identity_counts.get(key) else {
            continue;
        };
        let remaining_old = old_indices
            .iter()
            .copied()
            .filter(|index| !claimed_old.contains(index))
            .collect::<Vec<_>>();
        let remaining_candidates = candidate_indices
            .iter()
            .copied()
            .filter(|index| !claimed_candidates.contains(index))
            .collect::<Vec<_>>();
        if remaining_old.is_empty() || remaining_candidates.is_empty() {
            continue;
        }
        // Uniqueness is evaluated against the complete old/new sets, not only unclaimed rows.
        if old_indices.len() == 1
            && candidate_indices.len() == 1
            && old_clips[remaining_old[0]].source_dir_id
                == candidates[remaining_candidates[0]].source_dir_id
        {
            let old_index = remaining_old[0];
            let candidate_index = remaining_candidates[0];
            claimed_old.insert(old_index);
            claimed_candidates.insert(candidate_index);
            matches.push(RelocationMatch {
                old_index,
                candidate_index,
                kind: MatchKind::StableIdentity,
            });
        } else {
            conflicts.push(SourceRelocationConflict {
                code: "identity-ambiguous".to_string(),
                message: "稳定文件身份在旧记录或新目录中不唯一，未自动合并".to_string(),
                old_clip_ids: old_indices
                    .iter()
                    .map(|index| old_clips[*index].id.to_string())
                    .collect(),
                candidate_paths: candidate_indices
                    .iter()
                    .map(|index| candidates[*index].file_path.clone())
                    .collect(),
            });
        }
    }

    // Legacy fallback has the same affected-set uniqueness boundary as stable identity. It is
    // intentionally weaker proof, so duplicate fingerprints in sibling logical sources must not
    // become independently "unique" matches merely because their source ids differ.
    let mut old_legacy_counts: HashMap<(String, i64, String), Vec<usize>> = HashMap::new();
    let mut candidate_legacy_counts: HashMap<(String, i64, String), Vec<usize>> = HashMap::new();
    for (index, old) in old_clips.iter().enumerate() {
        if old.identity.is_none() {
            if let Some(key) =
                legacy_key(&old.file_name, old.size_bytes, old.modified_at.as_deref())
            {
                old_legacy_counts.entry(key).or_default().push(index);
            }
        }
    }
    for (index, candidate) in candidates.iter().enumerate() {
        if ambiguous_candidates.contains(&index) {
            continue;
        }
        if let Some(key) = legacy_key(
            &candidate.file_name,
            candidate.size_bytes,
            Some(&candidate.modified_at),
        ) {
            candidate_legacy_counts.entry(key).or_default().push(index);
        }
    }
    for (key, old_indices) in &old_legacy_counts {
        let Some(candidate_indices) = candidate_legacy_counts.get(key) else {
            continue;
        };
        let remaining_old = old_indices
            .iter()
            .copied()
            .filter(|index| !claimed_old.contains(index))
            .collect::<Vec<_>>();
        let remaining_candidates = candidate_indices
            .iter()
            .copied()
            .filter(|index| !claimed_candidates.contains(index))
            .collect::<Vec<_>>();
        if remaining_old.is_empty() || remaining_candidates.is_empty() {
            continue;
        }
        if old_indices.len() == 1
            && candidate_indices.len() == 1
            && old_clips[remaining_old[0]].source_dir_id
                == candidates[remaining_candidates[0]].source_dir_id
        {
            let old_index = remaining_old[0];
            let candidate_index = remaining_candidates[0];
            claimed_old.insert(old_index);
            claimed_candidates.insert(candidate_index);
            matches.push(RelocationMatch {
                old_index,
                candidate_index,
                kind: MatchKind::LegacyFingerprint,
            });
        } else {
            conflicts.push(SourceRelocationConflict {
                code: "legacy-fingerprint-ambiguous".to_string(),
                message: "旧文件名、大小和修改时间指纹不唯一，未自动合并".to_string(),
                old_clip_ids: old_indices
                    .iter()
                    .map(|index| old_clips[*index].id.to_string())
                    .collect(),
                candidate_paths: candidate_indices
                    .iter()
                    .map(|index| candidates[*index].file_path.clone())
                    .collect(),
            });
        }
    }

    for (old_index, candidate_index, relative) in exact_mismatches {
        if claimed_old.contains(&old_index) || claimed_candidates.contains(&candidate_index) {
            continue;
        }
        conflicts.push(SourceRelocationConflict {
            code: "exact-path-fingerprint-mismatch".to_string(),
            message: format!(
                "相同相对路径的文件身份与大小/修改时间均不一致：{}",
                join_relative_components(new_root, &relative).display()
            ),
            old_clip_ids: vec![old_clips[old_index].id.to_string()],
            candidate_paths: vec![candidates[candidate_index].file_path.clone()],
        });
    }

    matches.sort_by_key(|matched| old_clips[matched.old_index].id);
    matches
}

fn validate_final_clip_paths(
    connection: &Connection,
    old_clips: &[OldClip],
    candidates: &[RelocationCandidate],
    matches: &[RelocationMatch],
    conflicts: &mut Vec<SourceRelocationConflict>,
) -> DbResult<()> {
    let matched_ids = matches
        .iter()
        .map(|matched| old_clips[matched.old_index].id)
        .collect::<HashSet<_>>();
    let mut target_owners = HashMap::<String, i64>::new();
    for matched in matches {
        let old = &old_clips[matched.old_index];
        let candidate = &candidates[matched.candidate_index];
        if let Some(other) = target_owners.insert(candidate.normalized_path.clone(), old.id) {
            conflicts.push(SourceRelocationConflict {
                code: "duplicate-final-clip-path".to_string(),
                message: format!("多个旧素材会映射到同一路径：{}", candidate.file_path),
                old_clip_ids: vec![other.to_string(), old.id.to_string()],
                candidate_paths: vec![candidate.file_path.clone()],
            });
        }
        let owner = connection
            .query_row(
                "SELECT id FROM clips WHERE normalized_path = ?1 LIMIT 1",
                params![candidate.normalized_path],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| readable_error("checking final clip path", error))?;
        if owner.is_some_and(|owner| owner != old.id && !matched_ids.contains(&owner)) {
            conflicts.push(SourceRelocationConflict {
                code: "final-clip-path-conflict".to_string(),
                message: format!("最终素材路径已被未受影响记录占用：{}", candidate.file_path),
                old_clip_ids: vec![old.id.to_string()],
                candidate_paths: vec![candidate.file_path.clone()],
            });
        }
    }
    Ok(())
}

fn expected_group_updates(
    sources: &[RelocationSource],
    old_clips: &[OldClip],
    candidates: &[RelocationCandidate],
    matches: &[RelocationMatch],
) -> usize {
    matches
        .iter()
        .filter(|matched| {
            let old = &old_clips[matched.old_index];
            let source = sources
                .iter()
                .find(|source| source.source.id == old.source_dir_id);
            let desired = source.and_then(|source| {
                desired_group_key(&candidates[matched.candidate_index], &source.new_path)
            });
            source.is_some_and(|source| source.source.source_kind == SourceKind::Aclos)
                && desired.as_deref() != old.clip_group_key.as_deref()
        })
        .count()
}

fn count_metadata_reference_updates(
    connection: &Connection,
    old_root: &Path,
    excluded_reference_roots: &[PathBuf],
    matched_old: &HashSet<usize>,
    old_clips: &[OldClip],
) -> DbResult<usize> {
    let matched_ids = matched_old
        .iter()
        .map(|index| old_clips[*index].id)
        .collect::<BTreeSet<_>>();
    let mut count = 0usize;
    if !matched_ids.is_empty() {
        let placeholders = sql_placeholders(matched_ids.len());
        let sql = format!("SELECT json_path FROM clip_metadata WHERE clip_id IN ({placeholders}) AND json_path IS NOT NULL");
        let values = matched_ids
            .iter()
            .copied()
            .map(Value::Integer)
            .collect::<Vec<_>>();
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| readable_error("preparing relocation metadata paths", error))?;
        let paths = statement
            .query_map(params_from_iter(values), |row| row.get::<_, String>(0))
            .map_err(|error| readable_error("querying relocation metadata paths", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading relocation metadata paths", error))?;
        count += paths
            .iter()
            .filter(|path| relative_components(Path::new(path), old_root).is_some())
            .count();
    }
    let mut statement = connection
        .prepare("SELECT package_path, thumb_path FROM match_snapshots WHERE package_path IS NOT NULL OR thumb_path IS NOT NULL")
        .map_err(|error| readable_error("preparing relocation snapshot paths", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| readable_error("querying relocation snapshot paths", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading relocation snapshot paths", error))?;
    for (package_path, thumb_path) in rows {
        count += [package_path, thumb_path]
            .into_iter()
            .flatten()
            .filter(|path| {
                let path = Path::new(path);
                relative_components(path, old_root).is_some()
                    && !path_is_within_any(path, excluded_reference_roots)
            })
            .count();
    }
    Ok(count)
}

fn apply_relocation_plan(
    transaction: &Transaction<'_>,
    plan: &RelocationPlan,
    identity_reader: &IdentityReader<'_>,
) -> DbResult<()> {
    // Recheck authorization objects after planning while the IMMEDIATE transaction owns the
    // writer lock. This is intentionally redundant with preview construction.
    let affected_ids = plan
        .sources
        .iter()
        .map(|source| source.source.id)
        .collect::<BTreeSet<_>>();
    let mut protected = Vec::new();
    add_protected_state_blockers(transaction, &affected_ids, &mut protected)?;
    if !protected.is_empty() {
        return Err(protected
            .into_iter()
            .map(|blocker| blocker.message)
            .collect::<Vec<_>>()
            .join("；"));
    }

    revalidate_matched_candidates(plan, identity_reader)?;

    let nonce = relocation_nonce();
    for source in &plan.sources {
        let placeholder = format!(
            "__valoframe_relocation__/{nonce}/source/{}",
            source.source.id
        );
        transaction
            .execute(
                "UPDATE source_dirs SET path = ?2, scan_root_path = ?3 WHERE id = ?1",
                params![
                    source.source.id,
                    placeholder,
                    format!("__valoframe_relocation__/{nonce}/root/{}", source.source.id)
                ],
            )
            .map_err(|error| readable_error("staging source relocation paths", error))?;
    }
    for matched in &plan.matches {
        let old = &plan.old_clips[matched.old_index];
        let placeholder = format!("__valoframe_relocation__/{nonce}/clip/{}", old.id);
        transaction
            .execute(
                "UPDATE clips SET file_path = ?2, normalized_path = ?3 WHERE id = ?1",
                params![old.id, placeholder, normalize_path(&placeholder)],
            )
            .map_err(|error| readable_error("staging clip relocation paths", error))?;
    }

    for source in &plan.sources {
        let new_source_path = stable_path_for_storage(&source.new_path.to_string_lossy());
        let new_root_path = stable_path_for_storage(&plan.new_root.to_string_lossy());
        transaction
            .execute(
                "UPDATE source_dirs SET path = ?2, scan_root_path = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![
                    source.source.id,
                    new_source_path,
                    new_root_path,
                ],
            )
            .map_err(|error| readable_error("writing relocated source paths", error))?;
    }

    let relocation_sources = plan
        .sources
        .iter()
        .map(|source| (source.source.id, source))
        .collect::<HashMap<_, _>>();
    let mut relocated_clip_ids = Vec::with_capacity(plan.matches.len());
    for matched in &plan.matches {
        let old = &plan.old_clips[matched.old_index];
        let candidate = &plan.candidates[matched.candidate_index];
        let candidate_file_path = stable_path_for_storage(&candidate.file_path);
        let candidate_normalized_path = normalize_path(&candidate_file_path);
        let (volume_serial, index_high, index_low) = identity_database_parts(candidate.identity);
        let source_relative_dir = candidate_relative_directory(candidate);
        let clip_group_id = if relocation_sources
            .get(&old.source_dir_id)
            .is_some_and(|source| source.source.source_kind == SourceKind::Aclos)
        {
            desired_group_key(candidate, &relocation_sources[&old.source_dir_id].new_path)
                .map(|key| ensure_group(transaction, old.source_dir_id, &key))
                .transpose()?
        } else {
            old.clip_group_id
        };
        let cover_path = old
            .cover_path
            .as_deref()
            .and_then(|path| {
                relocate_root_bound_path(Path::new(path), &plan.old_root, &plan.new_root)
                    .map(|path| stable_path_for_storage(&path.to_string_lossy()))
            })
            .or_else(|| {
                old.cover_path
                    .clone()
                    .map(|path| stable_path_for_storage(&path))
            });
        transaction
            .execute(
                "UPDATE clips
                 SET file_path = ?2,
                     normalized_path = ?3,
                     file_name = ?4,
                     size_bytes = ?5,
                     modified_at = ?6,
                     file_volume_serial = ?7,
                     file_index_high = ?8,
                     file_index_low = ?9,
                     source_relative_dir = ?10,
                     clip_group_id = ?11,
                     cover_path = ?12,
                     file_status = 'available',
                     last_seen_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![
                    old.id,
                    candidate_file_path,
                    candidate_normalized_path,
                    candidate.file_name,
                    candidate.size_bytes,
                    candidate.modified_at,
                    volume_serial,
                    index_high,
                    index_low,
                    source_relative_dir,
                    clip_group_id,
                    cover_path,
                ],
            )
            .map_err(|error| readable_error("writing relocated clip path", error))?;
        relocated_clip_ids.push(old.id);
    }

    let matched_ids = relocated_clip_ids.iter().copied().collect::<HashSet<_>>();
    for old in plan
        .old_clips
        .iter()
        .filter(|old| !matched_ids.contains(&old.id))
    {
        transaction
            .execute(
                "UPDATE clips SET file_status = 'missing', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![old.id],
            )
            .map_err(|error| readable_error("marking unmatched relocation clip missing", error))?;
    }

    rewrite_metadata_paths(transaction, plan, &matched_ids)?;
    rewrite_match_snapshot_paths(
        transaction,
        &plan.old_root,
        &plan.new_root,
        &plan.excluded_reference_roots,
    )?;
    let all_affected_clip_ids = plan.old_clips.iter().map(|old| old.id).collect::<Vec<_>>();
    thumbnails::reconcile_clip_thumbnails_unbounded_in_transaction(
        transaction,
        &all_affected_clip_ids,
        false,
    )?;
    Ok(())
}

fn revalidate_matched_candidates(
    plan: &RelocationPlan,
    identity_reader: &IdentityReader<'_>,
) -> DbResult<()> {
    let candidate_indices = plan
        .matches
        .iter()
        .map(|matched| matched.candidate_index)
        .collect::<BTreeSet<_>>();
    for index in candidate_indices {
        let candidate = &plan.candidates[index];
        let metadata_before = fs::symlink_metadata(&candidate.path).map_err(|error| {
            format!(
                "重新定位提交前无法复验候选 {}：{error}",
                candidate.path.display()
            )
        })?;
        if !metadata_before.is_file() || metadata_is_reparse_point(&metadata_before) {
            return Err(format!(
                "重新定位候选已变为非普通文件或 reparse point：{}",
                candidate.path.display()
            ));
        }
        let canonical = candidate.path.canonicalize().map_err(|error| {
            format!(
                "重新定位提交前无法规范化候选 {}：{error}",
                candidate.path.display()
            )
        })?;
        if canonical != candidate.path || !canonical.starts_with(&plan.new_root) {
            return Err(format!(
                "重新定位候选路径在预览后发生变化或越界：{}",
                candidate.path.display()
            ));
        }
        let current_identity = identity_reader(&candidate.path);
        let metadata_after = fs::symlink_metadata(&candidate.path).map_err(|error| {
            format!(
                "读取文件身份后无法复验候选 {}：{error}",
                candidate.path.display()
            )
        })?;
        let current_size = i64::try_from(metadata_after.len())
            .map_err(|_| format!("重新定位候选大小超出范围：{}", candidate.path.display()))?;
        let current_modified_time = metadata_after.modified().map_err(|error| {
            format!(
                "重新定位提交前无法读取候选修改时间 {}：{error}",
                candidate.path.display()
            )
        })?;
        if !metadata_after.is_file()
            || metadata_is_reparse_point(&metadata_after)
            || current_size != candidate.size_bytes
            || current_modified_time != candidate.modified_time
            || current_identity != candidate.identity
        {
            return Err(format!(
                "重新定位候选在预览后发生变化，请重新预览：{}",
                candidate.path.display()
            ));
        }
    }
    Ok(())
}

fn ensure_group(transaction: &Transaction<'_>, source_dir_id: i64, key: &str) -> DbResult<i64> {
    transaction
        .execute(
            "INSERT INTO clip_groups (source_dir_id, group_key, display_name)
             VALUES (?1, ?2, ?2)
             ON CONFLICT(source_dir_id, group_key) DO UPDATE SET
                 display_name = excluded.display_name,
                 updated_at = CURRENT_TIMESTAMP",
            params![source_dir_id, key],
        )
        .map_err(|error| readable_error("ensuring relocated clip group", error))?;
    transaction
        .query_row(
            "SELECT id FROM clip_groups WHERE source_dir_id = ?1 AND group_key = ?2",
            params![source_dir_id, key],
            |row| row.get(0),
        )
        .map_err(|error| readable_error("reading relocated clip group", error))
}

fn rewrite_metadata_paths(
    transaction: &Transaction<'_>,
    plan: &RelocationPlan,
    matched_ids: &HashSet<i64>,
) -> DbResult<()> {
    for clip_id in matched_ids {
        let json_path = transaction
            .query_row(
                "SELECT json_path FROM clip_metadata WHERE clip_id = ?1",
                params![clip_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| readable_error("reading relocated metadata path", error))?
            .flatten();
        let Some(path) = json_path else { continue };
        let Some(relocated) =
            relocate_root_bound_path(Path::new(&path), &plan.old_root, &plan.new_root)
        else {
            continue;
        };
        transaction
            .execute(
                "UPDATE clip_metadata SET json_path = ?2, updated_at = CURRENT_TIMESTAMP WHERE clip_id = ?1",
                params![clip_id, stable_path_for_storage(&relocated.to_string_lossy())],
            )
            .map_err(|error| readable_error("writing relocated metadata path", error))?;
    }
    Ok(())
}

fn rewrite_match_snapshot_paths(
    transaction: &Transaction<'_>,
    old_root: &Path,
    new_root: &Path,
    excluded_reference_roots: &[PathBuf],
) -> DbResult<()> {
    let mut statement = transaction
        .prepare("SELECT id, package_path, thumb_path FROM match_snapshots WHERE package_path IS NOT NULL OR thumb_path IS NOT NULL")
        .map_err(|error| readable_error("preparing relocated snapshot references", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| readable_error("querying relocated snapshot references", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading relocated snapshot references", error))?;
    drop(statement);
    for (id, package_path, thumb_path) in rows {
        let new_package = package_path
            .as_deref()
            .filter(|path| !path_is_within_any(Path::new(path), excluded_reference_roots))
            .and_then(|path| {
                relocate_root_bound_path(Path::new(path), old_root, new_root)
                    .map(|path| stable_path_for_storage(&path.to_string_lossy()))
            })
            .or_else(|| package_path.clone());
        let new_thumb = thumb_path
            .as_deref()
            .filter(|path| !path_is_within_any(Path::new(path), excluded_reference_roots))
            .and_then(|path| {
                relocate_root_bound_path(Path::new(path), old_root, new_root)
                    .map(|path| stable_path_for_storage(&path.to_string_lossy()))
            })
            .or_else(|| thumb_path.clone());
        if new_package == package_path && new_thumb == thumb_path {
            continue;
        }
        transaction
            .execute(
                "UPDATE match_snapshots SET package_path = ?2, thumb_path = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![id, new_package, new_thumb],
            )
            .map_err(|error| readable_error("writing relocated snapshot references", error))?;
    }
    Ok(())
}

fn path_is_within_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| relative_components(path, root).is_some())
}

fn assert_transaction_integrity(transaction: &Transaction<'_>) -> DbResult<()> {
    let placeholder_count: i64 = transaction
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM source_dirs WHERE path LIKE '__valoframe_relocation__/%' OR scan_root_path LIKE '__valoframe_relocation__/%') +
                (SELECT COUNT(*) FROM clips WHERE file_path LIKE '__valoframe_relocation__/%' OR normalized_path LIKE '__valoframe_relocation__/%')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| readable_error("checking relocation placeholders", error))?;
    if placeholder_count != 0 {
        return Err("source relocation left transaction-only placeholders behind".to_string());
    }
    let violation = transaction
        .query_row("PRAGMA foreign_key_check", [], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()
        .map_err(|error| readable_error("checking relocation foreign keys", error))?;
    if let Some((table, row_id)) = violation {
        return Err(format!(
            "source relocation foreign key violation in {table} row {row_id}"
        ));
    }
    Ok(())
}

fn relocation_rejection_message(preview: &ScanSourceRelocationPreview) -> String {
    let mut messages = preview
        .blockers
        .iter()
        .map(|blocker| blocker.message.clone())
        .collect::<Vec<_>>();
    messages.extend(
        preview
            .conflicts
            .iter()
            .map(|conflict| conflict.message.clone()),
    );
    if messages.is_empty() {
        "来源重新定位未通过安全验证".to_string()
    } else {
        messages.join("；")
    }
}

fn desired_group_key(candidate: &RelocationCandidate, source_path: &Path) -> Option<String> {
    let relative = relative_components(&candidate.path, source_path)?;
    if relative.len() < 2 {
        return None;
    }
    relative
        .get(relative.len() - 2)
        .cloned()
        .filter(|name| !name.trim().is_empty())
}

fn candidate_relative_directory(candidate: &RelocationCandidate) -> String {
    let end = candidate.relative_components.len().saturating_sub(1);
    candidate.relative_components[..end].join("/")
}

fn relocate_root_bound_path(path: &Path, old_root: &Path, new_root: &Path) -> Option<PathBuf> {
    let relative = relative_components(path, old_root)?;
    Some(join_relative_components(new_root, &relative))
}

fn identity_database_parts(
    identity: Option<StableFileIdentity>,
) -> (Option<i64>, Option<i64>, Option<i64>) {
    let Some(identity) = identity else {
        return (None, None, None);
    };
    let (volume, high, low) = identity.database_parts();
    (Some(volume), Some(high), Some(low))
}

fn identity_key(identity: StableFileIdentity) -> (u32, u32, u32) {
    (
        identity.volume_serial,
        identity.file_index_high,
        identity.file_index_low,
    )
}

fn legacy_key(
    file_name: &str,
    size_bytes: i64,
    modified_at: Option<&str>,
) -> Option<(String, i64, String)> {
    Some((
        file_name.trim().to_lowercase(),
        size_bytes,
        modified_at?.trim().to_string(),
    ))
}

fn has_mp4_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

fn format_system_time(time: SystemTime) -> String {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    false
}

/// Returns path components after `root` using case-insensitive Windows semantics while retaining
/// the original path's spelling. The textual normalization also removes Win32 verbatim prefixes,
/// allowing a stored `C:\\x` root to compare with a canonical `\\?\\C:\\x` path.
fn relative_components(path: &Path, root: &Path) -> Option<Vec<String>> {
    if has_parent_traversal(path) || has_parent_traversal(root) {
        return None;
    }
    let path_components = comparable_components(path);
    let root_components = comparable_components(root);
    if root_components.is_empty() || path_components.len() < root_components.len() {
        return None;
    }
    if !root_components
        .iter()
        .zip(&path_components)
        .all(|(root, path)| root.eq_ignore_ascii_case(path))
    {
        return None;
    }
    Some(path_components[root_components.len()..].to_vec())
}

fn has_parent_traversal(path: &Path) -> bool {
    path.display()
        .to_string()
        .replace('\\', "/")
        .split('/')
        .any(|component| component == "..")
}

fn comparable_components(path: &Path) -> Vec<String> {
    let mut value = path.display().to_string().replace('\\', "/");
    if value
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
    {
        value = format!("//{}", &value[8..]);
    } else if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/"))
    {
        value = value[4..].to_string();
    }
    value
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .map(str::to_string)
        .collect()
}

fn join_relative_components(root: &Path, components: &[String]) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in components {
        path.push(component);
    }
    path
}

fn relative_key(components: &[String]) -> String {
    components
        .iter()
        .map(|component| component.to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalized_path_key(path: &str) -> String {
    comparable_components(Path::new(path))
        .into_iter()
        .map(|component| component.to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// Canonicalizes the longest existing prefix before comparing paths. This expands Win32 short
/// names even when the leaf is currently missing, which is common for disconnected sources.
fn filesystem_path_key(path: &Path) -> String {
    let mut cursor = Some(path);
    let mut suffix = Vec::new();
    while let Some(current) = cursor {
        if let Ok(mut canonical) = current.canonicalize() {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return normalized_path_key(&canonical.display().to_string());
        }
        if let Some(name) = current.file_name() {
            suffix.push(name.to_os_string());
        }
        cursor = current.parent();
    }
    normalized_path_key(&path.display().to_string())
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn relocation_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, ClipInput, SourceDirInput, SourceProfileInput};
    use rusqlite::params;

    struct Fixture {
        connection: Connection,
        root: PathBuf,
        old_root: PathBuf,
        new_root: PathBuf,
        source_id: i64,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "vhm-relocation-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let old_root = root.join("old-root");
            let new_root = root.join("new-root");
            fs::create_dir_all(&new_root).expect("new root should exist");
            let connection = Connection::open_in_memory().expect("database should open");
            db::initialize_schema(&connection).expect("schema should initialize");
            let source = db::register_source_dir(
                &connection,
                SourceDirInput {
                    path: &old_root.display().to_string(),
                    name: "Relocation Fixture",
                },
                SourceProfileInput {
                    source_kind: SourceKind::Generic,
                    scan_mode: SourceKind::Generic.default_scan_mode(),
                    scan_root_path: &old_root.display().to_string(),
                },
                true,
            )
            .expect("source should register");
            Self {
                connection,
                root,
                old_root,
                new_root,
                source_id: source.id,
            }
        }

        fn candidate(&self, relative: &str, bytes: &[u8]) -> (PathBuf, String) {
            let path = self.new_root.join(relative);
            fs::create_dir_all(path.parent().expect("candidate should have parent"))
                .expect("candidate parent should exist");
            fs::write(&path, bytes).expect("candidate should be written");
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .map(format_system_time)
                .expect("candidate modified time should be readable");
            (path, modified)
        }

        fn insert_old_clip(
            &self,
            relative: &str,
            size_bytes: i64,
            modified_at: &str,
            identity: Option<StableFileIdentity>,
            cover_path: Option<&str>,
        ) -> i64 {
            let path = self.old_root.join(relative).display().to_string();
            db::upsert_scanned_clip_with_file_identity(
                &self.connection,
                ClipInput {
                    source_dir_id: self.source_id,
                    clip_group_id: None,
                    video_path: &path,
                    file_name: Path::new(relative).file_name().unwrap().to_str().unwrap(),
                    file_size: size_bytes,
                    modified_at: Some(modified_at),
                    duration_ms: Some(30_000),
                    recorded_at: Some("2026-08-01T10:00:00Z"),
                    cover_path,
                    cover_source: "missing",
                },
                identity,
            )
            .expect("old clip should insert")
            .clip
            .id
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn configure_shared_logical_sources(fixture: &Fixture) -> db::SourceDir {
        fixture
            .connection
            .execute(
                "UPDATE source_dirs SET path = ?2 WHERE id = ?1",
                params![
                    fixture.source_id,
                    fixture.old_root.join("account-a").display().to_string()
                ],
            )
            .unwrap();
        let second = db::register_source_dir(
            &fixture.connection,
            SourceDirInput {
                path: &fixture.old_root.join("account-b").display().to_string(),
                name: "Account B",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &fixture.old_root.display().to_string(),
            },
            true,
        )
        .unwrap();
        fs::create_dir_all(fixture.new_root.join("account-a")).unwrap();
        fs::create_dir_all(fixture.new_root.join("account-b")).unwrap();
        second
    }

    const IDENTITY_ONE: StableFileIdentity = StableFileIdentity {
        volume_serial: 11,
        file_index_high: 12,
        file_index_low: 13,
    };
    const IDENTITY_TWO: StableFileIdentity = StableFileIdentity {
        volume_serial: 21,
        file_index_high: 22,
        file_index_low: 23,
    };

    #[test]
    fn exact_relocation_preserves_user_state_rewrites_bound_paths_and_invalidates_thumbnail() {
        let fixture = Fixture::new("exact-state");
        let (candidate, modified) = fixture.candidate("nested/clip.mp4", b"candidate-video");
        let old_cover = fixture
            .old_root
            .join("nested/cover.jpg")
            .display()
            .to_string();
        let clip_id = fixture.insert_old_clip(
            "nested/clip.mp4",
            15,
            &modified,
            Some(IDENTITY_ONE),
            Some(&old_cover),
        );
        let (_, escape_modified) = fixture.candidate("escape/clip-escape.mp4", b"escape");
        let escaped_cover = fixture
            .old_root
            .join("../outside-cover.jpg")
            .display()
            .to_string();
        let escaped_json = fixture
            .old_root
            .join("../outside-metadata.json")
            .display()
            .to_string();
        let escaped_clip = fixture.insert_old_clip(
            "escape/clip-escape.mp4",
            6,
            &escape_modified,
            None,
            Some(&escaped_cover),
        );
        fixture
            .connection
            .execute(
                "UPDATE clip_metadata SET json_path = ?2 WHERE clip_id = ?1",
                params![escaped_clip, escaped_json],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "UPDATE source_dirs
                 SET status = 'unavailable', last_error = 'drive missing',
                     last_scanned_at = '2026-08-01T00:00:00Z'
                 WHERE id = ?1",
                params![fixture.source_id],
            )
            .unwrap();
        let escaped_snapshot_package = fixture
            .old_root
            .join("../outside-snapshot.json")
            .display()
            .to_string();
        let escaped_snapshot_thumb = fixture
            .old_root
            .join("../outside-snapshot.jpg")
            .display()
            .to_string();
        fixture
            .connection
            .execute(
                "INSERT INTO match_snapshots (snapshot_id, account_id, package_path, thumb_path, raw_json)
                 VALUES ('snapshot-escape', 'account-1', ?1, ?2, '{}')",
                params![escaped_snapshot_package, escaped_snapshot_thumb],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "UPDATE clips
                 SET is_favorite = 1, review_decision = 'liked', reviewed_at = '2026-08-02T00:00:00Z',
                     note = 'keep me'
                 WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        fixture
            .connection
            .execute("INSERT INTO tags (name) VALUES ('复盘')", [])
            .unwrap();
        let tag_id = fixture.connection.last_insert_rowid();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_tags (clip_id, tag_id) VALUES (?1, ?2)",
                params![clip_id, tag_id],
            )
            .unwrap();
        let old_json = fixture
            .old_root
            .join("nested/meta.json")
            .display()
            .to_string();
        fixture
            .connection
            .execute(
                "UPDATE clip_metadata
                 SET metadata_status = 'parsed', json_path = ?2, extra_json = '{\"keep\":true}'
                 WHERE clip_id = ?1",
                params![clip_id, old_json],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_events (clip_id, event_key, event_type, raw_json)
                 VALUES (?1, 'event-1', 'kill', '{\"raw\":true}')",
                params![clip_id],
            )
            .unwrap();
        let package_path = fixture
            .old_root
            .join("snapshots/a.json")
            .display()
            .to_string();
        let thumb_path = fixture
            .old_root
            .join("snapshots/a.jpg")
            .display()
            .to_string();
        fixture
            .connection
            .execute(
                "INSERT INTO match_snapshots (snapshot_id, account_id, package_path, thumb_path, raw_json)
                 VALUES ('snapshot-1', 'account-1', ?1, ?2, '{\"unchanged\":true}')",
                params![package_path, thumb_path],
            )
            .unwrap();

        db::ensure_clip_thumbnails(&fixture.connection, &[clip_id]).unwrap();
        let old_job = db::claim_next_thumbnail_job(&fixture.connection, "2026-08-09T00:00:00Z")
            .unwrap()
            .expect("old thumbnail job should claim");

        let changes_before_preview = fixture.connection.total_changes();
        let preview = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert_eq!(fixture.connection.total_changes(), changes_before_preview);
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.exact_path_match_count, 2);
        assert_eq!(preview.expected_cover_update_count, 1);
        assert_eq!(preview.expected_metadata_reference_update_count, 3);

        let committed = commit_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert_eq!(committed.relocated_clip_count, 2);
        let state = fixture
            .connection
            .query_row(
                "SELECT source_dirs.path, source_dirs.scan_root_path, source_dirs.status,
                        source_dirs.last_error, source_dirs.last_scanned_at,
                        clips.id, clips.file_path, clips.source_relative_dir, clips.is_favorite,
                        clips.review_decision, clips.note, clips.cover_path, clips.file_status
                 FROM source_dirs JOIN clips ON clips.source_dir_id = source_dirs.id
                 WHERE clips.id = ?1",
                params![clip_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, String>(12)?,
                    ))
                },
            )
            .unwrap();
        let canonical_new_root = fixture.new_root.canonicalize().unwrap();
        let stored_new_root = stable_path_for_storage(&canonical_new_root.to_string_lossy());
        assert_eq!(state.0, stored_new_root);
        assert_eq!(state.1, stored_new_root);
        assert!(!db::has_windows_verbatim_prefix(&state.0));
        assert!(!db::has_windows_verbatim_prefix(&state.1));
        assert_eq!(state.2, "unavailable");
        assert_eq!(state.3.as_deref(), Some("drive missing"));
        assert_eq!(state.4.as_deref(), Some("2026-08-01T00:00:00Z"));
        assert_eq!(state.5, clip_id);
        assert_eq!(
            state.6,
            stable_path_for_storage(&candidate.canonicalize().unwrap().to_string_lossy())
        );
        assert!(!db::has_windows_verbatim_prefix(&state.6));
        assert_eq!(state.7, "nested");
        assert_eq!(
            (state.8, state.9.as_str(), state.10.as_deref()),
            (1, "liked", Some("keep me"))
        );
        assert_eq!(
            state.11.as_deref().map(normalize_path),
            Some(normalize_path(
                &canonical_new_root
                    .join("nested/cover.jpg")
                    .display()
                    .to_string()
            ))
        );
        assert_eq!(state.12, "available");
        assert!(!state
            .11
            .as_deref()
            .is_some_and(db::has_windows_verbatim_prefix));
        assert_eq!(
            fixture
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM clip_tags WHERE clip_id = ?1",
                    params![clip_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            fixture
                .connection
                .query_row(
                    "SELECT extra_json FROM clip_metadata WHERE clip_id = ?1",
                    params![clip_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "{\"keep\":true}"
        );
        assert_eq!(
            fixture
                .connection
                .query_row(
                    "SELECT raw_json FROM clip_events WHERE clip_id = ?1",
                    params![clip_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "{\"raw\":true}"
        );
        let (new_package, new_thumb, raw): (String, String, String) = fixture.connection.query_row(
            "SELECT package_path, thumb_path, raw_json FROM match_snapshots WHERE snapshot_id = 'snapshot-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert!(relative_components(Path::new(&new_package), &canonical_new_root).is_some());
        assert!(relative_components(Path::new(&new_thumb), &canonical_new_root).is_some());
        assert!(!db::has_windows_verbatim_prefix(&new_package));
        assert!(!db::has_windows_verbatim_prefix(&new_thumb));
        assert_eq!(raw, "{\"unchanged\":true}");
        let (unchanged_cover, unchanged_json): (Option<String>, Option<String>) = fixture
            .connection
            .query_row(
                "SELECT clips.cover_path, clip_metadata.json_path
                 FROM clips JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                 WHERE clips.id = ?1",
                params![escaped_clip],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(unchanged_cover.as_deref(), Some(escaped_cover.as_str()));
        assert_eq!(unchanged_json.as_deref(), Some(escaped_json.as_str()));
        let (unchanged_package, unchanged_thumb): (String, String) = fixture
            .connection
            .query_row(
                "SELECT package_path, thumb_path FROM match_snapshots WHERE snapshot_id = 'snapshot-escape'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(unchanged_package, escaped_snapshot_package);
        assert_eq!(unchanged_thumb, escaped_snapshot_thumb);
        let stale_cache = format!("{}-{}.jpg", old_job.clip_id, old_job.fingerprint);
        assert!(!db::complete_thumbnail_job_if_current(
            &fixture.connection,
            &old_job,
            &stale_cache,
            100,
            &old_job.fingerprint,
        )
        .unwrap());
    }

    #[test]
    fn renamed_paths_use_unique_identity_then_legacy_fingerprint() {
        let fixture = Fixture::new("identity-legacy");
        let (identity_path, identity_modified) =
            fixture.candidate("renamed/identity-new.mp4", b"identity-new");
        let (legacy_path, legacy_modified) = fixture.candidate("renamed/legacy.mp4", b"legacy");
        let identity_clip = fixture.insert_old_clip(
            "original/identity-old.mp4",
            999,
            "1",
            Some(IDENTITY_ONE),
            None,
        );
        let legacy_clip =
            fixture.insert_old_clip("original/legacy.mp4", 6, &legacy_modified, None, None);
        let reader = |path: &Path| {
            (path.file_name().and_then(|name| name.to_str()) == Some("identity-new.mp4"))
                .then_some(IDENTITY_ONE)
        };
        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap()
        .preview;
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.identity_match_count, 1);
        assert_eq!(preview.legacy_fingerprint_match_count, 1);
        assert_eq!(preview.exact_path_match_count, 0);
        commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap();
        for (clip_id, expected) in [(identity_clip, identity_path), (legacy_clip, legacy_path)] {
            let actual: String = fixture
                .connection
                .query_row(
                    "SELECT file_path FROM clips WHERE id = ?1",
                    params![clip_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                normalize_path(&actual),
                normalize_path(&expected.canonicalize().unwrap().display().to_string())
            );
        }
        assert_ne!(identity_modified, "1");
    }

    #[test]
    fn identity_ambiguity_and_zero_trust_fail_closed_without_writes() {
        let fixture = Fixture::new("ambiguity");
        fixture.candidate("a/duplicate.mp4", b"a");
        fixture.candidate("b/duplicate.mp4", b"bb");
        let clip_id =
            fixture.insert_old_clip("old/duplicate.mp4", 9, "9", Some(IDENTITY_ONE), None);
        let reader = |_path: &Path| Some(IDENTITY_ONE);
        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap()
        .preview;
        assert!(!preview.can_relocate);
        assert!(preview
            .conflicts
            .iter()
            .any(|conflict| conflict.code == "identity-ambiguous"));
        assert!(commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .is_err());
        let path: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![clip_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(relative_components(Path::new(&path), &fixture.old_root).is_some());

        let empty = Fixture::new("zero");
        empty.insert_old_clip("gone.mp4", 4, "4", None, None);
        let zero =
            preview_scan_source_relocation(&empty.connection, empty.source_id, &empty.new_root)
                .unwrap();
        assert!(!zero.can_relocate);
        assert!(zero
            .blockers
            .iter()
            .any(|blocker| blocker.code == "zero-trusted-matches"));
    }

    #[test]
    fn missing_or_file_new_root_fails_closed_without_database_writes() {
        let fixture = Fixture::new("invalid-root-zero-writes");
        let clip_id =
            fixture.insert_old_clip("nested/clip.mp4", 4, "2026-08-01T00:00:00Z", None, None);
        fixture
            .connection
            .execute(
                "UPDATE source_dirs SET last_scanned_at = '2026-08-01T00:00:00Z' WHERE id = ?1",
                params![fixture.source_id],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "UPDATE clips
                 SET is_favorite = 1, review_decision = 'liked', note = 'unchanged'
                 WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();

        let missing_root = fixture.root.join("does-not-exist");
        let file_root = fixture.root.join("ordinary-file.mp4");
        fs::write(&file_root, b"not a directory").unwrap();
        let expected_state: (String, String, Option<String>, i64, String, Option<String>) = fixture
            .connection
            .query_row(
                "SELECT source_dirs.scan_root_path, clips.file_path,
                        source_dirs.last_scanned_at, clips.is_favorite,
                        clips.review_decision, clips.note
                 FROM source_dirs JOIN clips ON clips.source_dir_id = source_dirs.id
                 WHERE clips.id = ?1",
                params![clip_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();

        for (invalid_root, expected_error_fragment) in [
            (&missing_root, "无法验证新来源目录"),
            (&file_root, "新来源根必须是普通目录"),
        ] {
            let changes_before_preview = fixture.connection.total_changes();
            let preview = preview_scan_source_relocation(
                &fixture.connection,
                fixture.source_id,
                invalid_root,
            )
            .expect("invalid roots should produce a fail-closed preview");
            assert!(!preview.can_relocate, "{preview:#?}");
            assert!(preview
                .blockers
                .iter()
                .any(|blocker| blocker.code == "invalid-new-root"));
            assert_eq!(fixture.connection.total_changes(), changes_before_preview);

            let changes_before_commit = fixture.connection.total_changes();
            let error =
                commit_scan_source_relocation(&fixture.connection, fixture.source_id, invalid_root)
                    .expect_err("invalid roots must reject commit");
            assert!(error.contains(expected_error_fragment), "{error}");
            assert_eq!(fixture.connection.total_changes(), changes_before_commit);

            let actual_state = fixture
                .connection
                .query_row(
                    "SELECT source_dirs.scan_root_path, clips.file_path,
                            source_dirs.last_scanned_at, clips.is_favorite,
                            clips.review_decision, clips.note
                     FROM source_dirs JOIN clips ON clips.source_dir_id = source_dirs.id
                     WHERE clips.id = ?1",
                    params![clip_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(actual_state, expected_state);
        }
    }

    #[test]
    fn case_equivalent_candidate_paths_are_ambiguous_before_matching() {
        let old_root = PathBuf::from("old-root");
        let new_root = PathBuf::from("new-root");
        let old = OldClip {
            id: 1,
            source_dir_id: 7,
            clip_group_id: None,
            clip_group_key: None,
            file_path: old_root.join("Clip.mp4").display().to_string(),
            file_name: "Clip.mp4".to_string(),
            size_bytes: 4,
            modified_at: Some("1".to_string()),
            identity: None,
            cover_path: None,
        };
        let candidates = ["Clip.mp4", "clip.mp4"]
            .into_iter()
            .map(|name| {
                let path = new_root.join(name);
                RelocationCandidate {
                    source_dir_id: 7,
                    path: path.clone(),
                    file_path: path.display().to_string(),
                    normalized_path: normalize_path(&path.display().to_string()),
                    relative_components: vec![name.to_string()],
                    relative_key: name.to_lowercase(),
                    file_name: name.to_string(),
                    size_bytes: 4,
                    modified_at: "1".to_string(),
                    modified_time: UNIX_EPOCH,
                    identity: None,
                }
            })
            .collect::<Vec<_>>();
        let mut conflicts = Vec::new();

        let matches = match_candidates(&old_root, &new_root, &[old], &candidates, &mut conflicts);

        assert!(matches.is_empty());
        assert!(conflicts.iter().any(|conflict| {
            conflict.code == "candidate-normalized-path-ambiguous"
                && conflict.candidate_paths.len() == 2
        }));
    }

    #[test]
    fn two_phase_placeholders_support_identity_swaps() {
        let fixture = Fixture::new("swap");
        let (candidate_a, modified_a) = fixture.candidate("a.mp4", b"BBBBBBBB");
        let (candidate_b, modified_b) = fixture.candidate("b.mp4", b"AAA");
        let old_a = fixture.insert_old_clip("a.mp4", 3, &modified_b, Some(IDENTITY_ONE), None);
        let old_b = fixture.insert_old_clip("b.mp4", 8, &modified_a, Some(IDENTITY_TWO), None);
        let reader = |path: &Path| match path.file_name().and_then(|name| name.to_str()) {
            Some("a.mp4") => Some(IDENTITY_TWO),
            Some("b.mp4") => Some(IDENTITY_ONE),
            _ => None,
        };
        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap()
        .preview;
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.identity_match_count, 2);
        commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap();
        let path_a: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![old_a],
                |row| row.get(0),
            )
            .unwrap();
        let path_b: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![old_b],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_path(&path_a),
            normalize_path(&candidate_b.canonicalize().unwrap().display().to_string())
        );
        assert_eq!(
            normalize_path(&path_b),
            normalize_path(&candidate_a.canonicalize().unwrap().display().to_string())
        );
        let placeholders: i64 = fixture
            .connection
            .query_row(
                "SELECT COUNT(*) FROM clips WHERE file_path LIKE '__valoframe_relocation__/%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placeholders, 0);
    }

    #[cfg(windows)]
    #[test]
    fn case_only_root_relocation_preserves_clip_id_and_user_state() {
        let fixture = Fixture::new("case-only-root");
        let original_clip_path = fixture.old_root.join("nested/clip.mp4");
        fs::create_dir_all(original_clip_path.parent().unwrap()).unwrap();
        fs::write(&original_clip_path, b"case-only-root-video").unwrap();
        let modified = fs::metadata(&original_clip_path)
            .and_then(|metadata| metadata.modified())
            .map(format_system_time)
            .unwrap();
        let clip_id = fixture.insert_old_clip(
            "nested/clip.mp4",
            i64::try_from(b"case-only-root-video".len()).unwrap(),
            &modified,
            read_stable_file_snapshot(&original_clip_path)
                .ok()
                .and_then(|snapshot| snapshot.identity),
            None,
        );
        fixture
            .connection
            .execute(
                "UPDATE clips
                 SET is_favorite = 1, review_decision = 'liked',
                     reviewed_at = '2026-08-02T00:00:00Z', note = 'keep case state'
                 WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        fixture
            .connection
            .execute("INSERT INTO tags (name) VALUES ('case-state')", [])
            .unwrap();
        let tag_id = fixture.connection.last_insert_rowid();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_tags (clip_id, tag_id) VALUES (?1, ?2)",
                params![clip_id, tag_id],
            )
            .unwrap();

        // Windows may treat a direct case-only rename as a no-op. Rename through a distinct
        // intermediate directory so the on-disk casing is deterministically changed first.
        let intermediate_root = fixture.root.join("case-only-intermediate");
        let case_only_root = fixture.root.join("OLD-ROOT");
        fs::rename(&fixture.old_root, &intermediate_root).unwrap();
        fs::rename(&intermediate_root, &case_only_root).unwrap();

        let preview =
            preview_scan_source_relocation(&fixture.connection, fixture.source_id, &case_only_root)
                .unwrap();
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.exact_path_match_count, 1);
        assert_eq!(preview.expected_clip_update_count, 1);

        let committed =
            commit_scan_source_relocation(&fixture.connection, fixture.source_id, &case_only_root)
                .unwrap();
        assert_eq!(committed.relocated_clip_count, 1);

        let state: (
            String,
            i64,
            String,
            i64,
            String,
            Option<String>,
            Option<String>,
            i64,
        ) = fixture
            .connection
            .query_row(
                "SELECT source_dirs.scan_root_path, clips.id, clips.file_path,
                        clips.is_favorite, clips.review_decision, clips.reviewed_at,
                        clips.note,
                        (SELECT COUNT(*) FROM clip_tags WHERE clip_tags.clip_id = clips.id)
                 FROM source_dirs JOIN clips ON clips.source_dir_id = source_dirs.id
                 WHERE clips.id = ?1",
                params![clip_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        let canonical_case_only_root = case_only_root.canonicalize().unwrap();
        assert_eq!(
            state.0,
            stable_path_for_storage(&canonical_case_only_root.to_string_lossy())
        );
        assert_eq!(
            Path::new(&state.0)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("OLD-ROOT")
        );
        assert_eq!(state.1, clip_id);
        assert_eq!(
            normalize_path(&state.2),
            normalize_path(
                &canonical_case_only_root
                    .join("nested/clip.mp4")
                    .display()
                    .to_string()
            )
        );
        assert_eq!(state.3, 1);
        assert_eq!(state.4, "liked");
        assert_eq!(state.5.as_deref(), Some("2026-08-02T00:00:00Z"));
        assert_eq!(state.6.as_deref(), Some("keep case state"));
        assert_eq!(state.7, 1);
    }

    #[test]
    fn relocation_freshness_waits_for_a_completed_follow_up_sync() {
        let fixture = Fixture::new("follow-up-freshness");
        let (_, modified) = fixture.candidate("clip.mp4", b"clip");
        fixture.insert_old_clip("clip.mp4", 4, &modified, None, None);
        fixture
            .connection
            .execute(
                "UPDATE source_dirs SET last_scanned_at = '2026-08-01T00:00:00Z' WHERE id = ?1",
                params![fixture.source_id],
            )
            .unwrap();

        commit_scan_source_relocation(&fixture.connection, fixture.source_id, &fixture.new_root)
            .unwrap();
        let freshness_after_commit =
            db::find_source_dir_by_id(&fixture.connection, fixture.source_id)
                .unwrap()
                .last_scanned_at;
        assert_eq!(
            freshness_after_commit.as_deref(),
            Some("2026-08-01T00:00:00Z"),
            "relocation commit itself must not claim a successful scan",
        );

        // `commands::sources::sync_selected_sources_with_parts` delegates its relocation
        // follow-up job to this same scanner entry point. Keeping the AppHandle/event shell out
        // of this repository test makes the freshness contract deterministic.
        let cancellation = std::sync::atomic::AtomicBool::new(true);
        let cancelled = crate::scanner::sync_scan_sources_with_progress_and_cancel(
            &fixture.connection,
            &[fixture.source_id],
            "relocation-follow-up-cancelled",
            &cancellation,
            |_| {},
        )
        .unwrap();
        assert_eq!(
            cancelled.status,
            crate::scanner::ScanExecutionStatus::Cancelled
        );
        assert_eq!(
            db::find_source_dir_by_id(&fixture.connection, fixture.source_id)
                .unwrap()
                .last_scanned_at,
            freshness_after_commit,
            "a cancelled follow-up must not refresh scan freshness",
        );

        cancellation.store(false, std::sync::atomic::Ordering::Release);
        let completed = crate::scanner::sync_scan_sources_with_progress_and_cancel(
            &fixture.connection,
            &[fixture.source_id],
            "relocation-follow-up-completed",
            &cancellation,
            |_| {},
        )
        .unwrap();
        assert_eq!(
            completed.status,
            crate::scanner::ScanExecutionStatus::Completed
        );
        let freshness_after_completed =
            db::find_source_dir_by_id(&fixture.connection, fixture.source_id)
                .unwrap()
                .last_scanned_at;
        assert!(freshness_after_completed.is_some());
        assert_ne!(freshness_after_completed, freshness_after_commit);
    }

    #[test]
    fn overlap_final_path_conflict_and_protected_delete_state_block_commit() {
        let fixture = Fixture::new("guards");
        let (_, modified) = fixture.candidate("clip.mp4", b"clip");
        let clip_id = fixture.insert_old_clip("clip.mp4", 4, &modified, None, None);
        let other_root = fixture.root.join("other");
        fs::create_dir_all(&other_root).unwrap();
        db::register_source_dir(
            &fixture.connection,
            SourceDirInput {
                path: &other_root.display().to_string(),
                name: "Other",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &fixture.new_root.join("nested").display().to_string(),
            },
            true,
        )
        .unwrap();
        let overlap = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert!(!overlap.can_relocate, "{overlap:#?}");
        assert!(overlap
            .blockers
            .iter()
            .any(|blocker| blocker.code == "source-root-overlap"));

        fixture
            .connection
            .execute(
                "DELETE FROM source_dirs WHERE id != ?1",
                params![fixture.source_id],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_trash_snapshots (
                clip_id, video_path, canonical_video_path, source_dir_path,
                canonical_source_dir_path, extension, file_existed
             )
             SELECT id, file_path, file_path, ?2, ?2, extension, 0 FROM clips WHERE id = ?1",
                params![clip_id, fixture.old_root.display().to_string()],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "UPDATE clips SET file_status = 'trashed' WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_delete_intents (
                    clip_id, video_path, source_dir_path, extension, file_existed
                 )
                 SELECT id, file_path, ?2, extension, 0 FROM clips WHERE id = ?1",
                params![clip_id, fixture.old_root.display().to_string()],
            )
            .unwrap();
        let protected = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert!(protected
            .blockers
            .iter()
            .any(|blocker| blocker.code == "trash-snapshots"));
        assert!(protected
            .blockers
            .iter()
            .any(|blocker| blocker.code == "trashed-clips"));
        assert!(protected
            .blockers
            .iter()
            .any(|blocker| blocker.code == "delete-intents"));
        assert!(commit_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root
        )
        .is_err());
        let unchanged: String = fixture
            .connection
            .query_row(
                "SELECT scan_root_path FROM source_dirs WHERE id = ?1",
                params![fixture.source_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_path(&unchanged),
            normalize_path(&fixture.old_root.display().to_string())
        );
    }

    #[test]
    fn existing_unaffected_source_overlap_blocks_snapshot_rewrites() {
        let fixture = Fixture::new("existing-overlap");
        let (_, modified) = fixture.candidate("clip.mp4", b"clip");
        fixture.insert_old_clip("clip.mp4", 4, &modified, None, None);

        let nested_root = fixture.old_root.join("nested-unaffected");
        let nested_root_string = nested_root.display().to_string();
        let other = db::register_source_dir(
            &fixture.connection,
            SourceDirInput {
                path: &nested_root_string,
                name: "Nested Unaffected",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &nested_root_string,
            },
            true,
        )
        .unwrap();
        let foreign_snapshot = nested_root
            .join("foreign-snapshot.jpg")
            .display()
            .to_string();
        fixture
            .connection
            .execute(
                "INSERT INTO match_snapshots (snapshot_id, account_id, package_path, raw_json)
                 VALUES ('foreign-overlap', 'other-account', ?1, '{}')",
                params![foreign_snapshot],
            )
            .unwrap();

        let preview = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert!(!preview.can_relocate, "{preview:#?}");
        assert!(preview
            .blockers
            .iter()
            .any(|blocker| blocker.code == "existing-source-root-overlap"));
        assert_eq!(preview.expected_metadata_reference_update_count, 0);
        assert!(commit_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .is_err());

        let (other_root_after, snapshot_after): (String, String) = fixture
            .connection
            .query_row(
                "SELECT source_dirs.scan_root_path, match_snapshots.package_path
                 FROM source_dirs CROSS JOIN match_snapshots
                 WHERE source_dirs.id = ?1 AND match_snapshots.snapshot_id = 'foreign-overlap'",
                params![other.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(other_root_after, nested_root_string);
        assert_eq!(snapshot_after, foreign_snapshot);
    }

    #[test]
    fn relocation_reconciles_more_than_public_thumbnail_command_limit() {
        const PUBLIC_THUMBNAIL_COMMAND_LIMIT: usize = 200;
        let fixture = Fixture::new("thumbnail-unbounded");
        for index in 0..=PUBLIC_THUMBNAIL_COMMAND_LIMIT {
            let relative = format!("batch/clip-{index:03}.mp4");
            let bytes = format!("video-{index:03}");
            let (_, modified) = fixture.candidate(&relative, bytes.as_bytes());
            fixture.insert_old_clip(
                &relative,
                i64::try_from(bytes.len()).unwrap(),
                &modified,
                None,
                None,
            );
        }

        let committed = commit_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .expect("relocation-owned thumbnail reconciliation must not use the command limit");
        assert_eq!(
            committed.relocated_clip_count,
            PUBLIC_THUMBNAIL_COMMAND_LIMIT + 1
        );
        let thumbnail_rows: i64 = fixture
            .connection
            .query_row("SELECT COUNT(*) FROM clip_thumbnails", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            thumbnail_rows,
            i64::try_from(PUBLIC_THUMBNAIL_COMMAND_LIMIT + 1).unwrap()
        );
    }

    #[test]
    fn final_clip_path_owned_by_an_unaffected_row_blocks_and_rolls_back() {
        let fixture = Fixture::new("final-path-conflict");
        let (candidate, modified) = fixture.candidate("clip.mp4", b"clip");
        let old_clip = fixture.insert_old_clip("clip.mp4", 4, &modified, None, None);
        let other_root = fixture.root.join("independent-source");
        fs::create_dir_all(&other_root).unwrap();
        let other = db::register_source_dir(
            &fixture.connection,
            SourceDirInput {
                path: &other_root.display().to_string(),
                name: "Independent",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &other_root.display().to_string(),
            },
            true,
        )
        .unwrap();
        let canonical_candidate = candidate.canonicalize().unwrap().display().to_string();
        db::upsert_clip(
            &fixture.connection,
            ClipInput {
                source_dir_id: other.id,
                clip_group_id: None,
                video_path: &canonical_candidate,
                file_name: "clip.mp4",
                file_size: 4,
                modified_at: Some(&modified),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .unwrap();
        let preview = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert!(!preview.can_relocate, "{preview:#?}");
        assert!(preview
            .conflicts
            .iter()
            .any(|conflict| conflict.code == "final-clip-path-conflict"));
        assert!(commit_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .is_err());
        let old_path: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![old_clip],
                |row| row.get(0),
            )
            .unwrap();
        assert!(relative_components(Path::new(&old_path), &fixture.old_root).is_some());
    }

    #[test]
    fn aclos_directory_rename_reassigns_group_without_changing_clip_id() {
        let fixture = Fixture::new("aclos-group");
        fixture
            .connection
            .execute(
                "UPDATE source_dirs SET source_kind = 'aclos', scan_mode = 'aclos-structured' WHERE id = ?1",
                params![fixture.source_id],
            )
            .unwrap();
        fixture
            .connection
            .execute(
                "INSERT INTO clip_groups (source_dir_id, group_key, display_name)
                 VALUES (?1, 'old-match', 'old-match')",
                params![fixture.source_id],
            )
            .unwrap();
        let old_group = fixture.connection.last_insert_rowid();
        let (candidate, _) = fixture.candidate("new-match/clip.mp4", b"new-content");
        fixture
            .connection
            .execute(
                "INSERT INTO clip_groups (source_dir_id, group_key, display_name)
                 VALUES (?1, 'same-match', 'same-match')",
                params![fixture.source_id],
            )
            .unwrap();
        let same_group = fixture.connection.last_insert_rowid();
        let (_, same_modified) = fixture.candidate("same-match/same.mp4", b"same");
        let (_, none_modified) = fixture.candidate("no-group/none.mp4", b"none");
        let old_path = fixture
            .old_root
            .join("old-match/clip.mp4")
            .display()
            .to_string();
        let clip_id = db::upsert_scanned_clip_with_file_identity(
            &fixture.connection,
            ClipInput {
                source_dir_id: fixture.source_id,
                clip_group_id: Some(old_group),
                video_path: &old_path,
                file_name: "clip.mp4",
                file_size: 999,
                modified_at: Some("1"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(IDENTITY_ONE),
        )
        .unwrap()
        .clip
        .id;
        let same_clip =
            fixture.insert_old_clip("same-match/same.mp4", 4, &same_modified, None, None);
        fixture
            .connection
            .execute(
                "UPDATE clips SET clip_group_id = ?2 WHERE id = ?1",
                params![same_clip, same_group],
            )
            .unwrap();
        let no_group_clip =
            fixture.insert_old_clip("no-group/none.mp4", 4, &none_modified, None, None);
        let reader = |path: &Path| {
            (path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("new-match"))
            .then_some(IDENTITY_ONE)
        };
        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap()
        .preview;
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.expected_group_update_count, 2);
        commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap();
        let (same_id, group_key, file_path): (i64, String, String) = fixture
            .connection
            .query_row(
                "SELECT clips.id, clip_groups.group_key, clips.file_path
                 FROM clips JOIN clip_groups ON clip_groups.id = clips.clip_group_id
                 WHERE clips.id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(same_id, clip_id);
        assert_eq!(group_key, "new-match");
        assert_eq!(
            normalize_path(&file_path),
            normalize_path(&candidate.canonicalize().unwrap().display().to_string())
        );
        let same_key: String = fixture
            .connection
            .query_row(
                "SELECT clip_groups.group_key FROM clips
                 JOIN clip_groups ON clip_groups.id = clips.clip_group_id WHERE clips.id = ?1",
                params![same_clip],
                |row| row.get(0),
            )
            .unwrap();
        let new_key: String = fixture
            .connection
            .query_row(
                "SELECT clip_groups.group_key FROM clips
                 JOIN clip_groups ON clip_groups.id = clips.clip_group_id WHERE clips.id = ?1",
                params![no_group_clip],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(same_key, "same-match");
        assert_eq!(new_key, "no-group");
    }

    #[test]
    fn shared_scan_root_relocates_every_logical_source_together() {
        let fixture = Fixture::new("shared-root");
        fixture
            .connection
            .execute(
                "UPDATE source_dirs SET path = ?2 WHERE id = ?1",
                params![
                    fixture.source_id,
                    fixture.old_root.join("account-a").display().to_string()
                ],
            )
            .unwrap();
        let second = db::register_source_dir(
            &fixture.connection,
            SourceDirInput {
                path: &fixture.old_root.join("account-b").display().to_string(),
                name: "Account B",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &fixture.old_root.display().to_string(),
            },
            true,
        )
        .unwrap();
        fs::create_dir_all(fixture.new_root.join("account-a")).unwrap();
        fs::create_dir_all(fixture.new_root.join("account-b")).unwrap();
        let (_, modified_a) = fixture.candidate("account-a/moved/same.mp4", b"same");
        let (_, modified_b) = fixture.candidate("account-b/moved/same.mp4", b"same");
        fixture.candidate("outside-logical-sources.mp4", b"stray");
        let old_a = fixture
            .old_root
            .join("account-a/original/same.mp4")
            .display()
            .to_string();
        let old_b = fixture
            .old_root
            .join("account-b/original/same.mp4")
            .display()
            .to_string();
        let clip_a = db::upsert_scanned_clip_with_file_identity(
            &fixture.connection,
            ClipInput {
                source_dir_id: fixture.source_id,
                clip_group_id: None,
                video_path: &old_a,
                file_name: "same.mp4",
                file_size: 4,
                modified_at: Some(&modified_a),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(IDENTITY_ONE),
        )
        .unwrap()
        .clip
        .id;
        let clip_b = db::upsert_scanned_clip_with_file_identity(
            &fixture.connection,
            ClipInput {
                source_dir_id: second.id,
                clip_group_id: None,
                video_path: &old_b,
                file_name: "same.mp4",
                file_size: 4,
                modified_at: Some(&modified_b),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(IDENTITY_TWO),
        )
        .unwrap()
        .clip
        .id;
        let reader = |path: &Path| {
            if path
                .components()
                .any(|component| component.as_os_str().to_string_lossy() == "account-b")
            {
                Some(IDENTITY_TWO)
            } else {
                Some(IDENTITY_ONE)
            }
        };
        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap()
        .preview;
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.affected_sources.len(), 2);
        assert_eq!(preview.identity_match_count, 2);
        assert_eq!(preview.new_candidate_count, 0);
        commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .unwrap();
        let paths = db::list_source_dirs(&fixture.connection).unwrap();
        assert!(paths
            .iter()
            .all(|source| normalize_path(&source.scan_root_path)
                == normalize_path(
                    &fixture
                        .new_root
                        .canonicalize()
                        .unwrap()
                        .display()
                        .to_string()
                )));
        assert!(paths
            .iter()
            .any(|source| source.path.to_lowercase().ends_with("account-a")));
        assert!(paths
            .iter()
            .any(|source| source.path.to_lowercase().ends_with("account-b")));
        let final_a: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![clip_a],
                |row| row.get(0),
            )
            .unwrap();
        let final_b: String = fixture
            .connection
            .query_row(
                "SELECT file_path FROM clips WHERE id = ?1",
                params![clip_b],
                |row| row.get(0),
            )
            .unwrap();
        assert!(normalize_path(&final_a).contains("/account-a/"));
        assert!(normalize_path(&final_b).contains("/account-b/"));
    }

    #[test]
    fn duplicate_identity_across_affected_sources_is_ambiguous() {
        let fixture = Fixture::new("shared-root-identity-ambiguity");
        let second = configure_shared_logical_sources(&fixture);
        let (_, modified) = fixture.candidate("account-a/renamed/same.mp4", b"same");
        fixture.insert_old_clip(
            "account-a/original/a-old.mp4",
            4,
            &modified,
            Some(IDENTITY_ONE),
            None,
        );
        db::upsert_scanned_clip_with_file_identity(
            &fixture.connection,
            ClipInput {
                source_dir_id: second.id,
                clip_group_id: None,
                video_path: &fixture
                    .old_root
                    .join("account-b/original/b-old.mp4")
                    .display()
                    .to_string(),
                file_name: "b-old.mp4",
                file_size: 4,
                modified_at: Some(&modified),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(IDENTITY_ONE),
        )
        .unwrap();

        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &|_| Some(IDENTITY_ONE),
        )
        .unwrap()
        .preview;

        assert!(!preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.identity_match_count, 0);
        assert!(preview
            .conflicts
            .iter()
            .any(|conflict| conflict.code == "identity-ambiguous"));
    }

    #[test]
    fn duplicate_legacy_fingerprint_across_affected_sources_is_ambiguous() {
        let fixture = Fixture::new("shared-root-legacy-ambiguity");
        let second = configure_shared_logical_sources(&fixture);
        let (_, modified) = fixture.candidate("account-a/renamed/same.mp4", b"same");
        fixture.insert_old_clip("account-a/original/same.mp4", 4, &modified, None, None);
        db::upsert_scanned_clip_with_file_identity(
            &fixture.connection,
            ClipInput {
                source_dir_id: second.id,
                clip_group_id: None,
                video_path: &fixture
                    .old_root
                    .join("account-b/original/same.mp4")
                    .display()
                    .to_string(),
                file_name: "same.mp4",
                file_size: 4,
                modified_at: Some(&modified),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            None,
        )
        .unwrap();

        let preview = build_relocation_plan(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &|_| None,
        )
        .unwrap()
        .preview;

        assert!(!preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.legacy_fingerprint_match_count, 0);
        assert!(preview
            .conflicts
            .iter()
            .any(|conflict| conflict.code == "legacy-fingerprint-ambiguous"));
    }

    #[test]
    fn candidate_change_between_plan_and_placeholder_write_rolls_back() {
        use std::cell::Cell;

        let fixture = Fixture::new("toctou");
        let (candidate, modified) = fixture.candidate("clip.mp4", b"safe");
        let clip_id = fixture.insert_old_clip("clip.mp4", 4, &modified, None, None);
        let calls = Cell::new(0usize);
        let reader = |path: &Path| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 1 {
                fs::write(path, b"changed-after-plan").unwrap();
            }
            None
        };
        let error = commit_scan_source_relocation_with_reader(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
            &reader,
        )
        .expect_err("candidate mutation must abort relocation");
        assert!(error.contains("发生变化"), "{error}");
        let (source_root, clip_path): (String, String) = fixture
            .connection
            .query_row(
                "SELECT source_dirs.scan_root_path, clips.file_path
                 FROM source_dirs JOIN clips ON clips.source_dir_id = source_dirs.id
                 WHERE clips.id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            normalize_path(&source_root),
            normalize_path(&fixture.old_root.display().to_string())
        );
        assert_eq!(
            normalize_path(&clip_path),
            normalize_path(&fixture.old_root.join("clip.mp4").display().to_string())
        );
        assert_eq!(fs::read(candidate).unwrap(), b"changed-after-plan");
    }

    #[test]
    fn root_symlink_or_reparse_point_is_rejected_when_platform_allows_fixture() {
        let fixture = Fixture::new("reparse");
        let link = fixture.root.join("linked-root");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&fixture.new_root, &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&fixture.new_root, &link);
        if created.is_err() {
            return;
        }
        let preview =
            preview_scan_source_relocation(&fixture.connection, fixture.source_id, &link).unwrap();
        assert!(!preview.can_relocate);
        assert!(preview
            .blockers
            .iter()
            .any(|blocker| blocker.code == "invalid-new-root"));
    }

    #[test]
    fn descendant_symlink_or_reparse_point_blocks_enumeration() {
        let fixture = Fixture::new("descendant-reparse");
        let outside = fixture.root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("clip.mp4"), b"outside").unwrap();
        let link = fixture.new_root.join("linked-child");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&outside, &link);
        if created.is_err() {
            return;
        }
        let preview = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &fixture.new_root,
        )
        .unwrap();
        assert!(!preview.can_relocate);
        assert!(preview
            .blockers
            .iter()
            .any(|blocker| blocker.code == "reparse-point"));
    }

    #[test]
    fn ordinary_selected_directory_below_reparse_parent_is_rejected() {
        let fixture = Fixture::new("parent-reparse");
        let outside = fixture.root.join("outside-parent");
        let selected_target = outside.join("selected");
        fs::create_dir_all(&selected_target).unwrap();
        let parent_link = fixture.root.join("parent-link");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&outside, &parent_link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&outside, &parent_link);
        if created.is_err() {
            return;
        }
        let selected_through_link = parent_link.join("selected");
        let selected_metadata = fs::symlink_metadata(&selected_through_link).unwrap();
        assert!(selected_metadata.is_dir());
        assert!(!metadata_is_reparse_point(&selected_metadata));
        let preview = preview_scan_source_relocation(
            &fixture.connection,
            fixture.source_id,
            &selected_through_link,
        )
        .unwrap();
        assert!(!preview.can_relocate);
        assert!(preview
            .blockers
            .iter()
            .any(|blocker| blocker.code == "invalid-new-root"));
    }

    #[test]
    fn component_replacement_is_not_a_string_prefix_replacement() {
        let old = Path::new("C:/clips");
        let new = Path::new("D:/restored");
        assert_eq!(
            relocate_root_bound_path(Path::new("C:/clips/a/x.json"), old, new)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/"),
            "D:/restored/a/x.json"
        );
        assert!(relocate_root_bound_path(Path::new("C:/clips-old/a.json"), old, new).is_none());
        assert!(
            relocate_root_bound_path(Path::new("C:/clips/../outside.json"), old, new).is_none()
        );
        let existing_relative = Path::new("src");
        assert!(existing_relative.is_dir());
        assert!(validate_relocation_root(existing_relative).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires VALOFRAME_CROSS_VOLUME_TEST_ROOT"]
    fn real_cross_volume_exact_path_match_preserves_clip_identity_and_user_state() {
        struct CrossVolumeCleanup(Vec<PathBuf>);
        impl Drop for CrossVolumeCleanup {
            fn drop(&mut self) {
                for path in &self.0 {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }

        let requested_second_root = std::env::var_os("VALOFRAME_CROSS_VOLUME_TEST_ROOT")
            .map(PathBuf::from)
            .expect("set VALOFRAME_CROSS_VOLUME_TEST_ROOT to an existing writable directory on the second NTFS volume");
        assert!(requested_second_root.is_absolute());
        assert!(requested_second_root.is_dir());
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let old_root = std::env::temp_dir().join(format!("valoframe-cross-volume-old-{nonce}"));
        let new_root = requested_second_root.join(format!("valoframe-cross-volume-new-{nonce}"));
        let _cleanup = CrossVolumeCleanup(vec![old_root.clone(), new_root.clone()]);
        let old_file = old_root.join("nested/clip.mp4");
        let new_file = new_root.join("nested/clip.mp4");
        fs::create_dir_all(old_file.parent().unwrap()).unwrap();
        fs::create_dir_all(new_file.parent().unwrap()).unwrap();

        let bytes = b"cross-volume-exact-match";
        let (old_modified, new_modified) = (0..32)
            .find_map(|_| {
                fs::write(&old_file, bytes).unwrap();
                fs::write(&new_file, bytes).unwrap();
                let old_modified = fs::metadata(&old_file)
                    .and_then(|metadata| metadata.modified())
                    .map(format_system_time)
                    .unwrap();
                let new_modified = fs::metadata(&new_file)
                    .and_then(|metadata| metadata.modified())
                    .map(format_system_time)
                    .unwrap();
                (old_modified == new_modified).then_some((old_modified, new_modified))
            })
            .expect("could not create both fixtures within the same persisted mtime second");
        assert_eq!(old_modified, new_modified);
        let old_identity = read_stable_file_snapshot(&old_file)
            .unwrap()
            .identity
            .expect("old NTFS file should expose stable identity");
        let new_identity = read_stable_file_snapshot(&new_file)
            .unwrap()
            .identity
            .expect("new NTFS file should expose stable identity");
        assert_ne!(
            old_identity.volume_serial, new_identity.volume_serial,
            "VALOFRAME_CROSS_VOLUME_TEST_ROOT must be on a different volume than the test temp directory"
        );

        let connection = Connection::open_in_memory().unwrap();
        db::initialize_schema(&connection).unwrap();
        let source = db::register_source_dir(
            &connection,
            SourceDirInput {
                path: &old_root.display().to_string(),
                name: "Cross Volume",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &old_root.display().to_string(),
            },
            true,
        )
        .unwrap();
        let clip_id = db::upsert_scanned_clip_with_file_identity(
            &connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &old_file.display().to_string(),
                file_name: "clip.mp4",
                file_size: i64::try_from(bytes.len()).unwrap(),
                modified_at: Some(&old_modified),
                duration_ms: Some(10_000),
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(old_identity),
        )
        .unwrap()
        .clip
        .id;
        connection
            .execute(
                "UPDATE clips SET is_favorite = 1, note = 'cross-volume-state' WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        fs::remove_file(&old_file).unwrap();

        let preview = preview_scan_source_relocation(&connection, source.id, &new_root).unwrap();
        assert!(preview.can_relocate, "{preview:#?}");
        assert_eq!(preview.exact_path_match_count, 1);
        assert_eq!(preview.identity_match_count, 0);
        let committed = commit_scan_source_relocation(&connection, source.id, &new_root).unwrap();
        assert_eq!(committed.relocated_clip_count, 1);
        let (same_id, favorite, note, path): (i64, i64, String, String) = connection
            .query_row(
                "SELECT id, is_favorite, note, file_path FROM clips WHERE id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(same_id, clip_id);
        assert_eq!(favorite, 1);
        assert_eq!(note, "cross-volume-state");
        assert_eq!(
            normalize_path(&path),
            normalize_path(&new_file.canonicalize().unwrap().display().to_string())
        );
    }
}
