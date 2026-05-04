//! Progress bars and spinners for CLI output.
//!
//! Uses raw ANSI escape sequences (no external dependency). Supports:
//! - Percentage progress bar with visual block characters
//! - Spinner with label
//! - OSC 9;4 terminal progress protocol (ConEmu/Windows Terminal/iTerm2)
//! - Delay suppression for fast operations
//! - Trait-based facade (`ProgressReporter`) so call sites can stay agnostic
//!   of TTY vs. non-TTY environments
//!
//! # Choosing a reporter
//!
//! Most callers should use [`auto`], which picks a sensible default based on
//! whether stderr is a TTY:
//!
//! ```no_run
//! use librefang_cli::progress::{auto, ProgressReporter};
//!
//! let mut p = auto("Indexing", Some(100));
//! for i in 0..100 {
//!     p.tick(1);
//!     # let _ = i;
//! }
//! p.finish("Indexed 100 items");
//! ```
//!
//! On a TTY this renders an animated unicode bar; over a pipe or dumb
//! terminal it falls back to plain `[n/total] msg` lines on stderr so logs
//! stay grep-friendly.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// Default progress bar width (in characters).
const DEFAULT_BAR_WIDTH: usize = 30;

/// Minimum elapsed time before showing progress output. Operations that
/// complete faster than this threshold produce no visual noise.
const DELAY_SUPPRESS_MS: u64 = 200;

/// Block characters for the progress bar.
const FILLED: char = '\u{2588}'; // █
const EMPTY: char = '\u{2591}'; // ░

/// Spinner animation frames.
const SPINNER_FRAMES: &[char] = &[
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280f}',
];

// ---------------------------------------------------------------------------
// OSC 9;4 progress protocol
// ---------------------------------------------------------------------------

/// Emit an OSC 9;4 progress sequence (supported by Windows Terminal, ConEmu,
/// iTerm2). `state`: 1 = set progress, 2 = error, 3 = indeterminate, 0 = clear.
fn osc_progress(state: u8, percent: u8) {
    // ESC ] 9 ; 4 ; state ; percent ST
    // ST = ESC \   (string terminator)
    let _ = write!(io::stderr(), "\x1b]9;4;{state};{percent}\x1b\\");
    let _ = io::stderr().flush();
}

/// Clear the OSC 9;4 progress indicator.
fn osc_progress_clear() {
    osc_progress(0, 0);
}

// ---------------------------------------------------------------------------
// ProgressBar
// ---------------------------------------------------------------------------

/// A simple percentage-based progress bar.
///
/// ```text
/// Downloading   [████████████░░░░░░░░░░░░░░░░░░]  40% (4/10)
/// ```
pub struct ProgressBar {
    label: String,
    total: u64,
    current: u64,
    width: usize,
    start: Instant,
    suppress_until: Duration,
    visible: bool,
    use_osc: bool,
}

impl ProgressBar {
    /// Create a new progress bar.
    ///
    /// `label`: text shown before the bar.
    /// `total`: the 100% value.
    pub fn new(label: &str, total: u64) -> Self {
        Self {
            label: label.to_string(),
            total: total.max(1),
            current: 0,
            width: DEFAULT_BAR_WIDTH,
            start: Instant::now(),
            suppress_until: Duration::from_millis(DELAY_SUPPRESS_MS),
            visible: false,
            use_osc: true,
        }
    }

    /// Set the bar width in characters.
    pub fn width(mut self, w: usize) -> Self {
        self.width = w.max(5);
        self
    }

    /// Disable delay suppression (always show immediately).
    pub fn no_delay(mut self) -> Self {
        self.suppress_until = Duration::ZERO;
        self
    }

    /// Disable OSC 9;4 terminal progress protocol.
    pub fn no_osc(mut self) -> Self {
        self.use_osc = false;
        self
    }

    /// Update progress to `n`.
    pub fn set(&mut self, n: u64) {
        self.current = n.min(self.total);
        self.draw();
    }

    /// Increment progress by `delta`.
    pub fn inc(&mut self, delta: u64) {
        self.current = (self.current + delta).min(self.total);
        self.draw();
    }

    /// Mark as finished and clear the line.
    pub fn finish(&mut self) {
        self.current = self.total;
        self.draw();
        if self.visible {
            // Move to next line
            eprintln!();
        }
        if self.use_osc {
            osc_progress_clear();
        }
    }

    /// Mark as finished with a message replacing the bar.
    pub fn finish_with_message(&mut self, msg: &str) {
        self.current = self.total;
        if self.visible {
            eprint!("\r\x1b[2K{msg}");
            eprintln!();
        } else if self.start.elapsed() >= self.suppress_until {
            eprintln!("{msg}");
        }
        if self.use_osc {
            osc_progress_clear();
        }
    }

    fn draw(&mut self) {
        // Delay suppression: don't render if op is still fast
        if self.start.elapsed() < self.suppress_until && self.current < self.total {
            return;
        }

        self.visible = true;

        let pct = (self.current as f64 / self.total as f64 * 100.0) as u8;
        let filled = (self.current as f64 / self.total as f64 * self.width as f64) as usize;
        let empty = self.width.saturating_sub(filled);

        let bar: String = std::iter::repeat_n(FILLED, filled)
            .chain(std::iter::repeat_n(EMPTY, empty))
            .collect();

        eprint!(
            "\r\x1b[2K{:<14} [{}] {:>3}% ({}/{})",
            self.label, bar, pct, self.current, self.total
        );
        let _ = io::stderr().flush();

        if self.use_osc {
            osc_progress(1, pct);
        }
    }
}

impl Drop for ProgressBar {
    fn drop(&mut self) {
        if self.use_osc && self.visible {
            osc_progress_clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Spinner
// ---------------------------------------------------------------------------

/// An indeterminate spinner for operations without known total.
///
/// ```text
/// ⠋ Loading models...
/// ```
pub struct Spinner {
    label: String,
    frame: usize,
    start: Instant,
    suppress_until: Duration,
    visible: bool,
    use_osc: bool,
}

impl Spinner {
    /// Create a spinner with the given label.
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            frame: 0,
            start: Instant::now(),
            suppress_until: Duration::from_millis(DELAY_SUPPRESS_MS),
            visible: false,
            use_osc: true,
        }
    }

    /// Disable delay suppression.
    pub fn no_delay(mut self) -> Self {
        self.suppress_until = Duration::ZERO;
        self
    }

    /// Disable OSC 9;4 protocol.
    pub fn no_osc(mut self) -> Self {
        self.use_osc = false;
        self
    }

    /// Advance the spinner by one frame and redraw.
    pub fn tick(&mut self) {
        if self.start.elapsed() < self.suppress_until {
            return;
        }

        self.visible = true;
        let ch = SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()];
        self.frame += 1;

        eprint!("\r\x1b[2K{ch} {}", self.label);
        let _ = io::stderr().flush();

        if self.use_osc {
            osc_progress(3, 0);
        }
    }

    /// Update the label text.
    pub fn set_label(&mut self, label: &str) {
        self.label = label.to_string();
    }

    /// Stop the spinner and clear the line.
    pub fn finish(&self) {
        if self.visible {
            eprint!("\r\x1b[2K");
            let _ = io::stderr().flush();
        }
        if self.use_osc {
            osc_progress_clear();
        }
    }

    /// Stop the spinner and print a final message.
    pub fn finish_with_message(&self, msg: &str) {
        if self.visible {
            eprint!("\r\x1b[2K");
        }
        eprintln!("{msg}");
        if self.use_osc {
            osc_progress_clear();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if self.use_osc && self.visible {
            osc_progress_clear();
        }
    }
}

// ---------------------------------------------------------------------------
// ProgressReporter trait + dynamic dispatch facade
// ---------------------------------------------------------------------------

/// Unified facade for CLI progress output.
///
/// Implementations are expected to be cheap to construct and to honour delay
/// suppression / TTY detection internally — call sites should never need to
/// branch on environment.
pub trait ProgressReporter {
    /// Advance progress by `delta`. For indeterminate reporters the delta is
    /// treated as a single step.
    fn tick(&mut self, delta: u64);
    /// Update the label / message displayed alongside progress.
    fn set_message(&mut self, msg: &str);
    /// Mark progress as complete and emit a final message.
    fn finish(&mut self, msg: &str);
}

impl ProgressReporter for ProgressBar {
    fn tick(&mut self, delta: u64) {
        self.inc(delta);
    }
    fn set_message(&mut self, msg: &str) {
        self.label = msg.to_string();
    }
    fn finish(&mut self, msg: &str) {
        self.finish_with_message(msg);
    }
}

impl ProgressReporter for Spinner {
    fn tick(&mut self, _delta: u64) {
        Spinner::tick(self);
    }
    fn set_message(&mut self, msg: &str) {
        self.set_label(msg);
    }
    fn finish(&mut self, msg: &str) {
        Spinner::finish_with_message(self, msg);
    }
}

/// Plain-text fallback reporter for non-TTY environments (CI logs, pipes,
/// dumb terminals).
///
/// Emits one line per `tick` to stderr in the form `[current/total] label`,
/// or `[current] label` when the total is unknown. Output is line-buffered
/// so it interleaves cleanly with surrounding tracing logs.
pub struct LogReporter {
    label: String,
    total: Option<u64>,
    current: u64,
}

impl LogReporter {
    /// Create a log-style reporter. `total = None` means indeterminate.
    pub fn new(label: &str, total: Option<u64>) -> Self {
        Self {
            label: label.to_string(),
            total,
            current: 0,
        }
    }
}

impl ProgressReporter for LogReporter {
    fn tick(&mut self, delta: u64) {
        self.current = self.current.saturating_add(delta);
        match self.total {
            Some(t) => eprintln!("[{}/{}] {}", self.current.min(t), t, self.label),
            None => eprintln!("[{}] {}", self.current, self.label),
        }
    }
    fn set_message(&mut self, msg: &str) {
        self.label = msg.to_string();
    }
    fn finish(&mut self, msg: &str) {
        eprintln!("{msg}");
    }
}

/// Pick a reporter based on whether stderr is a TTY.
///
/// On a TTY: animated [`ProgressBar`] when `total` is known, [`Spinner`]
/// otherwise. Off a TTY (pipe, redirect, CI): [`LogReporter`].
///
/// The returned trait object has dynamic dispatch — fine for CLI call sites
/// where we issue a handful of ticks per second, not millions.
pub fn auto(label: &str, total: Option<u64>) -> Box<dyn ProgressReporter> {
    if io::stderr().is_terminal() {
        match total {
            Some(t) => Box::new(ProgressBar::new(label, t)),
            None => Box::new(Spinner::new(label)),
        }
    } else {
        Box::new(LogReporter::new(label, total))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_bar_percentage() {
        let mut pb = ProgressBar::new("Test", 10).no_delay().no_osc();
        pb.set(5);
        assert_eq!(pb.current, 5);
        pb.inc(3);
        assert_eq!(pb.current, 8);
        // Cannot exceed total
        pb.inc(100);
        assert_eq!(pb.current, 10);
    }

    #[test]
    fn progress_bar_zero_total_no_panic() {
        // total of 0 should be clamped to 1 to avoid division by zero
        let mut pb = ProgressBar::new("Empty", 0).no_delay().no_osc();
        pb.set(0);
        pb.finish();
        assert_eq!(pb.total, 1);
    }

    #[test]
    fn spinner_frame_advance() {
        let mut sp = Spinner::new("Loading").no_delay().no_osc();
        sp.tick();
        assert_eq!(sp.frame, 1);
        sp.tick();
        assert_eq!(sp.frame, 2);
        sp.finish();
    }

    #[test]
    fn delay_suppression() {
        // With default suppress_until, a freshly-created bar should NOT
        // become visible on the first draw (elapsed < 200ms).
        let mut pb = ProgressBar::new("Quick", 10).no_osc();
        pb.set(1);
        assert!(!pb.visible);
    }

    #[test]
    fn log_reporter_tick_finish_round_trip() {
        // LogReporter is the canonical fallback path; verify the trait
        // contract (tick advances, finish doesn't panic, set_message
        // mutates the label) without inspecting stderr output.
        let mut r = LogReporter::new("Sync", Some(3));
        ProgressReporter::tick(&mut r, 1);
        ProgressReporter::tick(&mut r, 1);
        assert_eq!(r.current, 2);
        ProgressReporter::set_message(&mut r, "Sync (final)");
        assert_eq!(r.label, "Sync (final)");
        ProgressReporter::tick(&mut r, 1);
        assert_eq!(r.current, 3);
        ProgressReporter::finish(&mut r, "done");
    }

    #[test]
    fn log_reporter_indeterminate_does_not_overflow() {
        // total = None branch shouldn't gate on any total comparison.
        let mut r = LogReporter::new("Loading", None);
        ProgressReporter::tick(&mut r, u64::MAX / 2);
        ProgressReporter::tick(&mut r, u64::MAX / 2);
        // saturating_add prevents wraparound.
        assert_eq!(r.current, u64::MAX - 1);
        ProgressReporter::tick(&mut r, 100);
        assert_eq!(r.current, u64::MAX);
    }

    #[test]
    fn progress_bar_implements_reporter() {
        // Compile-time check that ProgressBar / Spinner satisfy the trait
        // (so `auto()` can box them) and that dispatching through &mut dyn
        // forwards to inc / set_label correctly.
        let mut pb = ProgressBar::new("T", 5).no_delay().no_osc();
        let r: &mut dyn ProgressReporter = &mut pb;
        r.tick(2);
        r.set_message("renamed");
        // Reach into the concrete type to verify side effects.
        assert_eq!(pb.current, 2);
        assert_eq!(pb.label, "renamed");
    }
}
