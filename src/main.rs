use clap::{Parser, Subcommand};
use runwaybar::cli::{self, Format, StatusOpts};

#[derive(Parser)]
#[command(
    name = "runwaybar",
    version = runwaybar::VERSION,
    about = "Your AI runway, at a glance — usage bar for Claude Code, Codex, z.ai/ZCode and OpenCode",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show current usage snapshot (default query surface)
    Status {
        /// Output format
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
        /// Restrict to specific provider ids (repeatable)
        #[arg(long = "provider")]
        providers: Vec<String>,
        /// Never touch the network; serve cache/empty state
        #[arg(long)]
        no_fetch: bool,
    },
    /// Force a refresh now (daemon if running, else one-shot)
    Refresh,
    /// One-off live smoke test against real credentials (gated by RUNWAYBAR_LIVE_SMOKE=1)
    Smoke,
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Status {
            format,
            providers,
            no_fetch,
        } => {
            match cli::run_status(StatusOpts {
                format,
                providers,
                no_fetch,
            }) {
                Ok(snap) => {
                    let out = match format {
                        Format::Json => cli::render_json(&snap),
                        Format::Text => cli::render_text(&snap),
                        Format::Waybar => cli::render_waybar(&snap),
                    };
                    println!("{out}");
                    0
                }
                Err(e) => {
                    eprintln!("runwaybar: {e}");
                    1
                }
            }
        }
        Command::Refresh => {
            if runwaybar::ipc::request_refresh() {
                println!("refresh requested");
                0
            } else {
                match cli::run_status(StatusOpts {
                    format: Format::Json,
                    providers: vec![],
                    no_fetch: false,
                }) {
                    Ok(_) => {
                        println!("refreshed (one-shot)");
                        0
                    }
                    Err(e) => {
                        eprintln!("runwaybar: {e}");
                        1
                    }
                }
            }
        }
        Command::Smoke => smoke(),
    };
    std::process::exit(code);
}

/// Live smoke: polls discoverable providers once, reports latency, gates on 5 s
/// time-to-first-data (invariant #20). Requires explicit opt-in via env.
fn smoke() -> i32 {
    if std::env::var("RUNWAYBAR_LIVE_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "runwaybar: --smoke touches your real accounts; set RUNWAYBAR_LIVE_SMOKE=1 to allow it"
        );
        return 2;
    }
    let config = std::sync::Arc::new(runwaybar::config::Config::load().unwrap_or_default());
    let start = std::time::Instant::now();
    let snap = runwaybar::providers::poll_all(config);
    let elapsed = start.elapsed();
    println!("{}", cli::render_text(&snap));
    println!(
        "time-to-first-data (all discoverable providers): {:.2?}",
        elapsed
    );
    let discoverable = snap
        .providers
        .iter()
        .filter(|p| !matches!(p.status, runwaybar::model::Status::NotInstalled))
        .count();
    println!("discoverable providers: {discoverable}");
    if discoverable > 0 && elapsed > std::time::Duration::from_secs(5) {
        eprintln!("runwaybar: smoke FAILED the 5s time-to-first-data budget");
        1
    } else {
        0
    }
}
