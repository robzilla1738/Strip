use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use strip_ipc::{encode, ClientMessage, ServerMessage};

use strip_engine::ax;
use strip_engine::bus::Bus;
use strip_engine::command;
use strip_engine::config::Config;
use strip_engine::hotkeys;
use strip_engine::lifecycle;
use strip_engine::private_api::PrivateApi;
use strip_engine::serve::{self, Serve};
use strip_engine::session::Runtime;
use strip_engine::wake;
use strip_engine::watch;

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

fn apply_default_paths(cli: &mut Cli) {
    let name = match std::env::var("STRIP_APP_NAME") {
        Ok(name) if !name.is_empty() && !name.contains('{') => name,
        Ok(name) if name.contains('{') => {
            eprintln!("MISSING: {{{{APP_NAME}}}}");
            return;
        }
        _ => return,
    };
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let support = PathBuf::from(&home).join("Library").join("Application Support").join(&name);
    let logs = PathBuf::from(home).join("Library").join("Logs").join(name);
    if cli.socket.is_none() {
        cli.socket = Some(support.join("engine.sock"));
    }
    if cli.config.is_none() {
        cli.config = Some(support.join("config.toml"));
    }
    if cli.state.is_none() {
        cli.state = Some(support.join("state.json"));
    }
    if cli.log.is_none() {
        cli.log = Some(logs.join("engine.log"));
    }
}

fn ensure_parent(path: &Option<PathBuf>) {
    if let Some(parent) = path.as_ref().and_then(|item| item.parent()) {
        let _ = std::fs::create_dir_all(parent);
    }
}

fn main() -> ExitCode {
    let mut cli = Cli::parse();
    if let Some(path) = cli.validate_config.as_ref() {
        return match Config::load(path) {
            Ok(_) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::from(1)
            }
        };
    }
    apply_default_paths(&mut cli);
    ensure_parent(&cli.socket);
    ensure_parent(&cli.config);
    ensure_parent(&cli.state);
    ensure_parent(&cli.log);

    let private_api = PrivateApi::live();
    eprintln!(
        "private_api skylight={} ax_get_window={} slps={}",
        private_api.skylight, private_api.ax_get_window, private_api.slps
    );
    let config = match cli.config.as_ref() {
        Some(path) if path.is_file() => match Config::load(path) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        },
        _ => Config::default(),
    };
    let snapshot = Arc::new(Mutex::new("{}".to_string()));
    let metrics = Arc::new(Mutex::new("{}".to_string()));
    let trusted = ax::trusted();
    let bus = Arc::new(Bus::new());
    if let Some(path) = cli.socket.as_ref() {
        let serve = Serve {
            state: Arc::clone(&snapshot),
            metrics: Arc::clone(&metrics),
            private_api: private_api.clone(),
            ax_trusted: trusted,
            bus: Arc::clone(&bus),
        };
        if let Err(err) = serve::listen(path, serve) {
            eprintln!("socket: {err}");
            return ExitCode::from(1);
        }
    }
    let _wakeup = lifecycle::install(Arc::new(|| {
        lifecycle::request_stop();
    }));
    let mut runtime = Runtime::start(
        config,
        cli.config.clone(),
        cli.state.clone(),
        &private_api,
        Arc::clone(&snapshot),
        Arc::clone(&metrics),
        trusted,
    );
    wake::install();
    let tap = hotkeys::install();
    hotkeys::remember_main_loop();
    watch::install_reload_source();
    let _config_watch = match cli.config.as_ref() {
        Some(path) => match watch::watch_config(path) {
            Ok(watch) => Some(watch),
            Err(err) => {
                eprintln!("config watch: {err}");
                None
            }
        },
        None => None,
    };
    runtime.tick();
    write_state(&cli, &snapshot, &bus);
    let mut next_tick = Instant::now() + Duration::from_millis(80);
    loop {
        if lifecycle::stop_requested() {
            runtime.unpark_all();
            write_state(&cli, &snapshot, &bus);
            break;
        }
        if wake::take_request() {
            runtime.resync_after_wake();
        }
        if watch::take_dirty() {
            apply_config(&mut runtime, &cli, &snapshot, &bus);
        }
        // The tap callback runs on this thread while the run loop is pumping.
        // Idle waits are long; IPC, the tap, and the display link wake this thread.
        let slice = if runtime.is_animating() { 0.02 } else { 1.0 };
        hotkeys::pump(&mut runtime, &bus, slice);
        runtime.drain_frames();
        let _ = tap.as_ref().map(hotkeys::EventTap::alive);
        for job in bus.poll(Duration::from_millis(0)) {
            let response = dispatch(&mut runtime, &bus, job.message);
            let _ = job.reply.send(response);
        }
        runtime.drain_frames();
        if watch::take_dirty() {
            apply_config(&mut runtime, &cli, &snapshot, &bus);
        }
        if lifecycle::stop_requested() {
            runtime.unpark_all();
            write_state(&cli, &snapshot, &bus);
            break;
        }
        if Instant::now() >= next_tick {
            runtime.tick();
            write_state(&cli, &snapshot, &bus);
            let period = if runtime.is_animating() { 80 } else { 1000 };
            next_tick = Instant::now() + Duration::from_millis(period);
        }
    }
    let _ = cli.log;
    ExitCode::SUCCESS
}

fn dispatch(runtime: &mut Runtime, bus: &Bus, message: ClientMessage) -> String {
    match message {
        ClientMessage::Command { id, action, args } => match command::run(runtime, bus, &action, &args) {
            Ok(()) => reply(&ServerMessage::ok(id)),
            Err(err) => reply(&ServerMessage::error(id, err.code(), err.message())),
        },
        ClientMessage::SetManaging { id, enabled } => {
            runtime.set_managing(enabled);
            reply(&ServerMessage::ok(id))
        }
        ClientMessage::ReloadConfig { id } => match runtime.reload() {
            Ok(()) => reply(&ServerMessage::ok(id)),
            Err(message) => reply(&ServerMessage::error(id, "bad_request", &message)),
        },
        ClientMessage::Shutdown { id } => {
            lifecycle::request_stop();
            reply(&ServerMessage::ok(id))
        }
        ClientMessage::Hello { id, .. } | ClientMessage::Query { id, .. } | ClientMessage::Subscribe { id, .. } => {
            reply(&ServerMessage::error(id, "internal", "handled on the connection"))
        }
    }
}

fn reply(message: &ServerMessage) -> String {
    encode(message).trim_end().to_string()
}

fn apply_config(runtime: &mut Runtime, cli: &Cli, snapshot: &Mutex<String>, bus: &Bus) {
    if let Err(err) = runtime.reload() {
        eprintln!("config reload: {err}");
        return;
    }
    runtime.tick();
    write_state(cli, snapshot, bus);
}

fn write_state(cli: &Cli, snapshot: &Mutex<String>, bus: &Bus) {
    let Some(path) = cli.state.as_ref() else {
        return;
    };
    if let Ok(text) = snapshot.lock() {
        let _ = std::fs::write(path, &*text);
        if let Ok(value) = serde_json::from_str(text.as_str()) {
            bus.emit("state", value);
        }
    }
}
