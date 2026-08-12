use std::sync::{Arc, Mutex, MutexGuard, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriticalTaskKind {
    Scan,
    PermanentDelete,
    Export,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CriticalTaskSnapshot {
    pub scan_count: usize,
    pub permanent_delete_count: usize,
    pub export_count: usize,
    pub relocation_count: usize,
    pub update_installing: bool,
}

impl CriticalTaskSnapshot {
    pub fn is_busy(self) -> bool {
        self.scan_count > 0
            || self.permanent_delete_count > 0
            || self.export_count > 0
            || self.relocation_count > 0
    }

    pub fn busy_message(self) -> String {
        let mut tasks = Vec::new();
        if self.scan_count > 0 {
            tasks.push("扫描任务");
        }
        if self.permanent_delete_count > 0 {
            tasks.push("永久删除任务");
        }
        if self.export_count > 0 {
            tasks.push("视频导出任务");
        }
        if self.relocation_count > 0 {
            tasks.push("来源重新定位任务");
        }
        if self.update_installing {
            tasks.push("更新安装任务");
        }
        if tasks.is_empty() {
            "当前没有关键任务".to_string()
        } else {
            format!("正在执行{}，请等待任务结束后重试", tasks.join("和"))
        }
    }
}

#[derive(Default)]
pub struct CriticalTaskGate {
    state: Mutex<CriticalTaskSnapshot>,
}

impl CriticalTaskGate {
    pub fn enter(
        self: &Arc<Self>,
        kind: CriticalTaskKind,
    ) -> Result<CriticalTaskLease, &'static str> {
        let mut state = lock_recover(&self.state);
        if state.update_installing {
            return Err("应用正在安装更新，不能启动新的关键任务");
        }
        if state.relocation_count > 0 {
            return Err("应用正在重新定位视频来源，不能启动新的文件任务");
        }
        match kind {
            CriticalTaskKind::Scan => state.scan_count += 1,
            CriticalTaskKind::PermanentDelete => state.permanent_delete_count += 1,
            CriticalTaskKind::Export => state.export_count += 1,
        }
        Ok(CriticalTaskLease {
            gate: Arc::downgrade(self),
            kind,
            active: true,
        })
    }

    pub fn begin_update_install(
        self: &Arc<Self>,
    ) -> Result<UpdateInstallLease, CriticalTaskSnapshot> {
        let mut state = lock_recover(&self.state);
        if state.update_installing || state.is_busy() {
            return Err(*state);
        }
        state.update_installing = true;
        Ok(UpdateInstallLease {
            gate: Arc::downgrade(self),
            active: true,
        })
    }

    /// Acquires the only task lease that is exclusive with every filesystem-sensitive operation.
    /// The caller must drop this lease before starting the post-relocation synchronization job.
    pub fn begin_source_relocation(
        self: &Arc<Self>,
    ) -> Result<SourceRelocationLease, CriticalTaskSnapshot> {
        let mut state = lock_recover(&self.state);
        if state.update_installing || state.is_busy() {
            return Err(*state);
        }
        state.relocation_count = 1;
        Ok(SourceRelocationLease {
            gate: Arc::downgrade(self),
            active: true,
        })
    }

    #[cfg(test)]
    fn snapshot(&self) -> CriticalTaskSnapshot {
        *lock_recover(&self.state)
    }
}

pub struct CriticalTaskLease {
    gate: Weak<CriticalTaskGate>,
    kind: CriticalTaskKind,
    active: bool,
}

impl Drop for CriticalTaskLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(gate) = self.gate.upgrade() else {
            return;
        };
        let mut state = lock_recover(&gate.state);
        match self.kind {
            CriticalTaskKind::Scan => state.scan_count = state.scan_count.saturating_sub(1),
            CriticalTaskKind::PermanentDelete => {
                state.permanent_delete_count = state.permanent_delete_count.saturating_sub(1)
            }
            CriticalTaskKind::Export => state.export_count = state.export_count.saturating_sub(1),
        }
        self.active = false;
    }
}

pub struct UpdateInstallLease {
    gate: Weak<CriticalTaskGate>,
    active: bool,
}

#[derive(Debug)]
pub struct SourceRelocationLease {
    gate: Weak<CriticalTaskGate>,
    active: bool,
}

impl Drop for SourceRelocationLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(gate) = self.gate.upgrade() {
            lock_recover(&gate.state).relocation_count = 0;
        }
        self.active = false;
    }
}

impl Drop for UpdateInstallLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(gate) = self.gate.upgrade() {
            lock_recover(&gate.state).update_installing = false;
        }
        self.active = false;
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

    #[test]
    fn update_install_waits_for_all_critical_tasks() {
        let gate = Arc::new(CriticalTaskGate::default());
        let scan = gate
            .enter(CriticalTaskKind::Scan)
            .expect("scan should enter");
        let deletion = gate
            .enter(CriticalTaskKind::PermanentDelete)
            .expect("delete should enter");

        let blocked = match gate.begin_update_install() {
            Ok(_) => panic!("update should be blocked"),
            Err(snapshot) => snapshot,
        };
        assert_eq!(blocked.scan_count, 1);
        assert_eq!(blocked.permanent_delete_count, 1);

        drop(scan);
        drop(deletion);
        let install = gate
            .begin_update_install()
            .expect("idle gate should allow update");
        assert!(gate.snapshot().update_installing);
        assert!(gate.enter(CriticalTaskKind::Scan).is_err());
        assert!(gate.enter(CriticalTaskKind::Export).is_err());
        drop(install);
        assert!(!gate.snapshot().update_installing);
        gate.enter(CriticalTaskKind::Scan)
            .expect("tasks should resume after a failed or completed install");
    }

    #[test]
    fn dropping_task_leases_is_panic_safe_and_saturating() {
        let gate = Arc::new(CriticalTaskGate::default());
        {
            let _scan = gate
                .enter(CriticalTaskKind::Scan)
                .expect("scan should enter");
            assert_eq!(gate.snapshot().scan_count, 1);
        }
        assert_eq!(gate.snapshot().scan_count, 0);
    }

    #[test]
    fn relocation_is_exclusive_in_both_directions_and_releases_on_drop() {
        let gate = Arc::new(CriticalTaskGate::default());
        let scan = gate
            .enter(CriticalTaskKind::Scan)
            .expect("scan should enter an idle gate");
        let blocked = gate
            .begin_source_relocation()
            .expect_err("an active scan must block relocation");
        assert_eq!(blocked.scan_count, 1);
        drop(scan);

        let relocation = gate
            .begin_source_relocation()
            .expect("relocation should enter an idle gate");
        assert_eq!(gate.snapshot().relocation_count, 1);
        assert!(gate.enter(CriticalTaskKind::Scan).is_err());
        assert!(gate.enter(CriticalTaskKind::PermanentDelete).is_err());
        assert!(gate.enter(CriticalTaskKind::Export).is_err());
        assert!(gate.begin_update_install().is_err());
        assert!(gate.begin_source_relocation().is_err());
        drop(relocation);

        assert_eq!(gate.snapshot().relocation_count, 0);
        gate.enter(CriticalTaskKind::Scan)
            .expect("tasks should resume when relocation releases its lease");
    }

    #[test]
    fn update_install_waits_for_exports_and_blocks_new_exports() {
        let gate = Arc::new(CriticalTaskGate::default());
        let export = gate
            .enter(CriticalTaskKind::Export)
            .expect("export should enter an idle gate");
        let blocked = match gate.begin_update_install() {
            Ok(_) => panic!("an active export must block update installation"),
            Err(snapshot) => snapshot,
        };
        assert_eq!(blocked.export_count, 1);
        assert!(blocked.busy_message().contains("视频导出任务"));
        drop(export);

        let install = gate
            .begin_update_install()
            .expect("update should enter after export completes");
        assert!(gate.enter(CriticalTaskKind::Export).is_err());
        drop(install);
        gate.enter(CriticalTaskKind::Export)
            .expect("exports should resume after the install lease is released");
    }
}
