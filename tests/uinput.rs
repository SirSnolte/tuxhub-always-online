//! Tests with a real uinput device.
//!
//! Each test grabs the device exclusively (EVIOCGRAB) before the first impulse
//! is sent, so no event ever reaches the desktop. Without access to
//! /dev/uinput or /dev/input the tests say so and return, as in the other
//! tuxhub tools (CI machines have neither); run them on a desktop machine.

use evdev::{Device as Reader, EventType, KeyCode, RelativeAxisCode};
use std::mem::ManuallyDrop;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tuxhub_always_online::config::Impulse;
use tuxhub_always_online::impulse::{DEVICE_NAME, Device};

/// A device and a grabbed reader on its node.
struct Setup {
    device: Option<Device>,
    node: PathBuf,
    events: mpsc::Receiver<(EventType, u16, i32)>,
    /// The capabilities seen by the reader.
    keys: Vec<KeyCode>,
    axes: Vec<RelativeAxisCode>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl Drop for Setup {
    /// Lets the reader release its grab while the device still exists, before
    /// the fields (and with them the device) are dropped.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
    }
}

fn setup(kind: Impulse) -> Option<Setup> {
    let mut device = match Device::new(kind) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    let (node, mut reader) = match open_own_node(&mut device) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    reader
        .grab()
        .expect("the grab must succeed, otherwise real input would reach the desktop");
    let keys = reader
        .supported_keys()
        .map(|k| k.iter().collect())
        .unwrap_or_default();
    let axes = reader
        .supported_relative_axes()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    reader.set_nonblocking(true).expect("non-blocking reads");
    let (tx, events) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let handle = std::thread::spawn({
        let stop = stop.clone();
        move || {
            while !stop.load(Ordering::Relaxed) {
                match reader.fetch_events() {
                    Ok(batch) => {
                        for ev in batch {
                            let _ = tx.send((ev.event_type(), ev.code(), ev.value()));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
            // The device may be gone already; a normal drop would complain
            // that it cannot release the grab then.
            let mut reader = ManuallyDrop::new(reader);
            let _ = reader.ungrab();
            unsafe { libc::close(reader.as_raw_fd()) };
        }
    });
    Some(Setup {
        device: Some(device),
        node,
        events,
        keys,
        axes,
        stop,
        reader: Some(handle),
    })
}

impl Setup {
    fn dev(&mut self) -> &mut Device {
        self.device.as_mut().expect("the device is still there")
    }
}

/// Opens the event node of the virtual device. Checks the name, so a physical
/// device is never grabbed by accident, and waits until udev has applied the
/// permissions of the new node.
fn open_own_node(device: &mut Device) -> Result<(PathBuf, Reader), String> {
    let end = Instant::now() + Duration::from_secs(3);
    let mut last_err = String::from("no event node found");
    while Instant::now() < end {
        let nodes = device.dev_nodes().map_err(|e| e.to_string())?;
        for node in nodes.iter().filter(|n| {
            n.file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("event"))
        }) {
            match Reader::open(node) {
                Ok(d) if d.name() == Some(DEVICE_NAME) => return Ok((node.clone(), d)),
                Ok(d) => {
                    last_err = format!("{} is {:?}, not the test device", node.display(), d.name())
                }
                Err(e) => last_err = format!("{}: {e}", node.display()),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(last_err)
}

/// Events that are not sync events.
fn collect(s: &Setup, n: usize, timeout: Duration) -> Vec<(EventType, u16, i32)> {
    let end = Instant::now() + timeout;
    let mut out = Vec::new();
    while out.len() < n {
        match s
            .events
            .recv_timeout(end.saturating_duration_since(Instant::now()))
        {
            Ok(ev) if ev.0 == EventType::SYNCHRONIZATION => {}
            Ok(ev) => out.push(ev),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn the_key_impulse_presses_and_releases_f15_and_nothing_else() {
    let Some(mut s) = setup(Impulse::Key) else {
        return;
    };
    assert_eq!(s.keys, [KeyCode::KEY_F15]);
    assert!(s.axes.is_empty());
    assert_eq!(s.dev().kind(), Impulse::Key);

    s.dev().send().unwrap();
    let f15 = KeyCode::KEY_F15.code();
    assert_eq!(
        collect(&s, 2, Duration::from_secs(2)),
        [(EventType::KEY, f15, 1), (EventType::KEY, f15, 0)]
    );
    assert!(collect(&s, 1, Duration::from_millis(300)).is_empty());

    // Every impulse is the same.
    s.dev().send().unwrap();
    assert_eq!(collect(&s, 2, Duration::from_secs(2)).len(), 2);
}

#[test]
fn the_mouse_impulse_moves_one_unit_and_back() {
    let Some(mut s) = setup(Impulse::Mouse) else {
        return;
    };
    assert!(s.keys.is_empty());
    assert_eq!(s.axes, [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]);
    assert_eq!(s.dev().kind(), Impulse::Mouse);

    s.dev().send().unwrap();
    let x = RelativeAxisCode::REL_X.0;
    let got = collect(&s, 2, Duration::from_secs(2));
    assert_eq!(
        got,
        [(EventType::RELATIVE, x, 1), (EventType::RELATIVE, x, -1)]
    );
    // Net movement zero.
    assert_eq!(got.iter().map(|e| e.2).sum::<i32>(), 0);
    assert!(collect(&s, 1, Duration::from_millis(300)).is_empty());
}

#[test]
fn the_first_impulse_waits_for_the_device_to_settle() {
    let Some(mut s) = setup(Impulse::Key) else {
        return;
    };
    let t = Instant::now();
    s.dev().send().unwrap();
    // Creating the device and finding its node took some of the 500 ms.
    assert!(
        t.elapsed() < tuxhub_always_online::impulse::SETTLE + Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
    // A second impulse does not wait again.
    let t = Instant::now();
    s.dev().send().unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn the_device_is_gone_when_it_is_dropped() {
    let Some(s) = setup(Impulse::Key) else {
        return;
    };
    let node = s.node.clone();
    assert!(node.exists());
    drop(s);
    let end = Instant::now() + Duration::from_secs(3);
    while node.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!node.exists(), "{} is still there", node.display());
}

/// The service drops a device right after a test impulse; the last event must
/// not be lost with it.
#[test]
fn the_last_event_survives_dropping_the_device_right_after_sending() {
    for (kind, expected) in [
        (
            Impulse::Key,
            [
                (EventType::KEY, KeyCode::KEY_F15.code(), 1),
                (EventType::KEY, KeyCode::KEY_F15.code(), 0),
            ],
        ),
        (
            Impulse::Mouse,
            [
                (EventType::RELATIVE, RelativeAxisCode::REL_X.0, 1),
                (EventType::RELATIVE, RelativeAxisCode::REL_X.0, -1),
            ],
        ),
    ] {
        let Some(mut s) = setup(kind) else { return };
        s.dev().send().unwrap();
        drop(s.device.take());
        assert_eq!(collect(&s, 2, Duration::from_secs(2)), expected, "{kind:?}");
    }
}
