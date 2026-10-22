//! Benchmarks with the machine to themselves: `dv cargo bench` waits for the builds and tests
//! running through `dv cargo` (any dv's Claude, any repo: they're this user's, machine wide)
//! and holds new ones off till it's done; they wait out a benchmark, and run alongside each
//! other as ever.
//!
//! Two lock files: builds hold `run` shared; a benchmark holds it exclusive. It takes `gate`
//! first, and holds it while it waits, so builds starting meanwhile (they pass `gate` on their
//! way in) wait behind it rather than keep it waiting for good.
use anyhow::{Context, Result};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long either waits before giving up, saying why: under Claude's commands' 2 minutes.
const WAIT: Duration = Duration::from_secs(100);

/// Held while it runs (dropped: let go).
pub struct Held {
    _run: File,
    who: Option<PathBuf>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(p) = &self.who {
            _ = std::fs::remove_file(p);
        }
    }
}

/// Where the locks are: dv's data directory, this user's on this machine.
fn dir() -> Result<PathBuf> {
    let d = crate::claude::data_home()?.join("locks");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn open(d: &Path, name: &str) -> Result<File> {
    let p = d.join(name);
    File::options().create(true).truncate(false).write(true).open(&p).with_context(|| format!("opening {}", p.display()))
}

/// Who's benchmarking (for a build waiting on it to say), as they wrote it.
fn who(d: &Path) -> String {
    std::fs::read_to_string(d.join("bench.who")).map_or_else(|_| "a benchmark".into(), |w| w.trim().to_owned())
}

/// `lock` tried till it takes, or `WAIT` is up: then `why`. Says once that it's waiting.
fn wait(mut lock: impl FnMut() -> bool, waiting: impl Fn() -> String, why: impl Fn() -> String) -> Result<()> {
    let start = Instant::now();
    let mut said = false;
    while !lock() {
        if !said {
            eprintln!("dv: {}", waiting());
            said = true;
        }
        anyhow::ensure!(start.elapsed() < WAIT, "{}", why());
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// For a build or test: once no benchmark has the machine (or is waiting for it).
pub fn build() -> Result<Held> {
    build_in(&dir()?)
}

fn build_in(d: &Path) -> Result<Held> {
    let (gate, run) = (open(d, "bench.gate")?, open(d, "bench.run")?);
    let busy = || format!("{} has the machine: this waits for it to finish", who(d));
    let gave_up = || format!("{} still has the machine: run this again once it's done (a benchmark needs it quiet)", who(d));
    wait(|| gate.try_lock_shared().is_ok(), busy, gave_up)?;
    wait(|| run.try_lock_shared().is_ok(), busy, gave_up)?;
    drop(gate);
    Ok(Held { _run: run, who: None })
}

/// For a benchmark: once the builds and tests under way are done, and none starts till it is.
pub fn bench(what: &str) -> Result<Held> {
    bench_in(&dir()?, what)
}

fn bench_in(d: &Path, what: &str) -> Result<Held> {
    let (gate, run) = (open(d, "bench.gate")?, open(d, "bench.run")?);
    let waiting = || "waiting for the builds and tests running elsewhere (other dv's Claude) to finish; new ones wait for this".to_owned();
    wait(|| gate.try_lock().is_ok(), || format!("waiting for {} first", who(d)), || format!("{} has the machine: run this again once it's done", who(d)))?;
    wait(|| run.try_lock().is_ok(), waiting, || "builds and tests elsewhere are still running: run this again shortly".into())?;
    let who = d.join("bench.who");
    let cwd = std::env::current_dir().unwrap_or_default();
    _ = std::fs::write(&who, format!("`{what}` in {} (pid {})", cwd.display(), std::process::id()));
    drop(gate); // (`run` held exclusive keeps builds out; others' benchmarks wait on it too)
    Ok(Held { _run: run, who: Some(who) })
}

#[test]
fn a_benchmark_waits_for_builds_and_holds_them_off() {
    let d = std::env::temp_dir().join(format!("dv-bench-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let b = build_in(&d).unwrap();
    let b2 = build_in(&d).unwrap(); // builds run alongside each other
    // A benchmark can't start while they run…
    let run = open(&d, "bench.run").unwrap();
    assert!(run.try_lock().is_err());
    drop((b, b2));
    let held = bench_in(&d, "cargo bench").unwrap();
    assert!(who(&d).starts_with("`cargo bench` in "));
    // …and while it runs, a build can't start.
    let run2 = open(&d, "bench.run").unwrap();
    assert!(run2.try_lock_shared().is_err());
    drop(held);
    assert!(run2.try_lock_shared().is_ok());
    _ = std::fs::remove_dir_all(&d);
}
