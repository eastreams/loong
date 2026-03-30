#![cfg(unix)]

//! PTY-based integration tests for the TUI chat mode.
//!
//! These tests spawn `loong chat --ui tui` in a real pseudo-terminal so the
//! binary enters full-screen mode instead of falling back to text mode.

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;

static PTY_TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_pty_temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = PTY_TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loongclaw-tui-pty-{label}-{}-{nanos}-{counter}",
        std::process::id(),
    ))
}

/// Strip ANSI/CSI escape sequences from raw terminal output, returning plain
/// text suitable for substring matching.
fn strip_ansi(raw: &[u8]) -> String {
    let stripped = strip_ansi_escapes::strip(raw);
    String::from_utf8_lossy(&stripped).into_owned()
}

// ---------------------------------------------------------------------------
// TuiPtyFixture
// ---------------------------------------------------------------------------

/// A background reader thread pumps bytes from the blocking PTY reader into an
/// `mpsc` channel, allowing the fixture methods to poll with timeouts instead
/// of blocking forever.
struct TuiPtyFixture {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Box<dyn Write + Send>,
    /// Receives byte chunks from the background reader thread.
    rx: std_mpsc::Receiver<Vec<u8>>,
    /// Accumulated raw bytes read so far (including previous calls).
    accumulated: Vec<u8>,
    _root: PathBuf,
}

impl TuiPtyFixture {
    /// Spawn `loong chat --ui tui` inside a real PTY.
    ///
    /// `label` is used to create a unique temp directory for the fixture.
    /// A minimal default config is written so the binary can start without
    /// triggering the onboarding flow.
    fn spawn(label: &str) -> Self {
        let root = unique_pty_temp_path(label);
        let home_dir = root.join("home");
        std::fs::create_dir_all(&home_dir).expect("create fixture home directory");

        // Write a minimal config so chat does not enter onboarding.
        let config_path = root.join("loongclaw.toml");
        let config = loongclaw_app::config::LoongClawConfig::default();
        let config_path_str = config_path.to_string_lossy().into_owned();
        loongclaw_app::config::write(Some(&config_path_str), &config, true)
            .expect("write default config for PTY fixture");

        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open PTY pair");

        let binary_path = env!("CARGO_BIN_EXE_loongclaw");
        let mut cmd = CommandBuilder::new(binary_path);
        cmd.arg("chat");
        cmd.arg("--ui");
        cmd.arg("tui");
        cmd.arg("--config");
        cmd.arg(&config_path);
        cmd.env("HOME", &home_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORFGBG", "15;0");
        cmd.env_remove("LOONGCLAW_CONFIG_PATH");
        cmd.env_remove("USERPROFILE");

        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("spawn loong chat --ui tui in PTY");

        // The slave side must be dropped after spawning so EOF propagates
        // correctly when the child exits.
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("clone PTY master reader");
        let writer = pair.master.take_writer().expect("take PTY master writer");

        // Spawn a background thread that reads from the blocking PTY reader
        // and sends byte chunks over an mpsc channel.
        let (tx, rx) = std_mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name(format!("pty-reader-{label}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        #[allow(clippy::indexing_slicing)]
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("spawn PTY reader thread");

        Self {
            child,
            writer,
            rx,
            accumulated: Vec::new(),
            _root: root,
        }
    }

    /// Send raw bytes (keystrokes) to the PTY.
    fn send_keys(&mut self, keys: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(keys)
            .map_err(|e| format!("failed to send keys to PTY: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| format!("failed to flush PTY writer: {e}"))
    }

    /// Type a string of text into the PTY as individual characters.
    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.send_keys(text.as_bytes())
    }

    /// Send the Enter key (carriage return).
    #[allow(dead_code)]
    fn send_enter(&mut self) -> Result<(), String> {
        self.send_keys(b"\r")
    }

    /// Send the Escape key.
    fn send_escape(&mut self) -> Result<(), String> {
        self.send_keys(b"\x1b")
    }

    /// Send Ctrl+C (ETX).
    fn send_ctrl_c(&mut self) -> Result<(), String> {
        self.send_keys(b"\x03")
    }

    /// Drain any pending data from the reader channel into `accumulated`.
    fn drain_pending(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.accumulated.extend_from_slice(&chunk);
        }
    }

    /// Read all available output from the PTY, stripping ANSI escape
    /// sequences and returning plain text.  Waits up to `timeout` for
    /// data to arrive, then returns everything accumulated so far.
    fn read_screen(&mut self, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;

        loop {
            self.drain_pending();

            if !self.accumulated.is_empty() {
                // Give a short grace period for more data to arrive.
                std::thread::sleep(Duration::from_millis(100));
                self.drain_pending();
                break;
            }

            if Instant::now() >= deadline {
                break;
            }

            // Wait for the first chunk with a bounded recv_timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Duration::from_millis(100));
            if let Ok(chunk) = self.rx.recv_timeout(wait) {
                self.accumulated.extend_from_slice(&chunk);
            }
        }

        Ok(strip_ansi(&self.accumulated))
    }

    /// Wait until the screen output contains `pattern`, returning the full
    /// accumulated text.  Polls every 100ms until `timeout`.
    fn wait_for(&mut self, pattern: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;

        loop {
            self.drain_pending();

            let plain = strip_ansi(&self.accumulated);
            if plain.contains(pattern) {
                return Ok(plain);
            }

            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for pattern {:?} in PTY output (got: {:?})",
                    pattern, plain
                ));
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Duration::from_millis(100));
            match self.rx.recv_timeout(wait) {
                Ok(chunk) => {
                    self.accumulated.extend_from_slice(&chunk);
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                    let plain = strip_ansi(&self.accumulated);
                    return Err(format!(
                        "PTY reader disconnected before pattern {:?} appeared (got: {:?})",
                        pattern, plain,
                    ));
                }
            }
        }
    }

    /// Wait for the child process to exit within `timeout`.
    /// Returns the exit status code, or an error on timeout.
    fn wait_for_exit(&mut self, timeout: Duration) -> Result<u32, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(status.exit_code());
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return Err("timed out waiting for child process to exit".to_owned());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(format!("error checking child status: {e}"));
                }
            }
        }
    }
}

impl Drop for TuiPtyFixture {
    fn drop(&mut self) {
        // Best-effort kill of the child process.
        let _ = self.child.kill();
        // Wait for exit to reap the zombie.
        let _ = self.child.wait();
        // Clean up the temp directory.
        let _ = std::fs::remove_dir_all(&self._root);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The TUI enters full-screen mode (alternate screen) when spawned in a real
/// PTY, and does not crash or fall back to text mode.
#[test]
fn tui_enters_fullscreen_in_pty() {
    let mut fixture = TuiPtyFixture::spawn("enters-fullscreen");

    // The TUI welcome message proves we entered the full-screen surface
    // rather than falling back to text mode.
    let screen = fixture
        .wait_for("Welcome to LoongClaw TUI", Duration::from_secs(10))
        .expect("TUI should render the welcome message in PTY");

    // Verify the full welcome text is present.
    assert!(
        screen.contains("Welcome to LoongClaw TUI"),
        "TUI should show the welcome message: {screen:?}"
    );

    // Exit cleanly via Escape.
    fixture.send_escape().expect("send Escape key to exit TUI");

    let exit_code = fixture
        .wait_for_exit(Duration::from_secs(5))
        .expect("TUI should exit after Escape");
    assert_eq!(exit_code, 0, "TUI should exit with code 0 after Escape");
}

/// The TUI displays its welcome/ready message when started.
#[test]
fn tui_shows_welcome_message() {
    let mut fixture = TuiPtyFixture::spawn("shows-welcome");

    let screen = fixture
        .wait_for("Welcome to LoongClaw TUI", Duration::from_secs(10))
        .expect("TUI should render the welcome message");

    assert!(
        screen.contains("Type a message and press Enter"),
        "Welcome message should include input instructions: {screen:?}"
    );

    fixture.send_escape().expect("send Escape to exit");
}

/// Typing text into the TUI composer area makes it visible on the screen.
#[test]
fn tui_composer_accepts_input() {
    let mut fixture = TuiPtyFixture::spawn("composer-input");

    fixture
        .wait_for("Welcome to LoongClaw TUI", Duration::from_secs(10))
        .expect("TUI should be ready before typing");

    // Small delay to let the event loop settle after rendering.
    std::thread::sleep(Duration::from_millis(200));

    fixture
        .type_text("hello world")
        .expect("type text into TUI composer");

    // Give the TUI time to process keystrokes and re-render.
    std::thread::sleep(Duration::from_millis(500));

    let screen = fixture
        .read_screen(Duration::from_secs(3))
        .expect("read screen after typing");

    assert!(
        screen.contains("hello world"),
        "typed text should appear on the screen: {screen:?}"
    );

    fixture.send_escape().expect("send Escape to exit");
}

/// Pressing Escape exits the TUI cleanly with exit code 0.
#[test]
fn tui_exit_via_escape() {
    let mut fixture = TuiPtyFixture::spawn("exit-escape");

    fixture
        .wait_for("Welcome to LoongClaw TUI", Duration::from_secs(10))
        .expect("TUI should be ready before exit");

    fixture.send_escape().expect("send Escape to exit");

    let exit_code = fixture
        .wait_for_exit(Duration::from_secs(5))
        .expect("process should exit after Escape");

    assert_eq!(
        exit_code, 0,
        "TUI should exit cleanly with code 0 via Escape"
    );
}

/// Pressing Ctrl+C exits the TUI cleanly with exit code 0.
#[test]
fn tui_exit_via_ctrl_c() {
    let mut fixture = TuiPtyFixture::spawn("exit-ctrl-c");

    fixture
        .wait_for("Welcome to LoongClaw TUI", Duration::from_secs(10))
        .expect("TUI should be ready before Ctrl+C");

    fixture.send_ctrl_c().expect("send Ctrl+C to exit");

    let exit_code = fixture
        .wait_for_exit(Duration::from_secs(5))
        .expect("process should exit after Ctrl+C");

    assert_eq!(
        exit_code, 0,
        "TUI should exit cleanly with code 0 via Ctrl+C"
    );
}
