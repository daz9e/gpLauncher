use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{Result, anyhow};

use crate::{Reporter, http};

#[derive(Clone, Debug)]
pub struct Job {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

const THREADS: usize = 16;

/// Downloads every job that is missing or damaged, in parallel.
/// Progress is reported in bytes when sizes are known, otherwise in files.
pub fn run(jobs: Vec<Job>, label: &str, reporter: &Reporter) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let pending: Vec<Job> = jobs
        .into_iter()
        .filter(|j| seen.insert(j.path.clone()))
        .filter(|j| !http::file_ok(&j.path, j.sha1.as_deref(), j.size))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }

    let by_bytes = pending.iter().all(|j| j.size.is_some());
    let total = if by_bytes { pending.iter().map(|j| j.size.unwrap()).sum() } else { pending.len() as u64 };
    reporter.status(format!("{label}: {} file(s)", pending.len()));
    reporter.progress(0, total);

    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..THREADS.min(pending.len()) {
            scope.spawn(|| {
                loop {
                    if error.lock().unwrap().is_some() {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = pending.get(i) else { return };
                    let on_bytes = |n: u64| {
                        if by_bytes {
                            let d = done.fetch_add(n, Ordering::Relaxed) + n;
                            reporter.progress(d, total);
                        }
                    };
                    match http::download(&job.url, &job.path, job.sha1.as_deref(), &on_bytes) {
                        Ok(()) => {
                            if !by_bytes {
                                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                                reporter.progress(d, total);
                            }
                        }
                        Err(e) => {
                            error.lock().unwrap().get_or_insert(e);
                            return;
                        }
                    }
                }
            });
        }
    });

    match error.into_inner().unwrap() {
        Some(e) => Err(anyhow!("{label}: {e:#}")),
        None => Ok(()),
    }
}
