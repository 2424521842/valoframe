use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanJobStatus {
    Idle,
    Running,
    Cancelling,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl ScanJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressEvent {
    pub job_id: String,
    pub phase: String,
    pub current_root: Option<String>,
    pub source: Option<String>,
    pub processed: u64,
    pub total: Option<u64>,
    pub message: String,
    pub terminal: bool,
    pub status: ScanJobStatus,
    pub source_dir_count: i64,
    pub clip_group_count: i64,
    pub clip_file_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    pub job_id: Option<String>,
    pub phase: Option<String>,
    pub current_root: Option<String>,
    pub source: Option<String>,
    pub processed: u64,
    pub total: Option<u64>,
    pub message: String,
    pub terminal: bool,
    pub status: ScanJobStatus,
}

impl ScanStatus {
    fn idle() -> Self {
        Self {
            job_id: None,
            phase: None,
            current_root: None,
            source: None,
            processed: 0,
            total: None,
            message: "当前没有扫描任务".to_string(),
            terminal: false,
            status: ScanJobStatus::Idle,
        }
    }

    fn running(job_id: String) -> Self {
        Self {
            job_id: Some(job_id),
            phase: Some("starting".to_string()),
            current_root: None,
            source: None,
            processed: 0,
            total: None,
            message: "正在启动扫描任务".to_string(),
            terminal: false,
            status: ScanJobStatus::Running,
        }
    }

    fn from_event(event: &ScanProgressEvent) -> Self {
        Self {
            job_id: Some(event.job_id.clone()),
            phase: Some(event.phase.clone()),
            current_root: event.current_root.clone(),
            source: event.source.clone(),
            processed: event.processed,
            total: event.total,
            message: event.message.clone(),
            terminal: event.terminal,
            status: event.status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyRunning {
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelRequestOutcome {
    pub accepted: bool,
    pub reason: &'static str,
    pub active_job_id: Option<String>,
    pub status: ScanStatus,
}

struct ActiveScan {
    job_id: String,
    cancellation: Arc<AtomicBool>,
}

struct CoordinatorState {
    active: Option<ActiveScan>,
    latest: ScanStatus,
}

pub struct ScanCoordinator {
    next_job_id: AtomicU64,
    state: Mutex<CoordinatorState>,
}

impl Default for ScanCoordinator {
    fn default() -> Self {
        Self {
            next_job_id: AtomicU64::new(1),
            state: Mutex::new(CoordinatorState {
                active: None,
                latest: ScanStatus::idle(),
            }),
        }
    }
}

impl ScanCoordinator {
    pub fn begin(self: &Arc<Self>) -> Result<ScanJobLease, AlreadyRunning> {
        let mut state = lock_recover(&self.state);
        if let Some(active) = &state.active {
            return Err(AlreadyRunning {
                job_id: active.job_id.clone(),
            });
        }

        let sequence = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let job_id = format!("scan-{millis}-{sequence}");
        let cancellation = Arc::new(AtomicBool::new(false));
        state.active = Some(ActiveScan {
            job_id: job_id.clone(),
            cancellation: cancellation.clone(),
        });
        state.latest = ScanStatus::running(job_id.clone());

        Ok(ScanJobLease {
            coordinator: self.clone(),
            job_id,
            cancellation,
            finished: false,
        })
    }

    pub fn status(&self) -> ScanStatus {
        lock_recover(&self.state).latest.clone()
    }

    pub fn record_event(&self, event: &ScanProgressEvent) -> bool {
        let mut state = lock_recover(&self.state);
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.job_id != event.job_id)
        {
            return false;
        }

        state.latest = ScanStatus::from_event(event);
        true
    }

    pub fn request_cancel(&self, job_id: &str) -> CancelRequestOutcome {
        let mut state = lock_recover(&self.state);
        let Some(active) = &state.active else {
            return CancelRequestOutcome {
                accepted: false,
                reason: "not-running",
                active_job_id: None,
                status: state.latest.clone(),
            };
        };

        if active.job_id != job_id {
            return CancelRequestOutcome {
                accepted: false,
                reason: "job-mismatch",
                active_job_id: Some(active.job_id.clone()),
                status: state.latest.clone(),
            };
        }

        active.cancellation.store(true, Ordering::Release);
        state.latest.status = ScanJobStatus::Cancelling;
        state.latest.phase = Some("cancelling".to_string());
        state.latest.message = "正在取消扫描".to_string();
        state.latest.terminal = false;

        CancelRequestOutcome {
            accepted: true,
            reason: "accepted",
            active_job_id: Some(job_id.to_string()),
            status: state.latest.clone(),
        }
    }

    fn finish(&self, job_id: &str, status: ScanJobStatus, message: &str) {
        debug_assert!(status.is_terminal());
        let mut state = lock_recover(&self.state);
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.job_id != job_id)
        {
            return;
        }

        state.active = None;
        if state.latest.job_id.as_deref() != Some(job_id) {
            state.latest = ScanStatus::running(job_id.to_string());
        }
        state.latest.status = status;
        state.latest.terminal = true;
        state.latest.message = message.to_string();
        state.latest.phase = Some(status.as_str().to_string());
    }
}

pub struct ScanJobLease {
    coordinator: Arc<ScanCoordinator>,
    job_id: String,
    cancellation: Arc<AtomicBool>,
    finished: bool,
}

impl ScanJobLease {
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancellation.clone()
    }

    pub fn finish(mut self, status: ScanJobStatus, message: &str) {
        self.coordinator.finish(&self.job_id, status, message);
        self.finished = true;
    }
}

impl Drop for ScanJobLease {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.coordinator.finish(
            &self.job_id,
            ScanJobStatus::Failed,
            "扫描任务意外终止，互斥已释放",
        );
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::Barrier,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{db, scanner};

    #[test]
    fn concurrent_begin_allows_exactly_one_active_job() {
        let coordinator = Arc::new(ScanCoordinator::default());
        let first = coordinator.begin().expect("first scan should start");
        let conflict = match coordinator.begin() {
            Ok(_) => panic!("second scan should be rejected"),
            Err(conflict) => conflict,
        };

        assert_eq!(conflict.job_id, first.job_id());
        assert_eq!(coordinator.status().status, ScanJobStatus::Running);

        first.finish(ScanJobStatus::Completed, "done");
        let second = coordinator
            .begin()
            .expect("a terminal scan should release the coordinator");
        assert_ne!(second.job_id(), conflict.job_id);
    }

    #[test]
    fn concurrent_entry_attempts_create_only_one_actual_scan_run() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("vhm-coordinator-{unique}"));
        let source = root.join("wonderfulVideos-main");
        let group = source.join("match-a");
        let database_path = root.join("index.sqlite3");
        fs::create_dir_all(&group).expect("scan fixture should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        let connection =
            rusqlite::Connection::open(&database_path).expect("database should be created");
        db::initialize_schema(&connection).expect("database schema should initialize");
        drop(connection);

        let coordinator = Arc::new(ScanCoordinator::default());
        let start = Arc::new(Barrier::new(3));
        let attempted = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let coordinator = coordinator.clone();
                let start = start.clone();
                let attempted = attempted.clone();
                let database_path = database_path.clone();
                let source = source.clone();
                thread::spawn(move || {
                    start.wait();
                    let lease = coordinator.begin();
                    attempted.wait();
                    let Ok(lease) = lease else {
                        return false;
                    };
                    let connection =
                        db::open_database(&database_path).expect("worker database should open");
                    let cancellation = lease.cancellation();
                    let execution = scanner::scan_roots_with_progress_and_cancel(
                        &connection,
                        &[source],
                        lease.job_id(),
                        cancellation.as_ref(),
                        |_| {},
                    )
                    .expect("winning scan should run");
                    let status = match execution.status {
                        scanner::ScanExecutionStatus::Completed => ScanJobStatus::Completed,
                        scanner::ScanExecutionStatus::Partial => ScanJobStatus::Partial,
                        scanner::ScanExecutionStatus::Cancelled => ScanJobStatus::Cancelled,
                    };
                    lease.finish(status, "done");
                    true
                })
            })
            .collect::<Vec<_>>();

        start.wait();
        attempted.wait();
        let accepted = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker should join"))
            .filter(|accepted| *accepted)
            .count();
        assert_eq!(accepted, 1);

        let connection = db::open_database(&database_path).expect("database should reopen");
        let scan_run_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
            .expect("scan run count should load");
        let running_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM scan_runs WHERE status IN ('running', 'cancelling')",
                [],
                |row| row.get(0),
            )
            .expect("running count should load");
        assert_eq!(scan_run_count, 1);
        assert_eq!(running_count, 0);

        drop(connection);
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn only_the_active_job_id_can_request_cancellation() {
        let coordinator = Arc::new(ScanCoordinator::default());
        let lease = coordinator.begin().expect("scan should start");

        let stale = coordinator.request_cancel("scan-stale");
        assert!(!stale.accepted);
        assert_eq!(stale.reason, "job-mismatch");
        assert!(!lease.cancellation().load(Ordering::Acquire));

        let accepted = coordinator.request_cancel(lease.job_id());
        assert!(accepted.accepted);
        assert_eq!(accepted.status.status, ScanJobStatus::Cancelling);
        assert!(lease.cancellation().load(Ordering::Acquire));
        lease.finish(ScanJobStatus::Cancelled, "cancelled");
        coordinator
            .begin()
            .expect("a cancelled scan should release the coordinator");
    }

    #[test]
    fn dropping_a_lease_is_panic_safe_and_releases_the_coordinator() {
        let coordinator = Arc::new(ScanCoordinator::default());
        let first_job_id = {
            let lease = coordinator.begin().expect("scan should start");
            lease.job_id().to_string()
        };

        let status = coordinator.status();
        assert_eq!(status.job_id.as_deref(), Some(first_job_id.as_str()));
        assert_eq!(status.status, ScanJobStatus::Failed);
        assert!(status.terminal);
        coordinator
            .begin()
            .expect("dropped lease should release the coordinator");
    }

    #[test]
    fn stale_job_progress_cannot_replace_the_active_snapshot() {
        let coordinator = Arc::new(ScanCoordinator::default());
        let lease = coordinator.begin().expect("scan should start");
        let event = ScanProgressEvent {
            job_id: "scan-stale".to_string(),
            phase: "scanning".to_string(),
            current_root: Some("stale".to_string()),
            source: None,
            processed: 99,
            total: Some(100),
            message: "stale".to_string(),
            terminal: false,
            status: ScanJobStatus::Running,
            source_dir_count: 0,
            clip_group_count: 0,
            clip_file_count: 0,
        };

        assert!(!coordinator.record_event(&event));
        assert_eq!(coordinator.status().job_id.as_deref(), Some(lease.job_id()));
        assert_eq!(coordinator.status().processed, 0);
    }
}
