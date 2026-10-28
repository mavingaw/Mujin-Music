//! Long library-tools operations (rescan, re-identify, re-analyze, the
//! skipped-files check) run as jobs with live progress that every client
//! can poll, and that can be cancelled.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Rescan,
    Reidentify,
    Reanalyze,
    SkippedCheck,
    /// Better versions' spectral check of lossless files.
    LosslessCheck,
}

impl JobKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Rescan => "Rescan",
            Self::Reidentify => "Re-identify",
            Self::Reanalyze => "Re-analyze audio",
            Self::SkippedCheck => "Check for skipped files",
            Self::LosslessCheck => "Check for fake lossless",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobStatus {
    pub id: u64,
    pub kind: JobKind,
    pub title: String,
    /// What the job covers, e.g. "Whole library" or an album title.
    pub scope: String,
    pub state: JobState,
    /// Items to process; 0 while unknown.
    pub total: u64,
    pub done: u64,
    pub failed: u64,
    /// Phase or item being worked on.
    pub current: String,
    pub message: String,
    pub started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
}

struct Job {
    status: JobStatus,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Registry {
    jobs: VecDeque<Job>,
}

const KEEP_FINISHED: usize = 12;

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Handle a running job reports through. Dropping it without finishing
/// marks the job failed, so a panic never leaves a job "running".
pub struct JobHandle {
    id: u64,
    cancel: Arc<AtomicBool>,
    finished: bool,
}

impl JobHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    fn update(&self, change: impl FnOnce(&mut JobStatus)) {
        let mut registry = registry();
        if let Some(job) = registry
            .jobs
            .iter_mut()
            .find(|job| job.status.id == self.id)
        {
            change(&mut job.status);
        }
    }

    pub fn set_total(&self, total: u64) {
        self.update(|status| status.total = total);
    }

    pub fn set_current(&self, current: impl Into<String>) {
        let current = current.into();
        self.update(|status| status.current = current);
    }

    pub fn advance(&self, succeeded: bool) {
        self.update(|status| {
            status.done += 1;
            if !succeeded {
                status.failed += 1;
            }
        });
    }

    pub fn set_done(&self, done: u64) {
        self.update(|status| status.done = done);
    }

    pub fn finish(mut self, message: impl Into<String>) {
        let message = message.into();
        let cancelled = self.is_cancelled();
        self.update(|status| {
            status.state = if cancelled {
                JobState::Cancelled
            } else {
                JobState::Completed
            };
            status.current.clear();
            status.message = if cancelled {
                "Stopped before it finished.".into()
            } else {
                message
            };
            status.finished_at_ms = Some(now_ms());
        });
        self.finished = true;
    }

    pub fn fail(mut self, message: impl Into<String>) {
        let message = message.into();
        self.update(|status| {
            status.state = JobState::Failed;
            status.current.clear();
            status.message = message;
            status.finished_at_ms = Some(now_ms());
        });
        self.finished = true;
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.update(|status| {
                status.state = JobState::Failed;
                status.message = "The job stopped unexpectedly.".into();
                status.finished_at_ms = Some(now_ms());
            });
        }
    }
}

/// Starts a job, or returns the running job of the same kind.
pub fn start(kind: JobKind, scope: impl Into<String>) -> Result<JobHandle, JobStatus> {
    let mut registry = registry();
    if let Some(running) = registry
        .jobs
        .iter()
        .find(|job| job.status.kind == kind && job.status.state == JobState::Running)
    {
        return Err(running.status.clone());
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    registry.jobs.push_front(Job {
        status: JobStatus {
            id,
            kind,
            title: kind.title().into(),
            scope: scope.into(),
            state: JobState::Running,
            total: 0,
            done: 0,
            failed: 0,
            current: String::new(),
            message: String::new(),
            started_at_ms: now_ms(),
            finished_at_ms: None,
        },
        cancel: cancel.clone(),
    });
    let mut finished = 0;
    registry.jobs.retain(|job| {
        if job.status.state == JobState::Running {
            return true;
        }
        finished += 1;
        finished <= KEEP_FINISHED
    });
    Ok(JobHandle {
        id,
        cancel,
        finished: false,
    })
}

/// Every running job and the most recent finished ones, newest first.
pub fn list() -> Vec<JobStatus> {
    registry()
        .jobs
        .iter()
        .map(|job| job.status.clone())
        .collect()
}

pub fn get(id: u64) -> Option<JobStatus> {
    registry()
        .jobs
        .iter()
        .find(|job| job.status.id == id)
        .map(|job| job.status.clone())
}

/// Asks a running job to stop. Returns false for unknown or finished jobs.
pub fn cancel(id: u64) -> bool {
    let registry = registry();
    match registry
        .jobs
        .iter()
        .find(|job| job.status.id == id && job.status.state == JobState::Running)
    {
        Some(job) => {
            job.cancel.store(true, Ordering::Release);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_report_progress_refuse_doubles_and_cancel() {
        let job = start(JobKind::SkippedCheck, "Whole library").expect("starts");
        assert!(start(JobKind::SkippedCheck, "again").is_err());
        job.set_total(4);
        job.advance(true);
        job.advance(false);
        job.set_current("Album");
        let status = get(job.id()).expect("listed");
        assert_eq!((status.total, status.done, status.failed), (4, 2, 1));
        assert_eq!(status.current, "Album");
        assert!(cancel(job.id()));
        assert!(job.is_cancelled());
        let id = job.id();
        job.finish("done");
        let status = get(id).expect("kept");
        assert_eq!(status.state, JobState::Cancelled);
        assert!(!cancel(id));

        // A dropped handle never leaves a job running.
        let dropped = start(JobKind::SkippedCheck, "Whole library").expect("starts again");
        let id = dropped.id();
        drop(dropped);
        assert_eq!(get(id).expect("kept").state, JobState::Failed);
    }
}
