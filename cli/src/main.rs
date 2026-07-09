//! `scribe` - a thin CLI over Scribe's dictation-state interface.
//!
//! `scribe status`  - point-in-time query of the current snapshot.
//! `scribe watch`   - stream state changes (SSE), reconnecting on drop.
//!
//! See the crate docs and README for the wire contract this speaks.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};

use scribe_cli::client::{self, WatchItem, WatchOptions};
use scribe_cli::config::{self, Offline, Resolution, ResolveOptions};
use scribe_cli::discovery::Channel;
use scribe_cli::output;
use scribe_cli::snapshot::Snapshot;

// Exit codes. Chosen so scripts can gate cheaply.
//   0  ok / reachable
//   1  error (bad token, protocol/transport failure)
//   2  usage error (clap default)
//   3  offline - Scribe not running / unreachable (status prints not-dictating)
const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_OFFLINE: u8 = 3;

#[derive(Parser)]
#[command(
    name = "scribe",
    version,
    about = "Thin client for Scribe's dictation-state interface (read-only).",
    long_about = "Reads ~/.scribe/control.json to discover the running Scribe server, then \
queries or streams its dictation state over the frozen HTTP+SSE wire contract (v1).\n\n\
Stale-or-dead always resolves to not-dictating: a missing control file, a dead \
process, or a refused/closed connection are reported as not-dictating rather than \
hanging."
)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,

    #[command(subcommand)]
    command: Command,
}

/// Options shared by every subcommand (all usable before or after it).
#[derive(Args, Clone)]
struct CommonArgs {
    /// Target the Dev flavor's control.dev.json instead of control.json.
    /// (Env: SCRIBE_CHANNEL=dev)
    #[arg(long, global = true)]
    dev: bool,

    /// Explicit path to the control file, overriding discovery.
    /// (Env: SCRIBE_CONTROL)
    #[arg(long, value_name = "PATH", global = true)]
    control: Option<PathBuf>,

    /// Talk to this base URL directly, bypassing the control file entirely.
    /// (Env: SCRIBE_BASE_URL)
    #[arg(long, value_name = "URL", global = true)]
    base_url: Option<String>,

    /// Read token to send as `Authorization: Bearer`. Only needed with
    /// --base-url; discovery supplies it otherwise. (Env: SCRIBE_TOKEN)
    #[arg(long, value_name = "TOKEN", global = true)]
    token: Option<String>,

    /// Skip the pid liveness check on the discovered process.
    #[arg(long, global = true)]
    no_pid_check: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Point-in-time query of the current dictation state.
    Status(StatusArgs),
    /// Stream dictation state changes, reconnecting on drop.
    Watch(WatchArgs),
}

#[derive(Args)]
struct StatusArgs {
    /// Emit the raw snapshot JSON instead of the human summary.
    #[arg(long)]
    json: bool,

    /// Print nothing; exit code alone reflects the gate field (see --field).
    /// Exit 0 if the field is true, 1 otherwise (offline/unknown counts as
    /// false - the safe "you may act" failure).
    #[arg(long, short)]
    quiet: bool,

    /// Which boolean --quiet gates on.
    #[arg(long, value_enum, default_value_t = GateField::Busy)]
    field: GateField,
}

#[derive(Copy, Clone, ValueEnum)]
enum GateField {
    /// Broad flag: user is inside a dictation cycle (default).
    Busy,
    /// Narrow flag: the microphone is actively capturing.
    Dictating,
}

#[derive(Args)]
struct WatchArgs {
    /// Human-readable lines instead of newline-delimited JSON.
    #[arg(long)]
    human: bool,

    /// Exit after the first stream end instead of reconnecting forever.
    #[arg(long)]
    no_reconnect: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let opts = resolve_options(&cli.common);

    match &cli.command {
        Command::Status(args) => run_status(&opts, args),
        Command::Watch(args) => run_watch(&opts, args),
    }
}

/// Merge CLI flags with environment fallbacks into resolve options.
fn resolve_options(common: &CommonArgs) -> ResolveOptions {
    let dev = common.dev
        || std::env::var("SCRIBE_CHANNEL")
            .map(|v| v.eq_ignore_ascii_case("dev"))
            .unwrap_or(false);

    let control = common
        .control
        .clone()
        .or_else(|| std::env::var_os("SCRIBE_CONTROL").map(PathBuf::from));

    let base_url = common
        .base_url
        .clone()
        .or_else(|| std::env::var("SCRIBE_BASE_URL").ok());

    let token = common
        .token
        .clone()
        .or_else(|| std::env::var("SCRIBE_TOKEN").ok());

    ResolveOptions {
        base_url,
        token,
        control_path: control,
        channel: if dev { Channel::Dev } else { Channel::Stable },
        pid_check: !common.no_pid_check,
    }
}

fn run_status(opts: &ResolveOptions, args: &StatusArgs) -> ExitCode {
    // Resolve discovery. Offline -> synthesize a not-dictating snapshot.
    let target = match config::resolve(opts) {
        Resolution::Online(t) => t,
        Resolution::Offline(reason) => {
            return report_offline_status(&reason, args);
        }
    };

    match client::fetch_status(&target) {
        Ok(fetched) => {
            maybe_warn_future_version(&fetched.snapshot);
            if args.quiet {
                return gate_exit(&fetched.snapshot, args.field);
            }
            if args.json {
                println!("{}", output::json_line(&fetched.raw));
            } else {
                print!("{}", output::human_status(&fetched.snapshot));
            }
            ExitCode::from(EXIT_OK)
        }
        Err(client::ClientError::Offline(reason)) => {
            report_offline_status(&Offline::UnreadableControlFile(reason), args)
        }
        Err(client::ClientError::Unauthorized) => {
            // Cannot read state -> fail toward not-dictating, but signal error.
            eprintln!("scribe: unauthorized - the read token was rejected (401)");
            if args.quiet {
                return ExitCode::from(EXIT_ERROR);
            }
            let snap = Snapshot::offline();
            emit_offline_body(&snap, args);
            ExitCode::from(EXIT_ERROR)
        }
        Err(e) => {
            eprintln!("scribe: {e}");
            if args.quiet {
                return ExitCode::from(EXIT_ERROR);
            }
            let snap = Snapshot::offline();
            emit_offline_body(&snap, args);
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Print a not-dictating snapshot for an offline Scribe and return EXIT_OFFLINE.
fn report_offline_status(reason: &Offline, args: &StatusArgs) -> ExitCode {
    if args.quiet {
        // Offline -> field is false -> "you may act".
        return ExitCode::from(EXIT_ERROR);
    }
    let snap = Snapshot::offline();
    if args.json {
        println!(
            "{}",
            output::json_line(&serde_json::to_value(&snap).unwrap())
        );
    } else {
        print!("{}", output::human_status(&snap));
        eprintln!("scribe: {}", reason.reason());
    }
    ExitCode::from(EXIT_OFFLINE)
}

/// Emit just the body (json or human) for an offline/error snapshot.
fn emit_offline_body(snap: &Snapshot, args: &StatusArgs) {
    if args.json {
        println!(
            "{}",
            output::json_line(&serde_json::to_value(snap).unwrap())
        );
    } else {
        print!("{}", output::human_status(snap));
    }
}

/// Quiet-mode exit: 0 if the gated field is true, else 1.
fn gate_exit(snap: &Snapshot, field: GateField) -> ExitCode {
    let on = match field {
        GateField::Busy => snap.busy,
        GateField::Dictating => snap.dictating,
    };
    ExitCode::from(if on { EXIT_OK } else { EXIT_ERROR })
}

fn maybe_warn_future_version(snap: &Snapshot) {
    if snap.is_future_version() {
        eprintln!(
            "scribe: warning - snapshot schemaVersion {} is newer than supported ({}); \
trusting the booleans only",
            snap.schema_version,
            scribe_cli::snapshot::SUPPORTED_SCHEMA_VERSION
        );
    }
}

fn run_watch(opts: &ResolveOptions, args: &WatchArgs) -> ExitCode {
    // The stop flag is part of the library's testable watch API. In the binary
    // we let the OS handle Ctrl-C (default SIGINT termination), so it is only
    // ever read here, never set - `watch` streams until the process is killed.
    let stop = Arc::new(AtomicBool::new(false));

    let watch_opts = WatchOptions {
        reconnect: !args.no_reconnect,
        ..Default::default()
    };

    // Resolve first. If offline, emit one not-dictating line up front; then,
    // unless --no-reconnect, keep trying to discover a live server.
    let human = args.human;
    let mut stdout = std::io::stdout();

    loop {
        match config::resolve(opts) {
            Resolution::Online(target) => {
                client::watch(&target, &watch_opts, stop.clone(), |item| {
                    emit_watch_item(&mut stdout, &item, human);
                    true
                });
                // watch returns on stop or (with --no-reconnect) first end.
                if stop.load(Ordering::Relaxed) || args.no_reconnect {
                    return ExitCode::from(EXIT_OK);
                }
                // reconnect==true means watch only returns via stop; fallthrough
                // here is defensive.
            }
            Resolution::Offline(reason) => {
                let snap = Snapshot::offline();
                let raw = serde_json::to_value(&snap).unwrap();
                emit_watch_item(
                    &mut stdout,
                    &WatchItem::Offline {
                        snapshot: snap,
                        raw,
                        reason: reason.reason(),
                    },
                    human,
                );
                if args.no_reconnect {
                    return ExitCode::from(EXIT_OFFLINE);
                }
                // Wait before re-checking discovery.
                if !sleep_until_stop(std::time::Duration::from_secs(2), &stop) {
                    return ExitCode::from(EXIT_OK);
                }
            }
        }
    }
}

fn emit_watch_item(out: &mut std::io::Stdout, item: &WatchItem, human: bool) {
    let line = match item {
        WatchItem::Event {
            event,
            snapshot,
            raw,
        } => {
            if human {
                output::human_watch_line(event, snapshot)
            } else {
                output::json_line(raw)
            }
        }
        WatchItem::Offline {
            snapshot,
            raw,
            reason,
        } => {
            if human {
                format!(
                    "{}   ({reason})",
                    output::human_watch_line("offline", snapshot)
                )
            } else {
                output::json_line(raw)
            }
        }
    };
    // Ignore broken-pipe: a downstream `head` closing the pipe is normal.
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn sleep_until_stop(dur: std::time::Duration, stop: &Arc<AtomicBool>) -> bool {
    let step = std::time::Duration::from_millis(100);
    let mut remaining = dur;
    while remaining > std::time::Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let s = step.min(remaining);
        std::thread::sleep(s);
        remaining = remaining.saturating_sub(s);
    }
    !stop.load(Ordering::Relaxed)
}
