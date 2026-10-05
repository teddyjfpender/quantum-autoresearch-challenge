//! Phase timers for the notes (`FEMOCO_FASTSIM_PROF=1`; stderr only, never in `score.json`).
//! Off by default; when off, each probe is one relaxed load of a cached flag.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

pub const TRACK: usize = 0;
pub const GRAM: usize = 1;
pub const ELIM: usize = 2;
pub const AD: usize = 3;
pub const EXEC: usize = 4;
pub const KEYS: usize = 5;
pub const FOLD: usize = 6;
const NAMES: [&str; 7] = ["track", "gram", "elim", "ad", "exec", "keys", "fold"];

static NS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

#[must_use]
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FEMOCO_FASTSIM_PROF").is_some())
}

/// Starts a probe (`None` when profiling is off).
#[inline]
#[must_use]
pub fn start() -> Option<Instant> {
    on().then(Instant::now)
}

/// Adds the time since `t` to phase `i`.
#[inline]
pub fn stop(i: usize, t: Option<Instant>) {
    if let Some(t) = t {
        NS[i].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

/// Prints and clears the totals.
pub fn report() {
    if !on() {
        return;
    }
    let parts: Vec<String> = NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| {
            format!(
                "{n} {:.3} s",
                NS[i].swap(0, Ordering::Relaxed) as f64 * 1e-9
            )
        })
        .collect();
    eprintln!("fastsim prof (thread-seconds): {}", parts.join(", "));
}
