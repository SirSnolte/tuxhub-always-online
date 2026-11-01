use clap::{CommandFactory, Parser, Subcommand};
use serde_json::{Value, json};
use std::io;
use tuxhub_always_online::{client, daemon, impulse};

/// Keeps Discord, Steam and other apps from setting you to away while you are
/// at the computer, by sending a harmless input event at a fixed interval.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the background service in the foreground
    Daemon,
    /// Show the current state
    Status,
    /// Start keeping you online
    On,
    /// Stop keeping you online
    Off,
    /// Send one impulse now, whatever the switch and the schedule say
    Test,
    /// Grant access to /dev/uinput once (asks for the administrator password)
    Setup,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        None => Cli::command().print_help(),
        Some(cmd) => run(cmd),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run(cmd: Command) -> io::Result<()> {
    match cmd {
        Command::Daemon => {
            if !daemon::run()? {
                eprintln!("The service is already running.");
            }
            Ok(())
        }
        Command::Status => print_status(),
        Command::On => set(true),
        Command::Off => set(false),
        Command::Test => {
            client::request("impulse.test", Value::Null)?;
            println!("Sent one impulse.");
            Ok(())
        }
        Command::Setup => {
            impulse::setup_access()?;
            println!("Access to /dev/uinput is set up.");
            Ok(())
        }
    }
}

fn set(enabled: bool) -> io::Result<()> {
    client::request("config.set", json!({ "enabled": enabled }))?;
    print_status()
}

fn print_status() -> io::Result<()> {
    let s = client::request("status", Value::Null)?;
    let cfg = client::request("config.get", Value::Null)?;
    let state = s["state"].as_str().unwrap_or("unknown");
    println!("State:     {}", describe(state));
    if let Some(n) = s["next_in_s"].as_u64() {
        println!("Next:      in {}", duration(n));
    }
    if let Some(n) = s["run_elapsed_s"].as_u64() {
        println!("Running:   {}", duration(n));
    }
    println!(
        "Interval:  {}",
        duration(cfg["interval_s"].as_u64().unwrap_or(0))
    );
    println!("Impulse:   {}", cfg["impulse"].as_str().unwrap_or_default());
    match cfg["max_hours"].as_u64() {
        Some(0) | None => println!("Longest:   no limit"),
        Some(h) => println!("Longest:   {h} h"),
    }
    if cfg["schedule_enabled"] == json!(true) {
        match s["schedule_change_in_min"].as_u64() {
            Some(m) if s["schedule_active"] == json!(true) => {
                println!("Schedule:  inside a window, ends in {}", duration(m * 60));
            }
            Some(m) => println!("Schedule:  outside, next window in {}", duration(m * 60)),
            None => println!("Schedule:  never changes"),
        }
    } else {
        println!("Schedule:  off, always");
    }
    println!("Sent:      {}", s["sent"]);
    if let Some(e) = s["last_error"].as_str() {
        println!("Problem:   {e}");
    }
    Ok(())
}

fn describe(state: &str) -> &str {
    match state {
        "off" => "off",
        "outside" => "on, outside the schedule",
        "paused" => "paused, the longest run is over",
        "sending" => "on, sending impulses",
        "no_access" => "on, but no impulse can be sent",
        other => other,
    }
}

/// `4 min 20 s` style text for a number of seconds.
fn duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s} s"),
        (0, m, 0) => format!("{m} min"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, 0, _) => format!("{h} h"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(duration(0), "0 s");
        assert_eq!(duration(45), "45 s");
        assert_eq!(duration(240), "4 min");
        assert_eq!(duration(260), "4 min 20 s");
        assert_eq!(duration(3600), "1 h");
        assert_eq!(duration(3600 + 125), "1 h 2 min");
        assert_eq!(duration(5 * 3600 + 30 * 60), "5 h 30 min");
    }

    #[test]
    fn describes_every_state() {
        for s in ["off", "outside", "paused", "sending", "no_access"] {
            assert!(!describe(s).is_empty());
        }
        assert_eq!(describe("something else"), "something else");
    }
}
