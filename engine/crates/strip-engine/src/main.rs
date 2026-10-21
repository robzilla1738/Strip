use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clap::Parser;

use strip_engine::ax;
use strip_engine::config::Config;
use strip_engine::lifecycle;
use strip_engine::private_api::PrivateApi;
use strip_engine::serve;
use strip_engine::session::Runtime;

#[derive(Parser)]
#[command(name = "strip-engine", version, about = "Scrolling window engine")]
struct Cli {
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    state: Option<PathBuf>,
    #[arg(long)]
    log: Option<PathBuf>,
    /// Parse a config file and exit 0 when it matches the schema.
    #[arg(long)]
    validate_config: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(path) = cli.validate_config.as_ref() {
        return match Config::load(path) {
            Ok(_) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::from(1)
            }
        };
    }

    let private_api = PrivateApi::live();
    eprintln!(
        "private_api skylight={} ax_get_window={} slps={}",
        private_api.skylight, private_api.ax_get_window, private_api.slps
    );
    let config = match cli.config.as_ref() {
        Some(path) => match Config::load(path) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        },
        None => Config::default(),
    };
    let snapshot = Arc::new(Mutex::new("{}".to_string()));
    let trusted = ax::trusted();
    if let Some(path) = cli.socket.as_ref() {
        if let Err(err) = serve::listen(path, Arc::clone(&snapshot), private_api.clone(), trusted) {
            eprintln!("socket: {err}");
            return ExitCode::from(1);
        }
    }
    let _wakeup = lifecycle::install(Arc::new(|| {
        lifecycle::request_stop();
    }));
    let mut runtime = Runtime::start(config, &private_api, Arc::clone(&snapshot));
    loop {
        if lifecycle::stop_requested() {
            runtime.unpark_all();
            if let Some(path) = cli.state.as_ref() {
                if let Ok(text) = snapshot.lock() {
                    let _ = std::fs::write(path, &*text);
                }
            }
            break;
        }
        runtime.tick();
        if let Some(path) = cli.state.as_ref() {
            if let Ok(text) = snapshot.lock() {
                let _ = std::fs::write(path, &*text);
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
    let _ = cli.log;
    ExitCode::SUCCESS
}
