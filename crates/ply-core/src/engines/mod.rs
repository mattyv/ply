pub mod fuzz;
pub mod kani;
pub mod mutants;

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// How often [`run_with_timeout`] polls the child for exit -- a compromise
/// between wasted CPU (too tight) and slack in when a killed run is
/// noticed (too loose). Chosen small enough that no caller's wall-clock
/// budget is meaningfully overshot by it.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

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
/// polls [`Child::try_wait`] in a sleep loop. On expiry the child is killed
/// and reaped, `timed_out` is set, and the two files are read back into
/// memory.
pub fn run_with_timeout(cmd: &mut Command, budget: Duration) -> Result<TimedOutput> {
    let program = cmd.get_program().to_string_lossy().into_owned();

    let stdout_path = scratch_path("stdout");
    let stderr_path = scratch_path("stderr");
    let _stdout_guard = ScratchFile(stdout_path.clone());
    let _stderr_guard = ScratchFile(stderr_path.clone());

    let stdout_file = std::fs::File::create(&stdout_path)
        .context("creating a scratch file to capture the child process's stdout")?;
    let stderr_file = std::fs::File::create(&stderr_path)
        .context("creating a scratch file to capture the child process's stderr")?;

    let mut child = cmd
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .with_context(|| {
            format!("could not start `{program}` -- check that it is installed and on your PATH")
        })?;

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            // Kill and reap before propagating: returning through `?` here
            // would leave the child running with nothing ever waiting on it
            // -- an orphan that outlives the budget it was spawned under.
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error)
                    .with_context(|| format!("checking whether `{program}` has finished"));
            }
        }
        if start.elapsed() >= budget {
            timed_out = true;
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
    use super::run_with_timeout;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// A command that finishes well inside its budget must be run for
    /// real, not skipped or faked -- its actual stdout and exit status
    /// come back untouched.
    #[test]
    fn a_command_that_finishes_in_time_returns_its_real_stdout_and_status() {
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

    /// The actual defect: every engine used to shell out to a `timeout`/
    /// `gtimeout` binary, which macOS ships neither of. This process's own
    /// PATH is emptied for the duration of the call, so any implementation
    /// that still tries to spawn a *named* helper program (found only via
    /// PATH lookup) fails here exactly as it failed on macOS -- while a
    /// budget enforced in-process neither needs nor looks for one, so it
    /// keeps working with an absolute path to the real program.
    #[test]
    fn enforces_the_budget_with_no_timeout_binary_reachable_on_path() {
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
