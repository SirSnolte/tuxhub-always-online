//! The background service. It owns the configuration and the engine and
//! sends the impulses.
//!
//! Lifetime: **permanent**. The service runs without any client, starts at
//! login and keeps going when the window and tuxhub are closed, because
//! keeping you online is its whole job. Without an enabled switch (the state
//! after the installation) it only waits and creates no device.
//!
//! Exactly one service runs per user. The window, tuxhub and the command line
//! talk to it over the Unix socket of the tuxhub live interface
//! (`$XDG_RUNTIME_DIR/tuxhub/always-online.sock`, one JSON object per line),
//! so all of them always show the same state.

use crate::config::{Config, Impulse};
use crate::engine::{Engine, State};
use crate::impulse::{self, Device};
use crate::integration;
use crate::live::{self, Failure};
use crate::paths;
use crate::schedule::{self, Moment};
use crate::service::{Clients, Writer, bind, read_line, try_lock};
use serde_json::{Value, json};
use std::io::{self, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LOCK_NAME: &str = "tuxhub-always-online.lock";

pub fn socket_path() -> io::Result<PathBuf> {
    Ok(paths::runtime_dir()?
        .join("tuxhub")
        .join(format!("{}.sock", live::TOOL_ID)))
}

/// Whatever sends the impulses; the virtual device, or a fake in tests.
pub trait Output {
    fn send(&mut self) -> io::Result<()>;
    fn kind(&self) -> Impulse;
}

impl Output for Device {
    fn send(&mut self) -> io::Result<()> {
        Device::send(self)
    }

    fn kind(&self) -> Impulse {
        Device::kind(self)
    }
}

type Factory = Box<dyn FnMut(Impulse) -> io::Result<Box<dyn Output>>>;
/// The local day and time, and a monotonic time that does not count the time
/// the computer was suspended, so a night in standby is not part of a run.
type Clock = Box<dyn Fn() -> (Moment, Duration)>;

enum Msg {
    Request {
        /// Connection the request came from.
        client: u64,
        method: String,
        params: Value,
        reply: Sender<Result<Value, Failure>>,
    },
    /// The access setup has ended; wakes the loop.
    SetupDone,
    Terminate,
}

/// Runs the service until it is terminated. Returns `Ok(false)` if another
/// instance is already running.
pub fn run() -> io::Result<bool> {
    let dir = paths::runtime_dir()?;
    std::fs::create_dir_all(&dir)?;
    let Some(lock) = try_lock(&dir.join(LOCK_NAME))? else {
        return Ok(false);
    };
    let socket = socket_path()?;
    let server = bind(&socket)?;

    let (tx, rx) = std::sync::mpsc::channel();
    {
        let tx = tx.clone();
        ctrlc::set_handler(move || {
            let _ = tx.send(Msg::Terminate);
        })
        .map_err(io::Error::other)?;
    }

    let cfg = Config::load();
    if let Err(e) = integration::refresh(&cfg) {
        eprintln!("desktop integration: {e}");
    }
    let clients = Clients::default();
    {
        let (clients, tx) = (clients.clone(), tx.clone());
        std::thread::Builder::new()
            .name("accept".into())
            .spawn(move || accept(server, clients, tx))?;
    }

    let mut service = Service::new(
        cfg,
        clients,
        tx,
        Box::new(|kind| Ok(Box::new(Device::new(kind)?) as Box<dyn Output>)),
        Box::new(real_clock),
        Some(Config::path()?),
    );
    let restart = service.run(rx);
    let exe = service.exe.take();
    // Removes the virtual device before the socket disappears.
    drop(service);
    let _ = std::fs::remove_file(&socket);
    drop(lock);
    if restart {
        exec_self(exe.as_ref().map(|(p, _)| p));
    }
    Ok(true)
}

/// Replaces the process with the current executable, after an update.
/// `path` is the executable as it was at start. `current_exe()` cannot be
/// used here: after an update it names the deleted old file.
fn exec_self(path: Option<&PathBuf>) {
    use std::os::unix::process::CommandExt;
    if let Some(exe) = path {
        let err = std::process::Command::new(exe).arg("daemon").exec();
        eprintln!("could not restart after an update: {err}");
    }
}

/// Device and inode of a file, to detect when an update replaced it.
fn file_id(path: &PathBuf) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// The local day and time from the system clock, and the monotonic time.
fn real_clock() -> (Moment, Duration) {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec that the call fills in.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    let monotonic = Duration::new(
        ts.tv_sec.max(0) as u64,
        ts.tv_nsec.clamp(0, 999_999_999) as u32,
    );

    // SAFETY: `time` accepts a null pointer; `tm` is zeroed and then filled
    // in by `localtime_r`, which only reads `secs`. `localtime_r` does not look
    // at the time zone again after the first call, so a service that runs for
    // weeks would miss a change in the system settings; `tzset` does.
    let local = unsafe {
        unsafe extern "C" {
            // From the C library; the `libc` crate does not declare it.
            fn tzset();
        }
        tzset();
        let secs = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&secs, &mut tm).is_null()).then_some(tm)
    };
    let moment = local
        .and_then(|tm| moment_from_tm(tm.tm_wday, tm.tm_hour, tm.tm_min))
        .unwrap_or(Moment {
            weekday: 0,
            minute: 0,
        });
    (moment, monotonic)
}

/// `tm_wday` counts from Sunday (0), the schedule from Monday (0).
fn moment_from_tm(wday: i32, hour: i32, minute: i32) -> Option<Moment> {
    if !(0..=6).contains(&wday) {
        return None;
    }
    let weekday = u8::try_from((wday + 6) % 7).ok()?;
    Moment::new(
        weekday,
        u16::try_from(hour).ok()?,
        u16::try_from(minute).ok()?,
    )
}

fn accept(server: UnixListener, clients: Clients, tx: Sender<Msg>) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    for stream in server.incoming() {
        let Ok(stream) = stream else { continue };
        let (Ok(read), true) = (
            stream.try_clone(),
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .is_ok(),
        ) else {
            continue;
        };
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let writer: Writer = Arc::new(Mutex::new(stream));
        clients.add(id, writer.clone());
        let (clients, tx) = (clients.clone(), tx.clone());
        let _ = std::thread::Builder::new()
            .name(format!("client-{id}"))
            .spawn(move || {
                serve_client(id, read, &writer, &tx);
                clients.remove(id);
                // Wakes up a client that is still blocked on its socket.
                let _ = writer.lock().unwrap().shutdown(std::net::Shutdown::Both);
            });
    }
}

/// Answers the requests of one connection. A line that is too long or not a
/// JSON object closes it.
fn serve_client(client: u64, read: UnixStream, writer: &Writer, tx: &Sender<Msg>) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    while read_line(&mut reader, &mut buf).is_some() {
        let Ok(msg @ Value::Object(_)) = serde_json::from_slice::<Value>(&buf) else {
            break;
        };
        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let (rtx, rrx) = std::sync::mpsc::channel();
        if tx
            .send(Msg::Request {
                client,
                method,
                params,
                reply: rtx,
            })
            .is_err()
        {
            break;
        }
        let result = rrx
            .recv_timeout(Duration::from_secs(4))
            .unwrap_or_else(|_| Err("the service did not respond".into()));
        if let Some(id) = id {
            let reply = match result {
                Ok(r) => json!({ "id": id, "result": r }),
                Err(Failure {
                    message,
                    field: Some(field),
                }) => json!({ "id": id, "error": { "message": message, "field": field } }),
                Err(e) => json!({ "id": id, "error": { "message": e.message } }),
            };
            if writeln!(writer.lock().unwrap(), "{reply}").is_err() {
                break;
            }
        }
    }
}

// ----------------------------------------------------------------- Service

struct Service {
    cfg: Config,
    engine: Engine,
    /// The virtual device; exists only while sending.
    device: Option<Box<dyn Output>>,
    factory: Factory,
    clock: Clock,
    clients: Clients,
    tx: Sender<Msg>,
    /// Where the settings are saved; `None` in tests.
    store: Option<PathBuf>,
    /// Why the latest impulse could not be sent.
    error: Option<String>,
    /// Impulses sent since the service started.
    sent: u64,
    /// The status as last published, without the countdowns.
    published: Value,
    /// The executable as it was at start, to notice an update.
    exe: Option<(PathBuf, (u64, u64))>,
}

impl Service {
    fn new(
        cfg: Config,
        clients: Clients,
        tx: Sender<Msg>,
        factory: Factory,
        clock: Clock,
        store: Option<PathBuf>,
    ) -> Self {
        Self {
            cfg,
            engine: Engine::new(),
            device: None,
            factory,
            clock,
            clients,
            tx,
            store,
            error: None,
            sent: 0,
            published: Value::Null,
            exe: std::env::current_exe()
                .ok()
                .and_then(|p| Some((p.clone(), file_id(&p)?))),
        }
    }

    /// Processes messages until the service ends. Returns true if the
    /// executable was replaced and the service should restart itself.
    fn run(&mut self, rx: Receiver<Msg>) -> bool {
        let mut wake = self.tick();
        self.publish_status();
        loop {
            match rx.recv_timeout(wake) {
                Ok(Msg::Terminate) | Err(RecvTimeoutError::Disconnected) => return false,
                Ok(Msg::Request {
                    client,
                    method,
                    params,
                    reply,
                }) => {
                    if method == "quit" {
                        let _ = reply.send(Ok(json!(true)));
                        return false;
                    }
                    let _ = reply.send(self.request(client, &method, params));
                }
                // The setup ended: the loop goes on to the next tick at once,
                // which tries the device again.
                Ok(Msg::SetupDone) => {}
                Err(RecvTimeoutError::Timeout) => {}
            }
            wake = self.tick();
            self.publish_status();
            if self.exe_replaced() {
                return true;
            }
        }
    }

    fn exe_replaced(&self) -> bool {
        let Some((path, id)) = &self.exe else {
            return false;
        };
        // A missing file means the tool is being removed, not updated.
        file_id(path).is_some_and(|now| now != *id)
    }

    /// Lets the engine decide and does what it says. Returns how long the
    /// service may sleep.
    fn tick(&mut self) -> Duration {
        let (local, now) = (self.clock)();
        let decision = self.engine.step(&self.cfg, local, now);
        if decision.state != State::Sending {
            self.device = None;
            self.error = None;
        } else if decision.send {
            self.send_impulse();
        }
        decision.wake_in
    }

    /// Sends one impulse, creating the device first if there is none. A failure
    /// is kept for the status and tried again with the next impulse.
    fn send_impulse(&mut self) {
        if self.device.is_none() {
            match (self.factory)(self.cfg.impulse) {
                Ok(device) => self.device = Some(device),
                Err(e) => {
                    self.error = Some(e.to_string());
                    // Try again at the next wake-up, not only after a whole
                    // interval: access may come back at any time.
                    self.engine.retry();
                    return;
                }
            }
        }
        let Some(device) = self.device.as_mut() else {
            return;
        };
        match device.send() {
            Ok(()) => {
                self.sent += 1;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.device = None;
                self.engine.retry();
            }
        }
    }

    fn state_name(&self) -> &'static str {
        match (self.engine.state(), &self.error) {
            (State::Sending, Some(_)) => "no_access",
            (state, _) => state.name(),
        }
    }

    fn status(&self) -> Value {
        let (local, now) = (self.clock)();
        let secs_up = |d: Duration| d.as_secs() + u64::from(d.subsec_nanos() > 0);
        json!({
            "state": self.state_name(),
            "next_in_s": self.engine.next_impulse_in(&self.cfg, now).map(secs_up),
            "run_elapsed_s": self.engine.run_elapsed(now).map(|d| d.as_secs()),
            "sent": self.sent,
            "last_error": self.error,
            "schedule_active": self.cfg.schedule_enabled.then(|| schedule::is_active(&self.cfg, local)),
            "schedule_change_in_min": schedule::next_change(&self.cfg, local),
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    /// Tells the clients about a status that changed. The countdowns change
    /// all the time; the clients count them down themselves.
    fn publish_status(&mut self) {
        let mut status = self.status();
        let mut key = status.clone();
        for countdown in ["next_in_s", "run_elapsed_s", "schedule_change_in_min"] {
            key[countdown] = Value::Null;
        }
        if key != self.published {
            self.clients.notify("status", std::mem::take(&mut status));
            self.published = key;
        }
    }

    /// Saves and applies `cfg`. `origin` is the connection that set a value
    /// over the live interface; it gets no `changed` for it.
    fn set_config(&mut self, cfg: Config, origin: Option<u64>) -> Result<(), String> {
        if cfg == self.cfg {
            return Ok(());
        }
        if let Some(path) = &self.store {
            cfg.save_to(path)
                .map_err(|e| format!("could not save the settings: {e}"))?;
        }
        let old = std::mem::replace(&mut self.cfg, cfg);
        if old.impulse != self.cfg.impulse {
            // The new kind goes out at once, from a new device.
            self.device = None;
            self.engine.retry();
        }
        if old.start_at_login != self.cfg.start_at_login
            && let Err(e) = integration::refresh(&self.cfg)
        {
            eprintln!("desktop integration: {e}");
        }
        let values = live::values(&self.cfg);
        if live::values(&old) != values {
            self.clients
                .notify_except(origin, "changed", json!({ "values": values }));
        }
        self.clients.notify("config", json!(self.cfg));
        Ok(())
    }

    /// Sends one impulse of the chosen kind, whatever the state. Uses a device
    /// of its own if the service has none.
    fn test_impulse(&mut self) -> Result<Value, Failure> {
        let result = match self.device.as_mut() {
            Some(device) => device.send(),
            None => (self.factory)(self.cfg.impulse).and_then(|mut device| device.send()),
        };
        result.map_err(|e| Failure::from(e.to_string()))?;
        Ok(json!(true))
    }

    fn start_setup(&self) {
        let (clients, tx) = (self.clients.clone(), self.tx.clone());
        // The password dialog can take a while; do not block the service.
        std::thread::spawn(move || {
            let result = impulse::setup_access();
            clients.notify(
                "setup",
                json!({ "error": result.err().map(|e| e.to_string()) }),
            );
            let _ = tx.send(Msg::SetupDone);
        });
    }

    fn request(&mut self, client: u64, method: &str, params: Value) -> Result<Value, Failure> {
        match method {
            "hello" => live::hello(&params),
            "schema" => Ok(live::schema(
                params.get("locale").and_then(Value::as_str).unwrap_or("en"),
            )),
            "get" => Ok(json!({ "values": live::values(&self.cfg) })),
            "set" => {
                let (cfg, value) = live::set(&self.cfg, &params)?;
                self.set_config(cfg, Some(client))?;
                Ok(json!({ "value": value }))
            }
            "status" => Ok(self.status()),
            "config.get" => Ok(json!(self.cfg)),
            "config.set" => {
                let cfg = self.cfg.merged(params)?;
                self.set_config(cfg, None)?;
                Ok(json!(self.cfg))
            }
            "impulse.test" => self.test_impulse(),
            "access.setup" => {
                self.start_setup();
                Ok(json!(true))
            }
            "action" => {
                let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
                let enabled = match id {
                    "toggle" => !self.cfg.enabled,
                    "enable" => true,
                    "disable" => false,
                    "test" => return self.test_impulse(),
                    "setup" => {
                        self.start_setup();
                        return Ok(json!(true));
                    }
                    other => return Err(format!("unknown action {other:?}").into()),
                };
                let cfg = self.cfg.merged(json!({ "enabled": enabled }))?;
                self.set_config(cfg, None)?;
                Ok(json!(self.cfg.enabled))
            }
            other => Err(format!("unknown method {other:?}").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::io::{BufRead, BufReader};
    use std::rc::Rc;

    /// What the fake device and the fake clock share with the test.
    struct World {
        log: RefCell<Vec<String>>,
        fail_create: Cell<Option<&'static str>>,
        /// How often the service tried to create the device.
        attempts: Cell<u32>,
        fail_send: Cell<bool>,
        time: Cell<(Moment, Duration)>,
    }

    impl World {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                log: RefCell::default(),
                fail_create: Cell::new(None),
                attempts: Cell::new(0),
                fail_send: Cell::new(false),
                time: Cell::new((at(0, 12, 0), Duration::ZERO)),
            })
        }

        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.borrow_mut())
        }

        /// Moves the monotonic clock; the local time stays.
        fn advance(&self, secs: u64) {
            let (local, now) = self.time.get();
            self.time.set((local, now + Duration::from_secs(secs)));
        }

        fn set_local(&self, local: Moment) {
            self.time.set((local, self.time.get().1));
        }
    }

    struct Fake {
        kind: Impulse,
        world: Rc<World>,
    }

    impl Output for Fake {
        fn send(&mut self) -> io::Result<()> {
            if self.world.fail_send.get() {
                return Err(io::Error::other("send failed"));
            }
            self.world.log.borrow_mut().push("send".into());
            Ok(())
        }

        fn kind(&self) -> Impulse {
            self.kind
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.world.log.borrow_mut().push("drop".into());
        }
    }

    fn at(weekday: u8, hh: u16, mm: u16) -> Moment {
        Moment::new(weekday, hh, mm).unwrap()
    }

    fn cfg(patch: Value) -> Config {
        Config::default().merged(patch).unwrap()
    }

    fn service(world: &Rc<World>, cfg: Config) -> (Service, Receiver<Msg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (w1, w2) = (world.clone(), world.clone());
        let factory: Factory = Box::new(move |kind| {
            w1.attempts.set(w1.attempts.get() + 1);
            if let Some(why) = w1.fail_create.get() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
            }
            let name = format!("create {}", json!(kind).as_str().unwrap());
            w1.log.borrow_mut().push(name);
            Ok(Box::new(Fake {
                kind,
                world: w1.clone(),
            }))
        });
        let clock: Clock = Box::new(move || w2.time.get());
        (
            Service::new(cfg, Clients::default(), tx, factory, clock, None),
            rx,
        )
    }

    fn ask(s: &mut Service, method: &str, params: Value) -> Result<Value, Failure> {
        s.request(1, method, params)
    }

    /// Adds a connection that can be read.
    fn connect(s: &Service, id: u64) -> BufReader<UnixStream> {
        let (ours, theirs) = UnixStream::pair().unwrap();
        theirs
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        s.clients.add(id, Arc::new(Mutex::new(ours)));
        BufReader::new(theirs)
    }

    /// The notifications received so far, as `(method, params)`.
    fn notes(reader: &mut BufReader<UnixStream>) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
            let v: Value = serde_json::from_str(&line).unwrap();
            out.push((
                v["method"].as_str().unwrap().to_string(),
                v["params"].clone(),
            ));
            line.clear();
        }
        out
    }

    fn methods(notes: &[(String, Value)]) -> Vec<&str> {
        notes.iter().map(|(m, _)| m.as_str()).collect()
    }

    #[test]
    fn the_week_starts_on_monday() {
        // tm_wday counts from Sunday.
        for (wday, weekday) in [(1, 0), (2, 1), (5, 4), (6, 5), (0, 6)] {
            assert_eq!(moment_from_tm(wday, 7, 5), Moment::new(weekday, 7, 5));
        }
        assert_eq!(moment_from_tm(7, 0, 0), None);
        assert_eq!(moment_from_tm(-1, 0, 0), None);
        assert_eq!(moment_from_tm(1, 24, 0), None);
        assert_eq!(moment_from_tm(1, 0, 60), None);
        assert_eq!(moment_from_tm(1, -1, 0), None);
    }

    /// Milliseconds of `clock` as the kernel reports it.
    fn kernel_clock_ms(clock: libc::clockid_t) -> u128 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe { libc::clock_gettime(clock, &mut ts) };
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32).as_millis()
    }

    #[test]
    fn the_real_clock_does_not_count_the_time_the_computer_slept() {
        // CLOCK_BOOTTIME runs on during suspend, CLOCK_MONOTONIC does not. A
        // computer that slept before this test shows the difference; one that
        // never did cannot tell them apart.
        let ms = real_clock().1.as_millis();
        let monotonic = kernel_clock_ms(libc::CLOCK_MONOTONIC);
        assert!(
            ms.abs_diff(monotonic) < 100,
            "{ms} ms against {monotonic} ms"
        );
    }

    /// Changes the time zone of this process, as the system settings do.
    /// Runs alone: tests run with `--test-threads=1`.
    #[test]
    fn the_real_clock_follows_a_changed_time_zone() {
        // POSIX zone strings count east of Greenwich as negative.
        unsafe { std::env::set_var("TZ", "UTC0") };
        let before = real_clock().0;
        unsafe { std::env::set_var("TZ", "UTC-14") };
        let after = real_clock().0;
        unsafe { std::env::remove_var("TZ") };
        // 14 hours later in the day, give or take a minute rolling over.
        let shift = (after.minute + 1440 - before.minute) % 1440;
        assert!((839..=841).contains(&shift), "{before:?} then {after:?}");
    }

    #[test]
    fn the_real_clock_is_plausible() {
        let (local, boot) = real_clock();
        assert!(local.weekday < 7 && local.minute < 1440);
        assert!(boot > Duration::ZERO);
        // The monotonic time never runs backwards.
        assert!(real_clock().1 >= boot);
    }

    #[test]
    fn enabling_creates_the_device_and_sends_at_once() {
        let world = World::new();
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true, "interval_s": 240 })));
        assert_eq!(s.tick(), crate::engine::MAX_SLEEP);
        assert_eq!(world.take_log(), ["create key", "send"]);
        let status = s.status();
        assert_eq!(status["state"], "sending");
        assert_eq!(status["sent"], 1);
        assert_eq!(status["next_in_s"], 240);
        // Nothing before the interval is over, then one more on the same device.
        world.advance(100);
        s.tick();
        assert!(world.log().is_empty());
        world.advance(140);
        s.tick();
        assert_eq!(world.take_log(), ["send"]);
        assert_eq!(s.status()["sent"], 2);
        assert_eq!(s.status()["run_elapsed_s"], 240);
    }

    #[test]
    fn nothing_is_created_while_the_switch_is_off() {
        let world = World::new();
        let (mut s, _rx) = service(&world, Config::default());
        for _ in 0..3 {
            s.tick();
            world.advance(1000);
        }
        assert!(world.log().is_empty());
        let status = s.status();
        assert_eq!(status["state"], "off");
        assert_eq!(status["sent"], 0);
        assert_eq!(status["next_in_s"], Value::Null);
        assert_eq!(status["schedule_active"], Value::Null);
    }

    #[test]
    fn switching_the_impulse_replaces_the_device_and_sends_at_once() {
        let world = World::new();
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true })));
        s.tick();
        world.take_log();
        ask(&mut s, "config.set", json!({ "impulse": "mouse" })).unwrap();
        world.advance(5);
        s.tick();
        assert_eq!(world.take_log(), ["drop", "create mouse", "send"]);
        assert_eq!(s.status()["sent"], 2);
    }

    #[test]
    fn disabling_removes_the_device() {
        let world = World::new();
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true })));
        s.tick();
        world.take_log();
        assert_eq!(
            ask(&mut s, "action", json!({ "id": "toggle" })).unwrap(),
            json!(false)
        );
        s.tick();
        assert_eq!(world.take_log(), ["drop"]);
        assert_eq!(s.status()["state"], "off");
        assert_eq!(s.status()["run_elapsed_s"], Value::Null);
        // And on again with a new device.
        assert_eq!(
            ask(&mut s, "action", json!({ "id": "enable" })).unwrap(),
            json!(true)
        );
        s.tick();
        assert_eq!(world.take_log(), ["create key", "send"]);
    }

    #[test]
    fn follows_the_schedule_and_reports_when_it_changes() {
        let world = World::new();
        world.set_local(at(0, 3, 0));
        let (mut s, _rx) = service(
            &world,
            cfg(json!({ "enabled": true, "schedule_enabled": true })),
        );
        s.tick();
        assert!(world.log().is_empty());
        let status = s.status();
        assert_eq!(status["state"], "outside");
        assert_eq!(status["schedule_active"], false);
        assert_eq!(status["schedule_change_in_min"], 300);

        world.set_local(at(0, 8, 0));
        world.advance(60);
        s.tick();
        assert_eq!(world.take_log(), ["create key", "send"]);
        let status = s.status();
        assert_eq!(status["schedule_active"], true);
        assert_eq!(status["schedule_change_in_min"], 15 * 60);

        world.set_local(at(0, 23, 0));
        world.advance(60);
        s.tick();
        assert_eq!(world.take_log(), ["drop"]);
        assert_eq!(s.status()["state"], "outside");
    }

    #[test]
    fn missing_access_is_reported_and_tried_again_at_every_wake_up() {
        let world = World::new();
        world.fail_create.set(Some("no access to /dev/uinput"));
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true, "interval_s": 240 })));
        s.tick();
        let status = s.status();
        assert_eq!(status["state"], "no_access");
        assert!(
            status["last_error"]
                .as_str()
                .unwrap()
                .contains("/dev/uinput")
        );
        assert_eq!(status["sent"], 0);
        assert_eq!(world.attempts.get(), 1);
        // Still failing at the next wake-up; the service keeps going.
        world.advance(30);
        s.tick();
        assert_eq!(s.status()["state"], "no_access");
        assert_eq!(world.attempts.get(), 2);
        // Access is granted: the next wake-up works and clears the problem.
        world.fail_create.set(None);
        world.advance(30);
        s.tick();
        let status = s.status();
        assert_eq!(
            (status["state"].as_str(), status["sent"].as_u64()),
            (Some("sending"), Some(1))
        );
        assert_eq!(status["last_error"], Value::Null);
        assert_eq!(world.attempts.get(), 3);
        // Back to the normal pace: nothing more before the interval is over.
        world.advance(30);
        s.tick();
        assert_eq!(world.attempts.get(), 3);
    }

    #[test]
    fn access_that_comes_back_is_noticed_long_before_a_long_interval_is_over() {
        let world = World::new();
        world.fail_create.set(Some("denied"));
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true, "interval_s": 1800 })));
        s.tick();
        assert_eq!(s.status()["state"], "no_access");
        world.fail_create.set(None);
        world.advance(30);
        s.tick();
        assert_eq!(s.status()["state"], "sending");
        assert_eq!(world.take_log(), ["create key", "send"]);
    }

    #[test]
    fn a_failed_send_drops_the_device_and_the_next_impulse_starts_over() {
        let world = World::new();
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true, "interval_s": 240 })));
        s.tick();
        world.take_log();
        world.fail_send.set(true);
        world.advance(240);
        s.tick();
        assert_eq!(world.take_log(), ["drop"]);
        let status = s.status();
        assert_eq!(status["state"], "no_access");
        assert_eq!(status["last_error"], "send failed");
        world.fail_send.set(false);
        world.advance(240);
        s.tick();
        assert_eq!(world.take_log(), ["create key", "send"]);
        assert_eq!(s.status()["state"], "sending");
    }

    #[test]
    fn the_test_impulse_works_whatever_the_switch_says() {
        let world = World::new();
        let (mut s, _rx) = service(&world, Config::default());
        assert_eq!(
            ask(&mut s, "impulse.test", Value::Null).unwrap(),
            json!(true)
        );
        assert_eq!(world.take_log(), ["create key", "send", "drop"]);
        assert_eq!(s.status()["sent"], 0, "tests are not counted");
        assert_eq!(s.status()["state"], "off");
        assert!(ask(&mut s, "action", json!({ "id": "test" })).is_ok());
        assert_eq!(world.take_log(), ["create key", "send", "drop"]);

        // While sending it uses the device that is there.
        ask(&mut s, "config.set", json!({ "enabled": true })).unwrap();
        s.tick();
        world.take_log();
        ask(&mut s, "impulse.test", Value::Null).unwrap();
        assert_eq!(world.take_log(), ["send"]);

        // A problem is returned to the caller.
        let world = World::new();
        world.fail_create.set(Some("no access to /dev/uinput"));
        let (mut s, _rx) = service(&world, Config::default());
        let e = ask(&mut s, "impulse.test", Value::Null).unwrap_err();
        assert!(e.message.contains("/dev/uinput"), "{e:?}");
    }

    #[test]
    fn answers_the_live_methods() {
        let world = World::new();
        let (mut s, _rx) = service(&world, Config::default());
        assert_eq!(
            ask(&mut s, "hello", json!({ "protocol": 1 })).unwrap()["protocol"],
            1
        );
        assert!(ask(&mut s, "hello", json!({ "protocol": 9 })).is_err());
        assert_eq!(
            ask(&mut s, "schema", json!({ "locale": "de" })).unwrap(),
            live::schema("de")
        );
        assert_eq!(
            ask(&mut s, "schema", Value::Null).unwrap(),
            live::schema("en")
        );
        assert_eq!(
            ask(&mut s, "get", Value::Null).unwrap()["values"]["interval_s"],
            240
        );
        assert_eq!(
            ask(&mut s, "set", json!({ "key": "interval_s", "value": 90 })).unwrap(),
            json!({ "value": 90 })
        );
        assert_eq!(
            ask(&mut s, "config.get", Value::Null).unwrap()["interval_s"],
            90
        );
        let e = ask(&mut s, "set", json!({ "key": "interval_s", "value": 5 })).unwrap_err();
        assert_eq!(e.field.as_deref(), Some("interval_s"));
        assert_eq!(s.cfg.interval_s, 90);
        assert!(
            ask(&mut s, "nope", Value::Null)
                .unwrap_err()
                .message
                .contains("nope")
        );
        assert!(ask(&mut s, "action", json!({ "id": "dance" })).is_err());
        assert!(ask(&mut s, "action", Value::Null).is_err());
    }

    #[test]
    fn settings_are_saved_and_a_refused_change_is_not() {
        let dir = std::env::temp_dir().join(format!("tao-daemon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.json");
        let world = World::new();
        let (mut s, _rx) = service(&world, Config::default());
        s.store = Some(path.clone());

        ask(
            &mut s,
            "config.set",
            json!({ "max_hours": 6, "schedule_enabled": true }),
        )
        .unwrap();
        let saved = Config::load_from(&path);
        assert_eq!((saved.max_hours, saved.schedule_enabled), (6, true));

        // Invalid values are refused before anything is written.
        assert!(ask(&mut s, "config.set", json!({ "max_hours": 99 })).is_err());
        assert!(ask(&mut s, "config.set", json!({ "schedule": [] })).is_err());
        assert_eq!(Config::load_from(&path), saved);
        assert_eq!(s.cfg, saved);

        // A file that cannot be written leaves the settings as they were.
        s.store = Some(PathBuf::from("/proc/tuxhub-always-online-test/config.json"));
        let e = ask(&mut s, "config.set", json!({ "max_hours": 7 })).unwrap_err();
        assert!(e.message.contains("could not save"), "{e:?}");
        assert_eq!(s.cfg.max_hours, 6);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tells_the_other_connections_about_a_change() {
        let world = World::new();
        let (mut s, _rx) = service(&world, Config::default());
        let (mut one, mut two) = (connect(&s, 1), connect(&s, 2));
        // Connection 1 sets a value over the live interface.
        s.request(1, "set", json!({ "key": "interval_s", "value": 60 }))
            .unwrap();
        let (a, b) = (notes(&mut one), notes(&mut two));
        assert_eq!(methods(&a), ["config"]);
        assert_eq!(methods(&b), ["changed", "config"]);
        assert_eq!(
            b[0].1,
            json!({ "values": { "enabled": false, "interval_s": 60, "impulse": "key", "max_hours": 0, "start_at_login": true } })
        );
        // Everyone is told about a change made another way.
        s.request(1, "config.set", json!({ "max_hours": 3 }))
            .unwrap();
        assert_eq!(methods(&notes(&mut one)), ["changed", "config"]);
        assert_eq!(methods(&notes(&mut two)), ["changed", "config"]);
        // A setting that is not part of the schema sends no `changed`.
        s.request(1, "config.set", json!({ "schedule_enabled": true }))
            .unwrap();
        assert_eq!(methods(&notes(&mut one)), ["config"]);
        // Nothing changed, nothing is sent.
        s.request(1, "config.set", json!({ "schedule_enabled": true }))
            .unwrap();
        assert!(notes(&mut one).is_empty());
    }

    #[test]
    fn publishes_the_status_when_it_changes_but_not_for_the_countdown() {
        let world = World::new();
        let (mut s, _rx) = service(&world, cfg(json!({ "enabled": true, "interval_s": 240 })));
        let mut client = connect(&s, 1);
        s.tick();
        s.publish_status();
        let first = notes(&mut client);
        assert_eq!(methods(&first), ["status"]);
        assert_eq!(first[0].1["state"], "sending");
        // The countdown runs, nothing is published.
        for _ in 0..3 {
            world.advance(50);
            s.tick();
            s.publish_status();
        }
        assert!(notes(&mut client).is_empty());
        // A new impulse changes the counter.
        world.advance(100);
        s.tick();
        s.publish_status();
        let next = notes(&mut client);
        assert_eq!(
            (methods(&next), next[0].1["sent"].clone()),
            (vec!["status"], json!(2))
        );
        // So does a change of the state.
        ask(&mut s, "config.set", json!({ "enabled": false })).unwrap();
        s.tick();
        s.publish_status();
        assert_eq!(notes(&mut client).last().unwrap().1["state"], "off");
    }

    #[test]
    fn the_loop_answers_requests_and_ends_with_quit() {
        let world = World::new();
        let (mut s, rx) = service(&world, cfg(json!({ "enabled": true })));
        let tx = s.tx.clone();
        let call = |method: &str| {
            let (reply, answer) = std::sync::mpsc::channel();
            tx.send(Msg::Request {
                client: 1,
                method: method.into(),
                params: Value::Null,
                reply,
            })
            .unwrap();
            answer
        };
        let status = call("status");
        let quit = call("quit");
        s.run(rx);
        assert_eq!(status.recv().unwrap().unwrap()["state"], "sending");
        assert_eq!(quit.recv().unwrap().unwrap(), json!(true));
        drop(s);
        assert_eq!(world.log().last().unwrap(), "drop", "{:?}", world.log());
    }

    #[test]
    fn the_loop_ends_when_terminated() {
        let world = World::new();
        let (mut s, rx) = service(&world, Config::default());
        s.tx.send(Msg::Terminate).unwrap();
        s.run(rx);
        // And when nobody can send any more.
        let (mut s, rx) = service(&world, Config::default());
        drop(std::mem::replace(&mut s.tx, std::sync::mpsc::channel().0));
        s.run(rx);
    }
}
