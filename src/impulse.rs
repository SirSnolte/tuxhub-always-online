//! The impulse: one harmless input event from a virtual device in
//! `/dev/uinput`. The kernel treats the device like a physical keyboard or
//! mouse, so it resets the idle time of the session on Wayland and X11 alike.
//!
//! The device only has the capabilities of the chosen impulse, and the service
//! creates it when it starts sending and drops it afterwards.

use crate::config::Impulse;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEVICE_NAME: &str = "tuxhub-always-online";

/// Time the compositor needs to notice a new device. Events sent earlier
/// could get lost.
pub const SETTLE: Duration = Duration::from_millis(500);
/// Time between the two events of an impulse.
const KEY_HOLD: Duration = Duration::from_millis(20);
const MOUSE_BACK: Duration = Duration::from_millis(50);

/// The key of the impulse: no key on common keyboards and no binding in
/// common desktops or games.
const KEY: KeyCode = KeyCode::KEY_F15;

pub struct Device {
    dev: VirtualDevice,
    kind: Impulse,
    created: Instant,
}

impl Device {
    pub fn new(kind: Impulse) -> io::Result<Self> {
        let builder = VirtualDevice::builder().map_err(explain)?.name(DEVICE_NAME);
        let builder = match kind {
            Impulse::Key => builder.with_keys(&AttributeSet::from_iter([KEY]))?,
            // Both axes, or libinput does not treat the device as a pointer.
            Impulse::Mouse => builder.with_relative_axes(&AttributeSet::from_iter([
                RelativeAxisCode::REL_X,
                RelativeAxisCode::REL_Y,
            ]))?,
        };
        Ok(Self {
            dev: builder.build().map_err(explain)?,
            kind,
            created: Instant::now(),
        })
    }

    pub fn kind(&self) -> Impulse {
        self.kind
    }

    /// Paths of the `/dev/input/event*` nodes of this device.
    pub fn dev_nodes(&mut self) -> io::Result<Vec<PathBuf>> {
        self.dev.enumerate_dev_nodes_blocking()?.collect()
    }

    /// Sends one impulse. Waits first if the device is younger than
    /// [`SETTLE`], and takes about 20 to 50 ms.
    pub fn send(&mut self) -> io::Result<()> {
        let wait = SETTLE.saturating_sub(self.created.elapsed());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        match self.kind {
            Impulse::Key => {
                self.emit(EventType::KEY, KEY.code(), 1)?;
                std::thread::sleep(KEY_HOLD);
                self.emit(EventType::KEY, KEY.code(), 0)
            }
            Impulse::Mouse => {
                self.emit(EventType::RELATIVE, RelativeAxisCode::REL_X.0, 1)?;
                std::thread::sleep(MOUSE_BACK);
                self.emit(EventType::RELATIVE, RelativeAxisCode::REL_X.0, -1)
            }
        }
    }

    /// Emits one event; the library follows it with a sync event.
    fn emit(&mut self, kind: EventType, code: u16, value: i32) -> io::Result<()> {
        self.dev.emit(&[InputEvent::new(kind.0, code, value)])
    }
}

fn explain(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::PermissionDenied || e.kind() == io::ErrorKind::NotFound {
        io::Error::new(
            e.kind(),
            format!(
                "no access to /dev/uinput ({e}). Grant access once with \
                 `tuxhub-always-online setup` or the \"Set up access\" button."
            ),
        )
    } else {
        e
    }
}

/// udev rule that gives the logged-in user access to `/dev/uinput`. It is the
/// same file and text as in the other tuxhub tools, so one setup serves all.
pub const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/60-tuxhub-uinput.rules";
pub const UDEV_RULE: &str =
    "KERNEL==\"uinput\", SUBSYSTEM==\"misc\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\"\n";

/// Installs the udev rule with `pkexec`, which asks for the password in a
/// graphical dialog. The rule is passed on standard input, not interpolated
/// into the shell command.
pub fn setup_access() -> io::Result<()> {
    let script = format!(
        "cat > {UDEV_RULE_PATH} && modprobe uinput && \
         udevadm control --reload-rules && udevadm trigger --name-match=uinput"
    );
    let mut child = Command::new("pkexec")
        .args(["sh", "-c", &script])
        .stdin(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(UDEV_RULE.as_bytes())?;
    if child.wait()?.success() {
        Ok(())
    } else {
        Err(io::Error::other("the setup was cancelled or failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_missing_access_and_keeps_other_errors() {
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound] {
            let e = explain(io::Error::new(kind, "boom"));
            assert_eq!(e.kind(), kind);
            let text = e.to_string();
            assert!(text.contains("/dev/uinput"), "{text}");
            assert!(text.contains("tuxhub-always-online setup"), "{text}");
            assert!(text.contains("boom"), "{text}");
        }
        let other = explain(io::Error::new(io::ErrorKind::TimedOut, "slow"));
        assert_eq!(other.to_string(), "slow");
    }

    #[test]
    fn the_udev_rule_is_the_one_of_the_other_tools() {
        assert_eq!(UDEV_RULE_PATH, "/etc/udev/rules.d/60-tuxhub-uinput.rules");
        assert!(UDEV_RULE.contains("TAG+=\"uaccess\""));
        assert!(UDEV_RULE.ends_with('\n'));
        assert_eq!(UDEV_RULE.lines().count(), 1);
    }

    #[test]
    fn the_impulse_key_is_f15() {
        assert_eq!(format!("{KEY:?}"), "KEY_F15");
    }
}
