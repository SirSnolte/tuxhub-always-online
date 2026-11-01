//! `install`, `uninstall` and the desktop integration, run as separate
//! processes with their own HOME and XDG directories, so nothing touches the
//! real session. Nothing here switches the tool on, so no device is created.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_tuxhub-always-online");
const APP_ID: &str = "io.github.sirsnolte.TuxhubAlwaysOnline";
const EXE_NAME: &str = "tuxhub-always-online";

struct Env {
    root: PathBuf,
    runtime: PathBuf,
}

impl Env {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let tag = format!("{}-{n}", std::process::id());
        let root = std::env::temp_dir().join(format!("tuxhub-tao-inst-{name}-{tag}"));
        let user_run = PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }));
        let runtime = if user_run.is_dir() {
            user_run.join(format!("taoi-{tag}"))
        } else {
            root.join("run")
        };
        for dir in [&root, &runtime] {
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(dir).unwrap();
        }
        Self { root, runtime }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    fn dest(&self) -> PathBuf {
        self.data().join(EXE_NAME)
    }
    fn registration(&self) -> PathBuf {
        self.data().join("tuxhub/apps/always-online.toml")
    }
    fn link(&self) -> PathBuf {
        self.home().join(".local/bin").join(EXE_NAME)
    }
    fn launcher(&self) -> PathBuf {
        self.data()
            .join("applications")
            .join(format!("{APP_ID}.desktop"))
    }
    fn icon(&self) -> PathBuf {
        self.data()
            .join("icons/hicolor/scalable/apps")
            .join(format!("{APP_ID}.svg"))
    }
    fn autostart(&self) -> PathBuf {
        self.config()
            .join("autostart")
            .join(format!("{APP_ID}.desktop"))
    }
    fn socket(&self) -> PathBuf {
        self.runtime.join("tuxhub/always-online.sock")
    }

    /// An extracted release folder: the binary and the files next to it.
    fn release(&self) -> PathBuf {
        let rel = self.root.join("release");
        std::fs::create_dir_all(rel.join("assets")).unwrap();
        std::fs::copy(EXE, rel.join(EXE_NAME)).unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR"));
        for file in ["tuxhub.toml", "assets/icon.svg"] {
            std::fs::copy(project.join(file), rel.join(file)).unwrap();
        }
        rel
    }

    fn run(&self, exe: &Path, args: &[&str]) -> Output {
        Command::new(exe)
            .args(args)
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.config())
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
        let stream = UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut w = &stream;
        writeln!(
            w,
            "{}",
            json!({ "id": 1, "method": method, "params": params })
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        match msg.get("error") {
            Some(e) => Err(e.clone()),
            None => Ok(msg["result"].clone()),
        }
    }

    fn wait_for_service(&self) {
        assert!(
            wait_until(Duration::from_secs(5), || UnixStream::connect(
                self.socket()
            )
            .is_ok()),
            "the service did not start"
        );
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // Ends a service that a test left running.
        if self.socket().exists() {
            if let Ok(stream) = UnixStream::connect(self.socket()) {
                let mut w = &stream;
                let _ = writeln!(w, r#"{{"id":1,"method":"quit"}}"#);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = BufReader::new(&stream).read_line(&mut String::new());
            }
            wait_until(Duration::from_secs(3), || !self.socket().exists());
        }
        self.stop_leftovers();
        let _ = std::fs::remove_dir_all(&self.runtime);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Env {
    /// Ends every process that runs from this test's folder, for example a
    /// service that was still starting when the test ended.
    fn stop_leftovers(&self) {
        let root = self.root.to_string_lossy().into_owned();
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return;
        };
        for entry in procs.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let first = cmdline.split(|b| *b == 0).next().unwrap_or_default();
            if String::from_utf8_lossy(first).starts_with(&root) {
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }
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

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn installs_from_a_release_folder() {
    let env = Env::new("install");
    let rel = env.release();
    let out = env.run(&rel.join(EXE_NAME), &["install"]);
    assert!(out.status.success(), "{}", text(&out));

    let dest = env.dest();
    for file in [EXE_NAME, "tuxhub.toml", "assets/icon.svg"] {
        assert!(dest.join(file).is_file(), "{file}");
    }
    // tuxhub finds the installation through this file.
    let registration = std::fs::read_to_string(env.registration()).unwrap();
    assert!(
        registration.contains("id = \"always-online\""),
        "{registration}"
    );
    assert!(
        registration.contains(&format!("path = {}", json!(dest.to_string_lossy()))),
        "{registration}"
    );
    assert!(registration.contains("repo = \"SirSnolte/tuxhub-always-online\""));
    assert!(registration.contains("asset = \"*x86_64-linux.tar.gz\""));
    assert_eq!(std::fs::read_link(env.link()).unwrap(), dest.join(EXE_NAME));
    let launcher = std::fs::read_to_string(env.launcher()).unwrap();
    assert!(launcher.contains(&format!("Exec=\"{}\"\n", dest.join(EXE_NAME).display())));
    assert!(launcher.contains(&format!("StartupWMClass={APP_ID}")));
    assert!(launcher.contains("Name=Always Online"));
    assert_eq!(
        std::fs::read(env.icon()).unwrap(),
        std::fs::read(dest.join("assets/icon.svg")).unwrap()
    );
    let autostart = std::fs::read_to_string(env.autostart()).unwrap();
    assert!(
        autostart.contains(&format!(
            "Exec=\"{}\" daemon\n",
            dest.join(EXE_NAME).display()
        )),
        "{autostart}"
    );
    assert!(autostart.contains("NoDisplay=true"));

    // The service from the installed copy is running, and it is off.
    env.wait_for_service();
    assert_eq!(
        env.call("status", json!({})).unwrap()["state"],
        json!("off")
    );

    // A second installation replaces the first and leaves no staging folders.
    let again = env.run(&rel.join(EXE_NAME), &["install"]);
    assert!(again.status.success(), "{}", text(&again));
    assert_eq!(
        names(&env.data()),
        ["applications", "icons", "tuxhub", "tuxhub-always-online"]
    );
    env.wait_for_service();
}

#[test]
fn install_needs_a_release_folder() {
    let env = Env::new("nofolder");
    // The development binary has no tuxhub.toml beside it.
    let out = env.run(Path::new(EXE), &["install"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("tuxhub.toml"), "{}", text(&out));
    assert!(!env.dest().exists());
    assert!(!env.launcher().exists());
}

#[test]
fn install_refuses_when_tuxhub_manages_the_tool_unless_forced() {
    let env = Env::new("hub");
    let record = env
        .data()
        .join("io.github.sirsnolte.tuxhub/tools/always-online/tool.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "{}").unwrap();
    let rel = env.release();

    let out = env.run(&rel.join(EXE_NAME), &["install"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("already installed through tuxhub"),
        "{}",
        text(&out)
    );
    assert!(!env.dest().exists());
    assert!(!env.registration().exists());

    let forced = env.run(&rel.join(EXE_NAME), &["install", "--force"]);
    assert!(forced.status.success(), "{}", text(&forced));
    assert!(env.dest().join(EXE_NAME).is_file());
}

#[test]
fn the_service_keeps_the_login_entry_in_line_with_the_setting() {
    let env = Env::new("login");
    let rel = env.release();
    assert!(env.run(&rel.join(EXE_NAME), &["install"]).status.success());
    env.wait_for_service();
    assert!(env.autostart().is_file());

    env.call("config.set", json!({ "start_at_login": false }))
        .unwrap();
    assert!(!env.autostart().exists());
    assert!(env.launcher().is_file(), "the launcher stays");
    env.call("config.set", json!({ "start_at_login": true }))
        .unwrap();
    assert!(env.autostart().is_file());
}

#[test]
fn a_development_build_writes_no_integration() {
    let env = Env::new("dev");
    let mut daemon = Command::new(EXE)
        .arg("daemon")
        .env("HOME", env.home())
        .env("XDG_CONFIG_HOME", env.config())
        .env("XDG_DATA_HOME", env.data())
        .env("XDG_RUNTIME_DIR", &env.runtime)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    env.wait_for_service();
    env.call("config.set", json!({ "start_at_login": false }))
        .unwrap();
    env.call("config.set", json!({ "start_at_login": true }))
        .unwrap();
    env.call("quit", json!({})).unwrap();
    let _ = daemon.wait();
    assert!(!env.data().join("applications").exists());
    assert!(!env.config().join("autostart").exists());
    assert!(!env.data().join("tuxhub").exists());
}

#[test]
fn uninstall_removes_what_install_made() {
    let env = Env::new("uninstall");
    let rel = env.release();
    assert!(env.run(&rel.join(EXE_NAME), &["install"]).status.success());
    env.wait_for_service();
    env.call("config.set", json!({ "interval_s": 90 })).unwrap();
    let settings = env.config().join("tuxhub-always-online");
    assert!(settings.join("config.json").is_file());

    // Run from the installed copy, as tuxhub does.
    let out = env.run(&env.dest().join(EXE_NAME), &["uninstall"]);
    assert!(out.status.success(), "{}", text(&out));
    for path in [
        env.autostart(),
        env.launcher(),
        env.icon(),
        env.registration(),
        env.link(),
        env.dest(),
    ] {
        assert!(
            path.symlink_metadata().is_err(),
            "{} is still there",
            path.display()
        );
    }
    assert!(!env.socket().exists(), "the service was stopped");
    assert!(settings.join("config.json").is_file(), "settings are kept");

    // Nothing left: a second uninstall is fine, and --purge takes the settings.
    let purge = env.run(&rel.join(EXE_NAME), &["uninstall", "--purge"]);
    assert!(purge.status.success(), "{}", text(&purge));
    assert!(!settings.exists());
    assert!(
        rel.join(EXE_NAME).is_file(),
        "the release folder is untouched"
    );
}

#[test]
fn uninstall_keeps_registrations_and_links_of_other_installations() {
    let env = Env::new("foreign");
    let rel = env.release();
    std::fs::create_dir_all(env.registration().parent().unwrap()).unwrap();
    let foreign = "id = \"always-online\"\npath = \"/opt/somewhere/else\"\n";
    std::fs::write(env.registration(), foreign).unwrap();
    std::fs::create_dir_all(env.link().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/opt/somewhere/else/tuxhub-always-online", env.link()).unwrap();

    let out = env.run(&rel.join(EXE_NAME), &["uninstall"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(env.registration()).unwrap(),
        foreign
    );
    assert_eq!(
        std::fs::read_link(env.link()).unwrap(),
        PathBuf::from("/opt/somewhere/else/tuxhub-always-online")
    );
    assert!(rel.join(EXE_NAME).is_file());
}

/// After an update replaced the executable, the service starts the new one.
#[test]
fn the_service_restarts_after_an_update() {
    let env = Env::new("update");
    let rel = env.release();
    assert!(env.run(&rel.join(EXE_NAME), &["install"]).status.success());
    env.wait_for_service();
    let before = std::fs::metadata(env.socket()).unwrap().ino();

    // An update puts a new file in place of the old one.
    let next = env.dest().join("next");
    std::fs::copy(EXE, &next).unwrap();
    std::fs::rename(&next, env.dest().join(EXE_NAME)).unwrap();

    // The service looks at the executable whenever it wakes up; any request does.
    let _ = env.call("status", json!({}));
    assert!(
        wait_until(Duration::from_secs(10), || {
            std::fs::metadata(env.socket()).is_ok_and(|m| m.ino() != before)
                && UnixStream::connect(env.socket()).is_ok()
        }),
        "the service did not restart"
    );
    assert_eq!(
        env.call("status", json!({})).unwrap()["state"],
        json!("off")
    );
    // The settings are the same after the restart.
    assert_eq!(
        env.call("config.get", json!({})).unwrap()["interval_s"],
        json!(240)
    );
    assert!(!names(&env.dest()).contains(&"next".to_string()));
}
