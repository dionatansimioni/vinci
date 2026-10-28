//! Cancelling a running install: a flag shared between the UI and the worker, plus the
//! process group of the command running right now so it can be stopped at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    /// Undoing a cancelled job: its own commands must run to the end.
    cleaning_up: AtomicBool,
    /// Process group of the running step (its leader's pid), if any.
    running: Mutex<Option<u32>>,
}

impl Cancel {
    /// Marks the job cancelled and stops the running command with its children.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        if self.cleaning_up() {
            return;
        }
        if let Some(pgid) = *self.0.running.lock().unwrap_or_else(|e| e.into_inner()) {
            terminate_group(pgid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// From now on commands run to completion even though the job is cancelled: used
    /// while removing what the job created (the same runner does the cleanup).
    pub fn begin_cleanup(&self) {
        self.0.cleaning_up.store(true, Ordering::SeqCst);
    }

    fn cleaning_up(&self) -> bool {
        self.0.cleaning_up.load(Ordering::SeqCst)
    }

    /// Called by the runner around each command it starts in its own process group.
    pub(crate) fn set_running(&self, pgid: Option<u32>) {
        *self.0.running.lock().unwrap_or_else(|e| e.into_inner()) = pgid;
        // A cancel that landed between the check and the spawn must still stop it.
        if let (Some(pgid), true) = (pgid, self.is_cancelled() && !self.cleaning_up()) {
            terminate_group(pgid);
        }
    }
}

fn terminate_group(pgid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pgid as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{argv, ExecMode, HostRunner, Runner};
    use std::time::{Duration, Instant};

    #[test]
    fn stops_the_running_command_and_its_children() {
        let cancel = Cancel::default();
        let runner = HostRunner {
            mode: ExecMode::Direct,
            cancel: Some(cancel.clone()),
        };
        let c = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c.cancel();
        });
        let start = Instant::now();
        // The inner sleep is a grandchild: it must die with the group, or the pipes
        // stay open and the call never returns.
        let out = runner
            .run_streaming(&argv(&["sh", "-c", "sleep 30 & wait"]), &mut |_| {})
            .unwrap();
        t.join().unwrap();
        assert!(!out.success);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn a_cancel_before_the_start_still_stops_it() {
        let cancel = Cancel::default();
        cancel.cancel();
        let runner = HostRunner {
            mode: ExecMode::Direct,
            cancel: Some(cancel),
        };
        let start = Instant::now();
        let out = runner
            .run_streaming(&argv(&["sleep", "30"]), &mut |_| {})
            .unwrap();
        assert!(!out.success);
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    /// The cleanup after a cancel runs through the same runner; its commands must not
    /// be stopped (this once made the restore after a cancelled reinstall fail).
    #[test]
    fn cleanup_commands_run_to_the_end() {
        let cancel = Cancel::default();
        cancel.cancel();
        cancel.begin_cleanup();
        let runner = HostRunner {
            mode: ExecMode::Direct,
            cancel: Some(cancel.clone()),
        };
        let out = runner
            .run_streaming(&argv(&["sh", "-c", "sleep 0.3; echo done"]), &mut |_| {})
            .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, "done\n");
        cancel.cancel(); // a second click during the cleanup changes nothing
        assert!(cancel.is_cancelled());
    }
}
