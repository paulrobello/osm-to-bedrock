//! Shared job/application state for the HTTP API server.
//!
//! [`Jobs`] is the in-memory map of job ID → [`JobState`] that every handler
//! reads and mutates. [`AppState`] is the Axum-shared handle holding the
//! `Jobs` map plus the concurrency [`Semaphore`](tokio::sync::Semaphore).
//!
//! [`lock_jobs`] is the single entry point for acquiring the jobs lock — it
//! recovers from mutex poisoning (SEC-006) instead of propagating the panic,
//! so a panicked worker can't take down the API on the next request.
//! [`set_job_error`] and [`zip_and_persist`] are the only call sites that
//! should mutate a job's terminal state from a worker thread.
//!
//! [`sanitize_world_name`] lives here because the sanitised name flows into
//! both the on-disk world directory and the `Content-Disposition` header
//! served by `/download`. [`job_eviction_task`] is the background TTL
//! reaper that drops stale `Done`/`Error` jobs and deletes their persisted
//! temp directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pipeline::zip_directory;

/// The state of a background conversion job.
#[derive(Clone)]
pub(crate) enum JobState {
    Running {
        progress: f32,
        message: String,
    },
    Done {
        path: PathBuf,
        /// Wall-clock time at which the job reached the Done state.
        created: Instant,
    },
    Error {
        /// Generic, client-safe summary (e.g. `"conversion failed"`). Returned
        /// from `/status` and `/download` so internal details don't leak.
        ///
        /// The full `anyhow` chain is logged at ERROR level by [`set_job_error`]
        /// when the job transitions into this state; it is intentionally not
        /// stored on the job to avoid any future code path leaking it.
        public_message: String,
        /// Wall-clock time at which the job failed.
        created: Instant,
    },
}

/// Shared application state holding all conversion jobs.
pub(crate) type Jobs = Arc<Mutex<HashMap<String, JobState>>>;

/// How long completed (Done or Error) jobs are kept in memory before eviction.
///
/// The persisted temp directory is also cleaned up at eviction time.
pub(crate) const JOB_TTL: Duration = Duration::from_secs(2 * 60 * 60); // 2 hours

/// Maximum number of simultaneously running conversion jobs.
///
/// Requests that would exceed this cap are rejected with HTTP 429.
pub(crate) const MAX_CONCURRENT_JOBS: usize = 4;

/// Axum app state.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) jobs: Jobs,
    /// Semaphore that bounds the number of concurrent blocking conversion jobs.
    pub(crate) semaphore: Arc<tokio::sync::Semaphore>,
}

/// Lock the shared jobs map, recovering from mutex poisoning.
///
/// If a worker thread panicked while holding the lock, the mutex becomes
/// "poisoned" and the standard `.lock().expect(...)` would propagate the panic,
/// taking down the API server on the *next* request. Recovering via
/// [`PoisonError::into_inner`] keeps the lock usable — at worst, some stale
/// state from the panicked worker remains visible, but the affected job has
/// already recorded its own `Error` state and the server stays up.
pub(crate) fn lock_jobs(jobs: &Jobs) -> std::sync::MutexGuard<'_, HashMap<String, JobState>> {
    jobs.lock().unwrap_or_else(|poisoned| {
        log::error!(
            "jobs mutex was poisoned by a panicked worker — recovering for server stability"
        );
        poisoned.into_inner()
    })
}

/// Record a terminal error state for `jid` into the jobs map.
///
/// `public_message` is the generic, client-safe string returned from
/// `/status` and `/download` (no `anyhow` chains, OS strings, or filesystem
/// paths). `full_error` is logged at ERROR level for operator post-mortem
/// but never stored on the job or sent to the client.
pub(crate) fn set_job_error(
    jobs: &Jobs,
    jid: &str,
    public_message: &str,
    full_error: impl std::fmt::Display,
) {
    log::error!("Job {jid} failed: {full_error}");
    let mut map = lock_jobs(jobs);
    map.insert(
        jid.to_string(),
        JobState::Error {
            public_message: public_message.to_string(),
            created: Instant::now(),
        },
    );
}

/// Zip `world_dir` into a `.mcworld` (Bedrock) or `.zip` (Java) archive, persist
/// the containing temp directory to disk (so the file survives the `TempDir` drop),
/// and record `JobState::Done`.
///
/// On failure the archive is left on the filesystem (it may be partial) and
/// `JobState::Error` is recorded instead.
pub(crate) fn zip_and_persist(
    jobs: &Jobs,
    jid: &str,
    output_dir: tempfile::TempDir,
    world_dir: &Path,
    world_name: &str,
    edition: crate::world::Edition,
) {
    let extension = match edition {
        crate::world::Edition::Bedrock => "mcworld",
        crate::world::Edition::Java => "zip",
    };
    let archive_path = output_dir.path().join(format!("{world_name}.{extension}"));
    match zip_directory(world_dir, &archive_path) {
        Ok(()) => {
            let persisted_dir = output_dir.keep();
            let final_path = persisted_dir.join(format!("{world_name}.{extension}"));
            let mut map = lock_jobs(jobs);
            map.insert(
                jid.to_string(),
                JobState::Done {
                    path: final_path,
                    created: Instant::now(),
                },
            );
        }
        Err(e) => {
            set_job_error(
                jobs,
                jid,
                "archive creation failed",
                format!("Failed to create .{extension}: {e}"),
            );
        }
    }
}

/// Sanitise a user-supplied world name so it is safe to use as a directory
/// component and as an HTTP `Content-Disposition` filename.
///
/// Rules applied (in order):
/// 1. Strip any leading/trailing whitespace.
/// 2. Remove path separator characters (`/`, `\`), dot characters (`.`,
///    which could form `..` traversal sequences), ASCII control characters
///    (0x00–0x1F), and DEL (0x7F).
/// 3. Collapse any remaining runs of whitespace to a single space.
/// 4. If the result is empty after sanitisation, fall back to `"OSM World"`.
pub(crate) fn sanitize_world_name(name: &str) -> String {
    // Step 1: trim surrounding whitespace.
    let s = name.trim();

    // Step 2: remove unsafe characters character by character.
    let char_filtered: String = s
        .chars()
        .filter(|c| *c != '/' && *c != '\\' && *c != '.' && (*c as u32) >= 0x20 && *c != '\x7f')
        .collect();

    // Step 3: collapse whitespace runs.
    let collapsed: String = char_filtered
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Step 4: fall back to a safe default if nothing remains.
    if collapsed.is_empty() {
        "OSM World".to_string()
    } else {
        collapsed
    }
}

/// Create application state with a shared jobs map and concurrency semaphore.
pub(crate) fn build_state() -> (AppState, Jobs) {
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let state = AppState {
        jobs: jobs.clone(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS)),
    };
    (state, jobs)
}

/// Background task that periodically evicts completed/errored jobs older than
/// [`JOB_TTL`] and deletes their associated persisted temp directories.
///
/// Runs every 15 minutes.  The loop exits naturally when the server shuts down.
pub(crate) async fn job_eviction_task(jobs: Jobs) {
    let interval = Duration::from_secs(15 * 60);
    loop {
        tokio::time::sleep(interval).await;

        let now = Instant::now();
        let mut to_evict: Vec<(String, PathBuf)> = Vec::new();

        {
            let guard = lock_jobs(&jobs);
            for (id, state) in guard.iter() {
                let (age, path) = match state {
                    JobState::Done { created, path } => {
                        (now.duration_since(*created), path.clone())
                    }
                    JobState::Error { created, .. } => {
                        (now.duration_since(*created), PathBuf::new())
                    }
                    JobState::Running { .. } => continue,
                };
                if age >= JOB_TTL {
                    to_evict.push((id.clone(), path));
                }
            }
        }

        if to_evict.is_empty() {
            continue;
        }

        let mut guard = lock_jobs(&jobs);
        for (id, path) in to_evict {
            guard.remove(&id);
            if path.as_os_str().is_empty() {
                continue;
            }
            // The .mcworld file lives inside a temp dir; remove the whole parent dir.
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::remove_dir_all(parent) {
                    log::warn!(
                        "Job eviction: could not remove temp dir {}: {e}",
                        parent.display()
                    );
                } else {
                    log::info!("Job eviction: removed temp dir for job {id}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Jobs, lock_jobs, sanitize_world_name};
    use std::sync::{Arc, Mutex};

    #[test]
    fn normal_name_passes_through() {
        assert_eq!(sanitize_world_name("My City"), "My City");
    }

    #[test]
    fn path_traversal_dots_removed() {
        assert_eq!(sanitize_world_name("../../../etc/passwd"), "etcpasswd");
    }

    #[test]
    fn forward_slashes_removed() {
        assert_eq!(sanitize_world_name("foo/bar"), "foobar");
    }

    #[test]
    fn backslashes_removed() {
        assert_eq!(sanitize_world_name("foo\\bar"), "foobar");
    }

    #[test]
    fn dot_dot_literal_becomes_default() {
        assert_eq!(sanitize_world_name(".."), "OSM World");
    }

    #[test]
    fn empty_string_becomes_default() {
        assert_eq!(sanitize_world_name(""), "OSM World");
    }

    #[test]
    fn whitespace_only_becomes_default() {
        assert_eq!(sanitize_world_name("   "), "OSM World");
    }

    #[test]
    fn control_characters_removed() {
        assert_eq!(sanitize_world_name("hello\x00world\x1f!"), "helloworld!");
    }

    #[test]
    fn internal_whitespace_collapsed() {
        assert_eq!(sanitize_world_name("My   World"), "My World");
    }

    #[test]
    fn header_injection_chars_removed() {
        // Newlines and CRs inside a name could inject extra HTTP header lines
        assert_eq!(
            sanitize_world_name("world\r\nX-Evil: injected"),
            "worldX-Evil: injected"
        );
    }

    #[test]
    fn lock_jobs_recovers_from_poisoned_mutex_instead_of_panicking() {
        let jobs: Jobs = Arc::new(Mutex::new(std::collections::HashMap::new()));
        // Simulate a panicked worker by deliberately poisoning the lock.
        let jobs_for_panic = jobs.clone();
        let panic_thread = std::thread::spawn(move || {
            let _guard = jobs_for_panic.lock().unwrap();
            panic!("simulated worker panic while holding the lock");
        });
        assert!(
            panic_thread.join().is_err(),
            "sanity: the thread should have panicked"
        );

        // The mutex is now poisoned. `lock_jobs` must NOT panic — it should
        // recover the inner map and let the server keep serving requests.
        let recovered = lock_jobs(&jobs);
        assert!(
            recovered.is_empty(),
            "poison recovery should expose the underlying map"
        );
    }
}
