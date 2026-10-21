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
    let mut runtime = Runtime::start(config, cli.config.clone(), &private_api, Arc::clone(&snapshot), Arc::clone(&metrics), trusted);
    let tap = hotkeys::install();
    hotkeys::remember_main_loop();
    runtime.tick();
    write_state(&cli, &snapshot);
    let mut next_tick = Instant::now() + Duration::from_millis(80);
    loop {
        if lifecycle::stop_requested() {
            runtime.unpark_all();
            write_state(&cli, &snapshot);
            break;
        }
        // The tap callback runs on this thread while the run loop is pumping.
        // Idle waits are long; IPC, the tap, and the display link wake this thread.
        let slice = if runtime.is_animating() { 0.02 } else { 0.25 };
        hotkeys::pump(&mut runtime, &bus, slice);
        runtime.drain_frames();
        let _ = tap.as_ref().map(hotkeys::EventTap::alive);
        for job in bus.poll(Duration::from_millis(0)) {
            let response = dispatch(&mut runtime, &bus, job.message);
            let _ = job.reply.send(response);
        }
        runtime.drain_frames();
        if lifecycle::stop_requested() {
            runtime.unpark_all();
            write_state(&cli, &snapshot);
            break;
        }
        if Instant::now() >= next_tick {
            runtime.tick();
            write_state(&cli, &snapshot);
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

fn write_state(cli: &Cli, snapshot: &Mutex<String>) {
    let Some(path) = cli.state.as_ref() else {
        return;
    };
    if let Ok(text) = snapshot.lock() {
        let _ = std::fs::write(path, &*text);
    }
}
