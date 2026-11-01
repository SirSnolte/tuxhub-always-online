//! The background service and its live interface, run as separate processes
//! with their own XDG directories, so nothing touches the real session.
//!
//! Tests that switch the service on watch for its virtual device and grab it
//! (EVIOCGRAB) as soon as it appears, so the impulses do not reach the
//! desktop. The service waits 500 ms after creating the device before the
//! first impulse; a missed grab would send a harmless F15 or a mouse step
//! of one unit and back.

use evdev::{Device, EventType};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::mem::ManuallyDrop;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_tuxhub-always-online");
const DEVICE_NAME: &str = "tuxhub-always-online";

/// Isolated home, XDG directories and a short runtime directory (socket
/// paths are limited to 108 bytes).
struct Env {
    root: PathBuf,
    runtime: PathBuf,
}

impl Env {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let tag = format!("{}-{n}", std::process::id());
        let root = std::env::temp_dir().join(format!("tuxhub-tao-{name}-{tag}"));
        let user_run = PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }));
        let runtime = if user_run.is_dir() {
            user_run.join(format!("tao-{tag}"))
        } else {
            root.join("run")
        };
        for dir in [&root, &runtime] {
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(dir).unwrap();
        }
        Self { root, runtime }
    }

    fn socket(&self) -> PathBuf {
        self.runtime.join("tuxhub/always-online.sock")
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(EXE);
        cmd.env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    fn spawn_daemon(&self) -> Guard {
        Guard(self.command().arg("daemon").spawn().unwrap())
    }

    /// Starts the service and waits for its socket.
    fn daemon(&self) -> Guard {
        let guard = self.spawn_daemon();
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(self.socket()).is_err() {
            assert!(Instant::now() < deadline, "the service did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        guard
    }

    fn connect(&self) -> Conn {
        let stream = UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Conn {
            reader: BufReader::new(stream.try_clone().unwrap()),
            stream,
            next: 0,
            notes: Vec::new(),
        }
    }

    fn config_file(&self) -> PathBuf {
        self.root.join("config/tuxhub-always-online/config.json")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.runtime);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Kills a process that a failed test left running.
struct Guard(Child);

impl Guard {
    fn exits_within(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.0.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A connection to the service that keeps the notifications it skips.
struct Conn {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
    notes: Vec<Value>,
}

impl Conn {
    fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap()),
        }
    }

    /// The answer as `Ok(result)` or `Err(error object)`.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let id = self.next;
        writeln!(
            self.stream,
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )
        .unwrap();
        loop {
            let msg = self.read().expect("connection closed");
            if msg.get("id") == Some(&json!(id)) {
                return match msg.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(msg["result"].clone()),
                };
            }
            self.notes.push(msg);
        }
    }

    fn hello(&mut self) -> Value {
        self.call(
            "hello",
            json!({ "protocol": 1, "client": "test", "locale": "en" }),
        )
        .unwrap()
    }

    /// Notifications named `method` received within `wait`.
    fn notes(&mut self, method: &str, wait: Duration) -> Vec<Value> {
        self.stream.set_read_timeout(Some(wait)).unwrap();
        while let Some(msg) = self.read() {
            self.notes.push(msg);
        }
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        self.notes
            .iter()
            .filter(|n| n["method"] == method)
            .map(|n| n["params"].clone())
            .collect()
    }
}

// ----------------------------------------------------------------- device

/// Paths of the event nodes of every virtual device of the tool, from the
/// kernel's list of input devices.
fn our_nodes() -> Vec<PathBuf> {
    let text = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    text.split("\n\n")
        .filter(|block| block.contains(&format!("Name=\"{DEVICE_NAME}\"")))
        .filter_map(|block| {
            block
                .lines()
                .find_map(|l| l.strip_prefix("H: Handlers="))?
                .split_whitespace()
                .find(|h| h.starts_with("event"))
                .map(|h| PathBuf::from("/dev/input").join(h))
        })
        .collect()
}

/// Grabs the virtual device of the service the moment it appears and reports
/// its events as `(type, code, value)` without the sync events.
struct Watcher {
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<(EventType, u16, i32)>,
    thread: Option<JoinHandle<()>>,
}

fn watch() -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, events) = mpsc::channel();
    let thread = std::thread::spawn({
        let stop = stop.clone();
        move || {
            let mut open: Vec<(PathBuf, ManuallyDrop<Device>)> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let nodes = our_nodes();
                for node in &nodes {
                    if open.iter().any(|(p, _)| p == node) {
                        continue;
                    }
                    // The node may not be readable for a moment.
                    if let Ok(mut dev) = Device::open(node)
                        && dev.name() == Some(DEVICE_NAME)
                        && dev.grab().is_ok()
                        && dev.set_nonblocking(true).is_ok()
                    {
                        open.push((node.clone(), ManuallyDrop::new(dev)));
                    }
                }
                open.retain_mut(|(_, dev)| {
                    let fd = dev.as_raw_fd();
                    match dev.fetch_events() {
                        Ok(batch) => {
                            for ev in batch {
                                if ev.event_type() != EventType::SYNCHRONIZATION {
                                    let _ = tx.send((ev.event_type(), ev.code(), ev.value()));
                                }
                            }
                            true
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                        Err(_) => {
                            // The device is gone; a normal drop would complain
                            // that it cannot release the grab.
                            unsafe { libc::close(fd) };
                            false
                        }
                    }
                });
                std::thread::sleep(Duration::from_millis(3));
            }
            for (_, mut dev) in open {
                let _ = dev.ungrab();
                unsafe { libc::close(dev.as_raw_fd()) };
            }
        }
    });
    Watcher {
        stop,
        events,
        thread: Some(thread),
    }
}

impl Watcher {
    fn next(&self, timeout: Duration) -> Option<(EventType, u16, i32)> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether the tests with a real virtual device can run here: they need to
/// write to /dev/uinput and to read the new device in /dev/input. CI machines
/// have neither, as in the other tuxhub tools.
fn can_test_the_device() -> bool {
    let uinput = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let input = std::fs::read_dir("/dev/input").is_ok();
    if !(uinput && input) {
        eprintln!("skipped: no access to /dev/uinput or /dev/input");
    }
    uinput && input
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

// ------------------------------------------------------------------ tests

#[test]
fn answers_the_live_protocol() {
    let env = Env::new("protocol");
    let _service = env.daemon();
    let mut c = env.connect();
    let hello = c.hello();
    assert_eq!(hello["protocol"], json!(1));
    assert_eq!(hello["name"], json!("Always Online"));
    assert_eq!(hello["version"], json!(env!("CARGO_PKG_VERSION")));
    assert!(c.call("hello", json!({ "protocol": 2 })).is_err());

    let de = c.call("schema", json!({ "locale": "de" })).unwrap();
    assert_eq!(de["sections"][0]["label"], json!("Allgemein"));
    let fr = c.call("schema", json!({ "locale": "fr" })).unwrap();
    assert_eq!(fr["sections"][0]["label"], json!("General"));

    // A new installation keeps quiet until the switch is turned on.
    assert_eq!(
        c.call("get", json!({})).unwrap(),
        json!({ "values": { "enabled": false, "interval_s": 240, "impulse": "key", "max_hours": 0, "start_at_login": true } })
    );
    assert_eq!(
        c.call("config.get", json!({})).unwrap()["schedule"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert!(c.call("configure", json!({})).is_err());
}

#[test]
fn a_new_installation_sends_nothing_and_creates_no_device() {
    let env = Env::new("quiet");
    let _service = env.daemon();
    let mut c = env.connect();
    c.hello();
    let status = c.call("status", json!({})).unwrap();
    assert_eq!(status["state"], json!("off"));
    assert_eq!(status["sent"], json!(0));
    std::thread::sleep(Duration::from_millis(700));
    assert!(our_nodes().is_empty(), "{:?}", our_nodes());
}

#[test]
fn socket_and_folder_are_private() {
    let env = Env::new("modes");
    let _service = env.daemon();
    let dir = std::fs::metadata(env.socket().parent().unwrap()).unwrap();
    let sock = std::fs::metadata(env.socket()).unwrap();
    assert_eq!(dir.permissions().mode() & 0o777, 0o700);
    assert_eq!(sock.permissions().mode() & 0o777, 0o600);
}

/// A second service ends at once and leaves the socket alone.
#[test]
fn a_second_service_ends_and_keeps_the_socket() {
    let env = Env::new("second");
    let _service = env.daemon();
    let mut second = env.spawn_daemon();
    assert!(second.exits_within(Duration::from_secs(5)).is_some());
    let mut c = env.connect();
    assert_eq!(c.hello()["protocol"], json!(1));
}

/// A socket file that nobody listens on is replaced.
#[test]
fn a_dead_socket_is_replaced() {
    let env = Env::new("dead");
    std::fs::create_dir_all(env.socket().parent().unwrap()).unwrap();
    drop(UnixListener::bind(env.socket()).unwrap());
    assert!(env.socket().exists());
    let _service = env.daemon();
    assert_eq!(env.connect().hello()["protocol"], json!(1));
}

/// A regular file or a symbolic link in place of the socket is never
/// removed; the service refuses to start.
#[test]
fn foreign_files_at_the_socket_path_are_kept() {
    for kind in ["file", "symlink"] {
        let env = Env::new(kind);
        std::fs::create_dir_all(env.socket().parent().unwrap()).unwrap();
        let target = env.root.join("target");
        std::fs::write(&target, "keep").unwrap();
        if kind == "file" {
            std::fs::write(env.socket(), "keep").unwrap();
        } else {
            std::os::unix::fs::symlink(&target, env.socket()).unwrap();
        }
        let mut service = env.spawn_daemon();
        let status = service
            .exits_within(Duration::from_secs(5))
            .expect("still running");
        assert!(!status.success(), "{kind}");
        let meta = std::fs::symlink_metadata(env.socket()).unwrap();
        assert_eq!(meta.file_type().is_symlink(), kind == "symlink");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }
}

/// Refused values name the field and change nothing.
#[test]
fn set_refuses_invalid_values() {
    let env = Env::new("refuse");
    let _service = env.daemon();
    let mut c = env.connect();
    c.hello();
    let before = c.call("config.get", json!({})).unwrap();
    for (key, value) in [
        ("schedule", json!([])),
        ("enabled", json!("yes")),
        ("interval_s", json!(5)),
        ("impulse", json!("dbus")),
        ("max_hours", json!(99)),
    ] {
        let e = c
            .call("set", json!({ "key": key, "value": value }))
            .unwrap_err();
        assert_eq!(e["field"], json!(key), "{e}");
        assert!(e["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    // The schedule is refused by `config.set` as well.
    assert!(c.call("config.set", json!({ "schedule": [] })).is_err());
    assert_eq!(c.call("config.get", json!({})).unwrap(), before);
    assert!(!env.config_file().exists());
}

/// The connection that sets a value gets no `changed`, the others do; the
/// value is saved.
#[test]
fn set_notifies_the_other_connections() {
    let env = Env::new("set");
    let _service = env.daemon();
    let (mut a, mut b) = (env.connect(), env.connect());
    a.hello();
    b.hello();
    let applied = a
        .call("set", json!({ "key": "interval_s", "value": 90 }))
        .unwrap();
    assert_eq!(applied, json!({ "value": 90 }));
    let changed = b.notes("changed", Duration::from_millis(500));
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["values"]["interval_s"], json!(90));
    assert_eq!(b.notes("config", Duration::from_millis(10)).len(), 1);
    assert!(a.notes("changed", Duration::from_millis(300)).is_empty());
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(env.config_file()).unwrap()).unwrap();
    assert_eq!(saved["interval_s"], json!(90));
}

/// A change from the window reaches every live connection; one outside the
/// schema sends no `changed`.
#[test]
fn config_set_notifies_every_connection() {
    let env = Env::new("window");
    let _service = env.daemon();
    let (mut window, mut hub) = (env.connect(), env.connect());
    hub.hello();
    window
        .call("config.set", json!({ "max_hours": 4 }))
        .unwrap();
    let changed = hub.notes("changed", Duration::from_millis(500));
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["values"]["max_hours"], json!(4));
    assert_eq!(window.notes("changed", Duration::from_millis(300)).len(), 1);
    let count = hub.notes("changed", Duration::from_millis(10)).len();
    window
        .call("config.set", json!({ "schedule_enabled": true }))
        .unwrap();
    assert_eq!(hub.notes("config", Duration::from_millis(300)).len(), 2);
    assert_eq!(hub.notes("changed", Duration::from_millis(10)).len(), count);
}

/// The settings survive a restart of the service.
#[test]
fn settings_are_kept_across_restarts() {
    let env = Env::new("restart");
    {
        let _service = env.daemon();
        let mut c = env.connect();
        c.hello();
        c.call(
            "config.set",
            json!({ "interval_s": 120, "impulse": "mouse", "max_hours": 6, "schedule_enabled": true }),
        )
        .unwrap();
        c.call("quit", json!({})).unwrap();
    }
    assert!(wait_until(Duration::from_secs(5), || !env
        .socket()
        .exists()));
    let _service = env.daemon();
    let mut c = env.connect();
    let cfg = c.call("config.get", json!({})).unwrap();
    assert_eq!(cfg["interval_s"], json!(120));
    assert_eq!(cfg["impulse"], json!("mouse"));
    assert_eq!(cfg["max_hours"], json!(6));
    assert_eq!(cfg["schedule_enabled"], json!(true));
    assert_eq!(cfg["enabled"], json!(false));
}

/// A settings file edited by hand with nonsense does not stop the service.
#[test]
fn a_broken_settings_file_gives_the_defaults() {
    let env = Env::new("broken");
    let path = env.config_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{ "interval_s": "soon", "impulse": "mouse", "schedule": 5, "extra": true }"#,
    )
    .unwrap();
    let _service = env.daemon();
    let mut c = env.connect();
    let cfg = c.call("config.get", json!({})).unwrap();
    assert_eq!(cfg["impulse"], json!("mouse"));
    assert_eq!(cfg["interval_s"], json!(240));
    assert_eq!(cfg["schedule"].as_array().unwrap().len(), 7);
}

/// `quit` answers, ends the service and removes the socket.
#[test]
fn quit_removes_the_socket() {
    let env = Env::new("quit");
    let mut service = env.daemon();
    let mut c = env.connect();
    c.hello();
    assert_eq!(c.call("quit", json!({})).unwrap(), json!(true));
    assert!(service.exits_within(Duration::from_secs(5)).is_some());
    assert!(!env.socket().exists());
}

/// A line over 1 MiB, or one that is not a JSON object, closes only that
/// connection.
#[test]
fn invalid_lines_close_only_their_connection() {
    let env = Env::new("invalid");
    let _service = env.daemon();
    let mut other = env.connect();
    other.hello();
    for line in [
        format!(
            "{{\"id\":1,\"method\":\"status\",\"params\":\"{}\"}}",
            "x".repeat(1 << 20)
        ),
        "[1, 2]".to_string(),
        "not json".to_string(),
    ] {
        let mut bad = env.connect();
        let _ = bad.stream.write_all(format!("{line}\n").as_bytes());
        assert!(
            bad.read().is_none(),
            "{} kept open",
            &line[..line.len().min(20)]
        );
    }
    assert_eq!(
        other.call("get", json!({})).unwrap()["values"]["impulse"],
        json!("key")
    );
}

/// The command line talks to the service; it starts one if none runs.
#[test]
fn the_command_line_shows_the_state() {
    let env = Env::new("cli");
    let out = env.command().arg("status").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("State:     off"), "{text}");
    assert!(text.contains("Interval:  4 min"), "{text}");
    assert!(text.contains("Schedule:  off, always"), "{text}");
    // The service was started on demand and answers.
    assert!(env.socket().exists());
    let mut c = env.connect();
    c.call("quit", json!({})).unwrap();
}

/// Switching on creates the device and sends at once, switching off removes
/// the device; impulses of both kinds and the test command arrive.
#[test]
fn impulses_arrive_and_the_device_goes_with_the_switch() {
    if !can_test_the_device() {
        return;
    }
    let env = Env::new("device");
    let _service = env.daemon();
    let watcher = watch();
    let mut c = env.connect();
    c.hello();

    let f15 = evdev::KeyCode::KEY_F15.code();
    c.call("config.set", json!({ "enabled": true })).unwrap();
    let first = watcher.next(Duration::from_secs(5));
    assert_eq!(first, Some((EventType::KEY, f15, 1)));
    assert_eq!(
        watcher.next(Duration::from_secs(2)),
        Some((EventType::KEY, f15, 0))
    );
    let status = c.call("status", json!({})).unwrap();
    assert_eq!(status["state"], json!("sending"));
    assert_eq!(status["sent"], json!(1));
    assert!(status["next_in_s"].as_u64().unwrap() <= 240);
    assert_eq!(our_nodes().len(), 1);

    // Another kind arrives at once, from a new device.
    c.call("config.set", json!({ "impulse": "mouse" })).unwrap();
    let rel_x = evdev::RelativeAxisCode::REL_X.0;
    assert_eq!(
        watcher.next(Duration::from_secs(5)),
        Some((EventType::RELATIVE, rel_x, 1))
    );
    assert_eq!(
        watcher.next(Duration::from_secs(2)),
        Some((EventType::RELATIVE, rel_x, -1))
    );
    assert_eq!(c.call("status", json!({})).unwrap()["sent"], json!(2));

    // The test command sends whatever the switch says.
    c.call("config.set", json!({ "enabled": false })).unwrap();
    assert!(wait_until(Duration::from_secs(3), || our_nodes().is_empty()));
    assert_eq!(c.call("status", json!({})).unwrap()["state"], json!("off"));
    let out = env.command().arg("test").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Sent one impulse"));
    assert_eq!(
        watcher.next(Duration::from_secs(5)),
        Some((EventType::RELATIVE, rel_x, 1))
    );
    assert_eq!(
        watcher.next(Duration::from_secs(2)),
        Some((EventType::RELATIVE, rel_x, -1))
    );
    assert!(wait_until(Duration::from_secs(3), || our_nodes().is_empty()));
    assert_eq!(c.call("status", json!({})).unwrap()["sent"], json!(2));
    assert!(watcher.next(Duration::from_millis(300)).is_none());
}

/// Ending the service removes the device too.
#[test]
fn the_device_is_gone_when_the_service_ends() {
    if !can_test_the_device() {
        return;
    }
    let env = Env::new("end");
    let mut service = env.daemon();
    let _watcher = watch();
    let mut c = env.connect();
    c.call("config.set", json!({ "enabled": true })).unwrap();
    assert!(wait_until(Duration::from_secs(3), || our_nodes().len() == 1));
    c.call("quit", json!({})).unwrap();
    assert!(service.exits_within(Duration::from_secs(5)).is_some());
    assert!(wait_until(Duration::from_secs(3), || our_nodes().is_empty()));
}
