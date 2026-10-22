//! Benchmarks with the machine to themselves: `dv cargo bench` waits for the builds and tests
//! running through `dv cargo` (any dv's Claude, any repo: they're this user's, machine wide)
//! and holds new ones off till it's done; they wait out a benchmark, and run alongside each
//! other as ever.
//!
//! Two lock files: builds hold `run` shared; a benchmark holds it exclusive. It takes `gate`
//! first, and holds it while it waits, so builds starting meanwhile (they pass `gate` on their
//! way in) wait behind it rather than keep it waiting for good. A lock is cargo's to hold
//! (its stdin): it lasts as long as cargo runs, dv killed or not. Who's benchmarking, or
//! waiting to, is written beside them (`<lock>.who`), for the others' messages.
use anyhow::Result;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long, all told, either waits before giving up, saying why: under Claude's commands' 2
/// minutes.
const WAIT: Duration = Duration::from_secs(100);

/// A lock held, for cargo to hold on (`stdin`); what it wrote of itself, taken away after.
pub struct Held {
    run: File,
    who: Option<PathBuf>,
}

impl Held {
    /// For cargo's stdin: the lock with it (the same open file), for as long as it runs.
    pub fn stdin(&self) -> std::process::Stdio {
        self.run.try_clone().map_or_else(|_| std::process::Stdio::null(), std::process::Stdio::from)
    }
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

fn open(d: &Path, name: &str) -> std::io::Result<File> {
    File::options().create(true).truncate(false).write(true).open(d.join(name))
}

/// What a benchmark wrote of itself beside `lock` (`<lock>.who`), while it's alive.
fn who(d: &Path, lock: &str) -> Option<String> {
    let text = std::fs::read_to_string(d.join(format!("{lock}.who"))).ok()?;
    let (pid, what) = text.split_once('\n')?;
    crate::alive(pid.trim().parse().ok()?).then(|| what.trim().to_owned())
}

/// Writes what this benchmark is beside `lock`, for the others to say.
fn write_who(d: &Path, lock: &str, what: &str) -> PathBuf {
    let p = d.join(format!("{lock}.who"));
    let cwd = std::env::current_dir().unwrap_or_default();
    _ = std::fs::write(&p, format!("{}\n`{what}` in {}", std::process::id(), cwd.display()));
    p
}

/// What waiting came to: taken; given up (why); or the lock can't be had at all (this
/// filesystem's, say: why).
enum Waited {
    Got,
    GaveUp,
    Broken(std::io::Error),
}

/// `lock` tried till it takes or `deadline` passes, saying once (`waiting`) that it waits.
fn wait(deadline: Instant, mut lock: impl FnMut() -> Result<(), TryLockError>, waiting: impl Fn() -> String) -> Waited {
    let mut said = false;
    loop {
        match lock() {
            Ok(()) => return Waited::Got,
            Err(TryLockError::Error(e)) => return Waited::Broken(e),
            Err(TryLockError::WouldBlock) if Instant::now() >= deadline => return Waited::GaveUp,
            Err(TryLockError::WouldBlock) => {}
        }
        if !said {
            eprintln!("dv: {}", waiting());
            said = true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// What a build waits for, said: the benchmark running, else the one waiting to.
fn holder(d: &Path) -> String {
    match (who(d, "bench.run"), who(d, "bench.gate")) {
        (Some(w), _) => format!("a benchmark ({w})"),
        (None, Some(w)) => format!("a benchmark about to start ({w}, waiting for builds to finish)"),
        (None, None) => "a benchmark".into(),
    }
}

/// The lock for a build or test, once no benchmark has the machine (or is waiting for it).
/// Without a lock to be had (no data directory, a filesystem without locks): `None`, said,
/// and it runs all the same.
pub fn build() -> Result<Option<Held>> {
    let Ok(d) = dir() else { return Ok(unlocked("dv's data directory isn't there")) };
    build_in(&d)
}

fn unlocked(why: &str) -> Option<Held> {
    eprintln!("dv: not keeping out of benchmarks' way ({why})");
    None
}

fn build_in(d: &Path) -> Result<Option<Held>> {
    let (Ok(gate), Ok(run)) = (open(d, "bench.gate"), open(d, "bench.run")) else { return Ok(unlocked("its lock files can't be opened")) };
    let deadline = Instant::now() + WAIT;
    let busy = || format!("{} has the machine: this waits for it", holder(d));
    for lock in [&gate, &run] {
        match wait(deadline, || lock.try_lock_shared(), busy) {
            Waited::Got => {}
            Waited::Broken(e) => return Ok(unlocked(&e.to_string())),
            Waited::GaveUp => anyhow::bail!("{} still has the machine: run this again once it's done (a benchmark needs it quiet)", holder(d)),
        }
    }
    drop(gate);
    Ok(Some(Held { run, who: None }))
}

/// The lock for a benchmark, once the builds and tests under way are done; none starts till
/// it is. (`None`, said: as `build`'s.)
pub fn bench(what: &str) -> Result<Option<Held>> {
    let Ok(d) = dir() else { return Ok(unlocked("dv's data directory isn't there")) };
    bench_in(&d, what)
}

fn bench_in(d: &Path, what: &str) -> Result<Option<Held>> {
    let (Ok(gate), Ok(run)) = (open(d, "bench.gate"), open(d, "bench.run")) else { return Ok(unlocked("its lock files can't be opened")) };
    let deadline = Instant::now() + WAIT;
    match wait(deadline, || gate.try_lock(), || format!("waiting for {} first", holder(d))) {
        Waited::Got => {}
        Waited::Broken(e) => return Ok(unlocked(&e.to_string())),
        Waited::GaveUp => anyhow::bail!("{} still has the machine: run this again once it's done", holder(d)),
    }
    let waiting_who = write_who(d, "bench.gate", what);
    let on_run = || match who(d, "bench.run") {
        Some(w) => format!("another benchmark ({w})"),
        None => "the builds and tests running elsewhere (other dv's Claude)".into(),
    };
    let got = wait(deadline, || run.try_lock(), || format!("waiting for {} to finish; new ones wait for this", on_run()));
    _ = std::fs::remove_file(&waiting_who);
    match got {
        Waited::Got => {}
        Waited::Broken(e) => return Ok(unlocked(&e.to_string())),
        Waited::GaveUp => anyhow::bail!("{} still running: run this again shortly", on_run()),
    }
    let who = write_who(d, "bench.run", what);
    drop(gate); // (`run` held exclusive keeps builds out; others' benchmarks wait on it too)
    Ok(Some(Held { run, who: Some(who) }))
}

#[test]
fn a_benchmark_waits_for_builds_and_holds_them_off() {
    let d = std::env::temp_dir().join(format!("dv-bench-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let b = build_in(&d).unwrap().unwrap();
    let b2 = build_in(&d).unwrap().unwrap(); // builds run alongside each other
    // A benchmark can't start while they run…
    let run = open(&d, "bench.run").unwrap();
    assert!(run.try_lock().is_err());
    drop((b, b2));
    let held = bench_in(&d, "cargo bench").unwrap().unwrap();
    assert!(holder(&d).starts_with("a benchmark (`cargo bench` in "), "{}", holder(&d));
    // …and while it runs, a build can't start; nor while cargo, holding it, outlives dv.
    let run2 = open(&d, "bench.run").unwrap();
    assert!(run2.try_lock_shared().is_err());
    let cargo = held.stdin();
    drop(held);
    assert!(run2.try_lock_shared().is_err(), "held while cargo has it");
    drop(cargo);
    // (Let go: a moment, while another test's child, forked meanwhile, still has it till its exec.)
    let free = (0..100).any(|_| run2.try_lock_shared().is_ok() || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        false
    });
    assert!(free);
    assert_eq!(holder(&d), "a benchmark"); // (what it wrote of itself, gone)
    _ = std::fs::remove_dir_all(&d);
}
