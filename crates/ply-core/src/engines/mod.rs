pub mod fuzz;
pub mod kani;
pub mod mutants;

use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

// **`cargo` is spawned by bare name, on purpose** (2026-09-08).
//
// It is tempting to run the binary `CARGO` names -- cargo sets it for
// everything it launches -- since that removes a `PATH` lookup and, on a
// machine with two toolchains, sounds more precise. It was tried, and it
// is wrong: the `cargo` found on `PATH` is rustup's proxy, and the proxy
// is what reads a crate's own `rust-toolchain.toml` and dispatches to the
// toolchain that crate pins. Naming a concrete binary skips that, so a
// crate pinning its own compiler would be checked with a different one --
// and the compiler is a fingerprint input (§5.2a), so the stored result
// would be attributed to a toolchain that never touched the code.
//
// `tests/e2e/tests/toolchainprobe_fixture.rs` is the test that catches
// this; its own doc explains why it strips `RUSTUP_TOOLCHAIN` so that
// directory-based resolution is what runs.

/// How often [`run_with_timeout`] polls the child for exit -- a compromise
/// between wasted CPU (too tight) and slack in when a killed run is
/// noticed (too loose). Chosen small enough that no caller's wall-clock
/// budget is meaningfully overshot by it.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

static CANCELLATION_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static CANCELLATION_MANAGED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn begin_cancellation_scope() {
    reset_cancellation();
    CANCELLATION_MANAGED.store(true, Ordering::SeqCst);
}

pub fn end_cancellation_scope() {
    CANCELLATION_MANAGED.store(false, Ordering::SeqCst);
    reset_cancellation();
}

pub fn request_cancellation() {
    CANCELLATION_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn reset_cancellation() {
    CANCELLATION_REQUESTED.store(false, Ordering::SeqCst);
}

pub fn cancellation_requested() -> bool {
    CANCELLATION_REQUESTED.load(Ordering::SeqCst)
}

pub fn cancellation_flag() -> &'static std::sync::atomic::AtomicBool {
    &CANCELLATION_REQUESTED
}

fn cancellation_managed() -> bool {
    CANCELLATION_MANAGED.load(Ordering::SeqCst)
}

/// One finished (or killed) subprocess, captured without ever depending on
/// an external `timeout` utility -- see [`run_with_timeout`].
#[derive(Debug)]
pub struct TimedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
    /// Whether the process was still running when its budget expired and
    /// had to be killed. Replaces the old convention of reading GNU
    /// `timeout`'s exit code 124 off `status` -- that code is never
    /// produced by anything here, so a caller must read this flag, not
    /// `status`, to learn whether the run timed out.
    pub timed_out: bool,
    /// The user interrupted the run. Kept separate from timeout because one
    /// is a run-level cancellation and the other is an engine outcome.
    pub cancelled: bool,
}

impl TimedOutput {
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A scratch file that removes itself on drop, best-effort -- cleanup that
/// fires whether the function this guards returns normally, early via `?`,
/// or panics, without needing a dependency on the `tempfile` crate (a
/// dev-only dependency in this workspace; this path also runs in the
/// shipped binary).
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A unique path under the OS temp directory for one captured stream.
/// Uniqueness comes from the process id, a monotonic counter, and the
/// current time, which is enough to make a collision practically
/// impossible without needing an atomic create-if-absent primitive.
fn scratch_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "ply-engine-{}-{tag}-{nanos}-{n}.txt",
        std::process::id()
    ))
}

/// Reaps a child that has already been killed (or is otherwise exiting),
/// tolerating the (benign) race where it finishes between the kill and this
/// wait.
fn reap(child: &mut Child) -> Result<ExitStatus> {
    child
        .wait()
        .context("waiting for the child process to exit after killing it")
}

/// How many times [`kill_descendants`] re-lists the process tree before
/// giving up. Each extra sweep catches a descendant that forked late in
/// the previous one, at the cost of one more `ps` call and
/// [`SWEEP_GAP`]'s delay -- kept small so an expired budget still returns
/// promptly.
const KILL_SWEEPS: u32 = 3;

/// The pause between sweeps in [`kill_descendants`] -- long enough to let a
/// process that just forked actually appear in the next `ps` listing,
/// short enough that three sweeps still add well under a second.
const SWEEP_GAP: Duration = Duration::from_millis(20);

/// Kills every live descendant of `root_pid`, but not `root_pid` itself --
/// callers that also want the root gone (this module's caller does, via
/// `Child::kill`, which already knows how to target that specific pid) do
/// so separately. Walks the process tree with `ps` rather than reading
/// `/proc`, since macOS has no `/proc` to read.
///
/// Listing a live process tree and then acting on it is inherently racy: a
/// process forked in the gap between one sweep's snapshot and its kills
/// has reparented to its grandparent by the time the next sweep looks, and
/// so is missed by it too unless it is *also* still a descendant of
/// `root_pid` through some other, already-listed ancestor. Re-listing
/// [`KILL_SWEEPS`] times narrows that gap -- a child that appears just
/// after one snapshot is caught by the next -- but does not close it: a
/// process forking children fast enough, right up to and past each sweep,
/// could still leave one behind. The private process group below closes that
/// race for ordinary descendants. This walk remains useful for a tool that
/// deliberately moved a child into another group while its ancestry is
/// still visible -- see [`run_with_timeout`]'s doc comment.
fn kill_descendants(root_pid: u32) {
    for sweep in 0..KILL_SWEEPS {
        let pairs = parent_pid_pairs();
        let descendants = descendants_of(root_pid, &pairs);
        if descendants.is_empty() {
            break;
        }
        for pid in descendants {
            // SAFETY: `kill` with a plain signal number only touches the
            // kernel's process table for the given pid; it dereferences no
            // pointer that could be invalid, so this is safe to call for
            // any pid value, including one that has already exited (that
            // call simply fails and is ignored -- the process is gone
            // either way, which is what this function is trying to
            // achieve).
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        if sweep + 1 < KILL_SWEEPS {
            std::thread::sleep(SWEEP_GAP);
        }
    }
}

/// Kills the process group created for one controlled command. Unlike a
/// parent/child walk, the group remains addressable after its leader exits,
/// so a crashing engine cannot orphan a compiler or solver before Ply sees
/// it. The descendant walk remains as a fallback for children that moved
/// themselves out of the group.
#[cfg(unix)]
fn kill_process_group(root_pid: u32) {
    // SAFETY: a negative pid asks `kill` to signal the process group whose
    // id is `root_pid`. `run_controlled` puts each child in exactly that
    // private group before spawning it, so this can never target Ply's own
    // foreground group.
    unsafe {
        libc::kill(-(root_pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_: u32) {}

/// `(pid, ppid)` for every process this user can currently see, read via
/// `ps` rather than `/proc` so the same code runs on macOS and Linux. A
/// line `ps` prints that this goes on to fail to parse -- most often
/// because the process it named has already exited -- is skipped rather
/// than treated as an error: a process disappearing between `ps` listing
/// it and this reading the listing is normal, not exceptional.
fn parent_pid_pairs() -> Vec<(u32, u32)> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

/// Every pid in `pairs` whose ancestry leads back to `root`, found by
/// repeatedly widening a frontier of known-descendant pids until a pass
/// turns up nothing new. `root` itself is never included in the result.
fn descendants_of(root: u32, pairs: &[(u32, u32)]) -> Vec<u32> {
    let mut found = Vec::new();
    let mut frontier = vec![root];
    loop {
        let next: Vec<u32> = pairs
            .iter()
            .filter(|&&(pid, ppid)| {
                frontier.contains(&ppid) && pid != root && !found.contains(&pid)
            })
            .map(|&(pid, _)| pid)
            .collect();
        if next.is_empty() {
            break;
        }
        found.extend(&next);
        frontier = next;
    }
    found
}

#[cfg(test)]
mod descendants_of_tests {
    use super::descendants_of;

    /// The exact shape `kill_descendants` relies on: a multi-generation
    /// tree (root -> sh -> sleep) must be found in full, in ancestry order
    /// through the middle generation, not just the root's direct children.
    #[test]
    fn finds_grandchildren_through_an_intermediate_process() {
        let pairs = [(100, 1), (200, 100), (300, 200), (999, 1)];
        let mut found = descendants_of(100, &pairs);
        found.sort_unstable();
        assert_eq!(found, vec![200, 300]);
    }

    /// A process with no children at all contributes nothing -- there is
    /// no tree to walk, so the result is empty rather than, say, including
    /// the root by mistake.
    #[test]
    fn a_childless_root_has_no_descendants() {
        let pairs = [(1, 0), (999, 1)];
        assert!(descendants_of(500, &pairs).is_empty());
    }

    /// `root` itself must never come back as its own descendant, even if
    /// `pairs` (built from real `ps` output, which can be racy) somehow
    /// contained a self-referential or duplicate entry.
    #[test]
    fn the_root_pid_is_never_included_in_its_own_result() {
        let pairs = [(100, 1), (100, 100), (200, 100)];
        let found = descendants_of(100, &pairs);
        assert!(!found.contains(&100));
        assert!(found.contains(&200));
    }
}

/// Runs `cmd` and enforces `budget` **in this process**, never by shelling
/// out to a `timeout`/`gtimeout` binary -- macOS ships neither, so wrapping
/// the real command in one made every engine invocation fail to spawn at
/// all on macOS, with the error naming the wrapper's argument (`cargo
/// test ...`) rather than the program that actually could not be launched
/// (`timeout`).
///
/// No thread is spawned to enforce this: `cmd`'s stdout and stderr are
/// redirected to temporary files (never a pipe -- a pipe plus polling
/// `try_wait` can deadlock once the child fills the pipe's buffer and blocks
/// writing to it, with nothing draining the other end), and this thread
/// polls [`Child::try_wait`] in a sleep loop. On expiry the whole process
/// tree rooted at the child is killed (see [`kill_descendants`]) and the
/// child is reaped, `timed_out` is set, and the two files are read back
/// into memory.
///
/// Every controlled command gets its own process group. The CLI's
/// SIGINT/SIGTERM handler turns terminal interruption into the shared
/// cancellation flag; this loop then kills the complete group. Group
/// ownership is also used after an unsuccessful exit, because an engine can
/// crash after spawning a compiler or solver and leave that child alive even
/// though the direct process has already been reaped. Successful tools are
/// allowed to leave an intentional daemon behind. The explicit process-tree
/// walk remains a fallback for a tool that deliberately leaves its group.
pub fn run_with_timeout(cmd: &mut Command, budget: Duration) -> Result<TimedOutput> {
    run_controlled(cmd, Some(budget))
}

/// Runs until the command finishes or the current verification run is
/// interrupted. Used by engines that enforce their own semantic timeout.
pub fn run_until_cancelled(cmd: &mut Command) -> Result<TimedOutput> {
    run_controlled(cmd, None)
}

fn run_controlled(cmd: &mut Command, budget: Option<Duration>) -> Result<TimedOutput> {
    if cancellation_requested() {
        anyhow::bail!("verification was interrupted before starting another subprocess");
    }
    let program = cmd.get_program().to_string_lossy().into_owned();

    let stdout_path = scratch_path("stdout");
    let stderr_path = scratch_path("stderr");
    let _stdout_guard = ScratchFile(stdout_path.clone());
    let _stderr_guard = ScratchFile(stderr_path.clone());

    let stdout_file = std::fs::File::create(&stdout_path)
        .context("creating a scratch file to capture the child process's stdout")?;
    let stderr_file = std::fs::File::create(&stderr_path)
        .context("creating a scratch file to capture the child process's stderr")?;

    let owns_process_group = cancellation_managed();
    #[cfg(unix)]
    if owns_process_group {
        cmd.process_group(0);
    }
    let mut child = cmd
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .with_context(|| {
            format!("could not start `{program}` -- check that it is installed and on your PATH")
        })?;

    let start = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if owns_process_group && !status.success() {
                    kill_process_group(child.id());
                }
                break status;
            }
            Ok(None) => {}
            // Kill and reap before propagating: returning through `?` here
            // would leave the child running with nothing ever waiting on it
            // -- an orphan that outlives the budget it was spawned under.
            Err(error) => {
                kill_descendants(child.id());
                if owns_process_group {
                    kill_process_group(child.id());
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error)
                    .with_context(|| format!("checking whether `{program}` has finished"));
            }
        }
        let interrupted = cancellation_requested();
        let expired = budget.is_some_and(|budget| start.elapsed() >= budget);
        if interrupted || expired {
            cancelled = interrupted;
            timed_out = expired && !interrupted;
            // Descendants before the direct child: killing `child` first
            // would let the kernel reparent any still-live descendant to
            // init before `kill_descendants` gets to look for it, which
            // erases exactly the parent-child chain it walks to find one.
            kill_descendants(child.id());
            if owns_process_group {
                kill_process_group(child.id());
            }
            let _ = child.kill();
            break reap(&mut child)?;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    std::fs::File::open(&stdout_path)
        .and_then(|mut f| f.read_to_end(&mut stdout))
        .with_context(|| format!("reading back `{program}`'s captured stdout"))?;
    std::fs::File::open(&stderr_path)
        .and_then(|mut f| f.read_to_end(&mut stderr))
        .with_context(|| format!("reading back `{program}`'s captured stderr"))?;

    Ok(TimedOutput {
        stdout,
        stderr,
        status,
        timed_out,
        cancelled,
    })
}

/// Every engine Ply drives is a program whose output Ply then *reads* --
/// which compiler error, which counterexample, which mutant survived. That
/// reading is line-oriented and matches on what a line starts with, so it
/// silently stops working the moment the tool decides to colour its output.
///
/// It is not hypothetical. This project's own CI sets
/// `CARGO_TERM_COLOR=always`, so every `error:` line reached Ply as
/// `\x1b[1m\x1b[91merror\x1b[0m: ...`, matched nothing, and a build failure
/// with a perfectly clear cause was reported as "the compiler gave no
/// specific error line" -- Ply's honest fallback for a failure it genuinely
/// cannot attribute, taken for one it could have attributed exactly. Worse
/// than a wrong answer: a true sentence used in the wrong place, which
/// looks like the tool working.
///
/// So output is stripped of ANSI escape sequences before anything parses
/// it. Passing `--color=never` would fix cargo alone; every engine's output
/// goes through here instead, because the next tool to add colour should
/// not cost another silent regression.
///
/// Handles the two forms that matter for terminal output: CSI sequences
/// (`ESC [ ... final-byte`, which covers every colour and style code) and
/// OSC sequences (`ESC ] ... BEL` or `ESC ] ... ESC \`, which is how
/// hyperlinks arrive). A bare `ESC` followed by anything else drops the
/// escape and keeps the character, so nothing is ever swallowed wholesale.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // CSI: parameter and intermediate bytes, then one final
                // byte in 0x40..=0x7e ends it.
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC: runs until BEL, or until the ST pair `ESC \`.
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod strip_ansi_tests {
    use super::strip_ansi;

    /// The exact shape that made this repository's own CI report a
    /// perfectly attributable build failure as unattributable.
    #[test]
    fn a_coloured_compiler_error_still_begins_with_the_word_error() {
        let coloured = "\u{1b}[1m\u{1b}[91merror\u{1b}[0m\u{1b}[1m: expected `;`\u{1b}[0m";
        assert_eq!(strip_ansi(coloured), "error: expected `;`");
        assert!(strip_ansi(coloured).trim().starts_with("error"));
    }

    /// The span line is what attributes an error to the function that
    /// caused it, so it has to survive too.
    #[test]
    fn a_coloured_span_line_still_points_at_a_file_and_line() {
        let coloured = "  \u{1b}[1m\u{1b}[94m-->\u{1b}[0m src/lib.rs:42:9";
        assert_eq!(strip_ansi(coloured), "  --> src/lib.rs:42:9");
    }

    #[test]
    fn plain_output_is_returned_unchanged() {
        let plain = "error[E0308]: mismatched types\n  --> src/lib.rs:1:1\n";
        assert_eq!(strip_ansi(plain), plain);
    }

    /// A terminal hyperlink is an OSC sequence, not a CSI one, and it
    /// carries a URL that must not be left behind as text.
    #[test]
    fn an_osc_hyperlink_is_removed_along_with_its_url() {
        let linked = "see \u{1b}]8;;https://example.com\u{7}the docs\u{1b}]8;;\u{7}";
        assert_eq!(strip_ansi(linked), "see the docs");
    }

    /// Nothing is swallowed wholesale: an escape Ply does not recognise
    /// costs one byte, not the rest of the line.
    #[test]
    fn an_unrecognised_escape_drops_only_itself() {
        assert_eq!(strip_ansi("a\u{1b}Zb"), "aZb");
    }
}

#[cfg(test)]
mod run_with_timeout_tests {
    use super::{
        begin_cancellation_scope, end_cancellation_scope, request_cancellation,
        run_until_cancelled, run_with_timeout,
    };
    use std::process::Command;
    use std::time::{Duration, Instant};

    const CANCELLATION_CHILD_ENV: &str = "PLY_TEST_CANCELLATION_CHILD";
    const CANCELLATION_TEST_NAME: &str = "engines::run_with_timeout_tests::cancellation_kills_an_active_process_tree_and_is_reported_separately";

    /// Serializes every test in this module against state that is shared
    /// process-wide rather than per-test: the temp-file namespace
    /// `scratch_path` writes into (every test transiently populates it,
    /// and one test below counts entries in it) and the `PATH` environment
    /// variable (mutated by another test below). `cargo test` runs these on
    /// separate threads by default, so without this lock they can observe
    /// each other's scratch files or PATH -- exactly the kind of cross-talk
    /// that would make either test pass or fail for the wrong reason.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct ManagedCancellation;

    impl ManagedCancellation {
        fn begin() -> Self {
            begin_cancellation_scope();
            Self
        }
    }

    impl Drop for ManagedCancellation {
        fn drop(&mut self) {
            end_cancellation_scope();
        }
    }

    /// A command that finishes well inside its budget must be run for
    /// real, not skipped or faked -- its actual stdout and exit status
    /// come back untouched.
    #[test]
    fn a_command_that_finishes_in_time_returns_its_real_stdout_and_status() {
        let _guard = test_lock();
        let mut cmd = Command::new("/bin/echo");
        cmd.arg("hello ply");
        let out = run_with_timeout(&mut cmd, Duration::from_secs(5)).unwrap();
        assert!(
            !out.timed_out,
            "a fast command must not be reported as timed out"
        );
        assert!(out.status.success());
        assert_eq!(out.stdout_string().trim(), "hello ply");
    }

    /// A command's own non-zero exit is a real failure, not a timeout --
    /// the two must never be conflated.
    #[test]
    fn a_failing_command_reports_its_real_exit_status_not_a_timeout() {
        let _guard = test_lock();
        let mut cmd = Command::new("/usr/bin/false");
        let out = run_with_timeout(&mut cmd, Duration::from_secs(5)).unwrap();
        assert!(!out.timed_out);
        assert!(!out.status.success());
    }

    /// A command that outlives its budget is killed -- the call returns
    /// promptly (long before the child's own sleep would have finished)
    /// and reports expiry through `timed_out`, never by inventing an exit
    /// code.
    #[test]
    fn a_command_that_outlives_its_budget_is_killed_and_reported_as_timed_out() {
        let _guard = test_lock();
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        let start = Instant::now();
        let out = run_with_timeout(&mut cmd, Duration::from_millis(150)).unwrap();
        assert!(
            out.timed_out,
            "a run that outlived its budget must be reported as timed out"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the helper must kill the child rather than waiting out its full sleep, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn cancellation_kills_an_active_process_tree_and_is_reported_separately() {
        // Cancellation is deliberately process-wide in production so every
        // active worker stops together. Exercise that global state in an
        // isolated copy of this test binary: otherwise libtest may run an
        // unrelated subprocess test concurrently and make it observe this
        // test's synthetic interrupt.
        if std::env::var_os(CANCELLATION_CHILD_ENV).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CANCELLATION_TEST_NAME, "--nocapture"])
                .env(CANCELLATION_CHILD_ENV, "1")
                .status()
                .unwrap();
            assert!(status.success(), "isolated cancellation test failed");
            return;
        }

        let _guard = test_lock();
        let _scope = ManagedCancellation::begin();
        let trigger = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            request_cancellation();
        });
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "/bin/sleep 300 & wait"]);
        let out = run_until_cancelled(&mut cmd).unwrap();
        trigger.join().unwrap();
        assert!(out.cancelled);
        assert!(!out.timed_out);
    }

    #[test]
    fn a_crashed_engine_cannot_leave_its_child_running() {
        let _guard = test_lock();
        let _scope = ManagedCancellation::begin();
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("child.pid");
        let mut cmd = Command::new("/bin/sh");
        cmd.args([
            "-c",
            &format!("/bin/sleep 300 & echo $! > {}; exit 9", pid_path.display()),
        ]);
        let out = run_until_cancelled(&mut cmd).unwrap();
        assert_eq!(out.status.code(), Some(9));
        let pid: libc::pid_t = std::fs::read_to_string(pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "engine child {pid} survived its parent's crash"
        );
    }

    /// The bug this test exists to catch: `run_with_timeout` used to kill
    /// only the one process it spawned directly. Every command Ply budgets
    /// is `cargo ...`, and cargo always spawns the real prover or test
    /// binary as a child of its own, so killing just the direct child let
    /// the actual hung process -- the thing the budget exists to stop --
    /// survive and grind on forever, invisibly.
    ///
    /// Reproduced with shell primitives instead of cargo: `sh` is the
    /// direct child `run_with_timeout` tracks; it backgrounds `sleep 300`
    /// (the grandchild), writes that grandchild's real pid to a file so
    /// this test can check on it independently of anything
    /// `run_with_timeout` captures, and then blocks on `wait` -- so `sh`
    /// itself outlives the budget exactly like a hung `cargo` would.
    #[test]
    fn a_timed_out_run_kills_the_whole_process_tree_not_just_its_direct_child() {
        let _guard = test_lock();
        let pid_file = std::env::temp_dir().join(format!(
            "ply-engine-test-grandchild-pid-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // Absolute paths throughout: a sibling test in this module clears
        // this process's PATH for the duration of a call, and this test can
        // run concurrently with it. `sleep` resolved via PATH lookup would
        // then fail to start, `$!` would capture nothing meaningful, and
        // `wait` would return immediately -- passing for the wrong reason.
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(format!(
            "/bin/sleep 300 & echo $! > {} ; wait",
            pid_file.display()
        ));

        let out = run_with_timeout(&mut cmd, Duration::from_millis(200)).unwrap();
        assert!(
            out.timed_out,
            "the sh wrapper outlives its budget and must be reported as timed out"
        );

        let grandchild_pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("sh must have written the backgrounded sleep's pid before its budget expired")
            .trim()
            .parse()
            .expect("the captured pid must be a plain integer");
        let _ = std::fs::remove_file(&pid_file);

        // SIGKILL delivery is not instantaneous, so poll briefly rather than
        // checking exactly once -- but bounded, so a real regression here
        // fails the test instead of hanging the suite.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut alive = process_is_alive(grandchild_pid);
        while alive && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            alive = process_is_alive(grandchild_pid);
        }

        assert!(
            !alive,
            "grandchild pid {grandchild_pid} (the backgrounded `sleep`) is still running \
             after its parent's budget expired -- the timeout killed the direct child but \
             let the real hung process escape"
        );
    }

    /// `kill(pid, 0)` sends no signal; it only reports whether the pid is
    /// live and reachable, which is exactly what checking "is this process
    /// really gone" needs -- as opposed to grepping `ps` output or trusting
    /// a log line, either of which could pass while the process lives on.
    fn process_is_alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Neither path through `run_with_timeout` -- finishing in time, or
    /// being killed for outliving its budget -- may leave its stdout/stderr
    /// capture files behind. Scans for the exact name shape `scratch_path`
    /// produces (tagged with this test process's own pid) rather than
    /// trusting the `Drop` guard blindly.
    #[test]
    fn no_scratch_file_survives_a_normal_run_or_a_timed_out_one() {
        let _guard = test_lock();
        let prefix = format!("ply-engine-{}-", std::process::id());
        let leftover_count = || -> usize {
            std::fs::read_dir(std::env::temp_dir())
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                        .count()
                })
                .unwrap_or(0)
        };

        let before = leftover_count();

        let mut fast = Command::new("/bin/echo");
        fast.arg("hi");
        run_with_timeout(&mut fast, Duration::from_secs(5)).unwrap();
        assert_eq!(
            leftover_count(),
            before,
            "a fast run must not leave its capture files behind"
        );

        let mut slow = Command::new("/bin/sleep");
        slow.arg("30");
        run_with_timeout(&mut slow, Duration::from_millis(150)).unwrap();
        assert_eq!(
            leftover_count(),
            before,
            "a timed-out run must not leave its capture files behind either"
        );
    }

    /// The actual defect: every engine used to shell out to a `timeout`/
    /// `gtimeout` binary, which macOS ships neither of. This process's own
    /// PATH is emptied for the duration of the call, so any implementation
    /// that still tries to spawn a *named* helper program (found only via
    /// PATH lookup) fails here exactly as it failed on macOS -- while a
    /// budget enforced in-process neither needs nor looks for one, so it
    /// keeps working with an absolute path to the real program.
    #[test]
    fn enforces_the_budget_with_no_timeout_binary_reachable_on_path() {
        let _guard = test_lock();
        let old_path = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "");
        }
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        let start = Instant::now();
        let result = run_with_timeout(&mut cmd, Duration::from_millis(150));
        match old_path {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        let out = result.unwrap();
        assert!(
            out.timed_out,
            "the budget must still be enforced with no `timeout` binary on PATH"
        );
        assert!(start.elapsed() < Duration::from_secs(10));
    }
}
