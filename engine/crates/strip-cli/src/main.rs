//! `strip` talks to the engine over the Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use strip_ipc::{encode, ClientMessage, ServerMessage};

#[derive(Parser)]
#[command(name = "strip", version, about = "Scrolling window manager commands")]
struct Cli {
    #[arg(long, env = "STRIP_SOCKET")]
    socket: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Query {
        what: String,
        #[arg(long)]
        json: bool,
    },
    Subscribe {
        #[arg(long)]
        json: bool,
    },
    /// Debug hooks. Present only in builds with the `debug-hooks` feature.
    #[cfg(feature = "debug-hooks")]
    Debug {
        #[command(subcommand)]
        hook: DebugHook,
    },
    /// An action id, then an optional 1-based index.
    #[command(external_subcommand)]
    Action(Vec<String>),
}

#[cfg(feature = "debug-hooks")]
#[derive(Subcommand)]
enum DebugHook {
    #[command(name = "inject_gesture")]
    InjectGesture {
        #[arg(long)]
        fingers: u8,
        #[arg(long)]
        dx: f64,
        #[arg(long, default_value_t = 0.0)]
        dy: f64,
        #[arg(long)]
        phase: String,
    },
    #[command(name = "simulate_wake")]
    SimulateWake,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stream = match UnixStream::connect(&cli.socket) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("socket: {err}");
            return ExitCode::from(1);
        }
    };
    let mut write = match stream.try_clone() {
        Ok(write) => write,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let mut reader = BufReader::new(stream);
    if send(&mut write, &ClientMessage::Hello { id: 1, client: "cli".to_string(), protocol: 1 }).is_err() {
        return ExitCode::from(1);
    }
    if read_message(&mut reader).is_err() {
        return ExitCode::from(1);
    }
    match cli.command {
        Command::Query { what, json: _ } => one_shot(&mut write, &mut reader, &ClientMessage::Query { id: 2, what }, true),
        Command::Subscribe { json: _ } => subscribe(&mut write, &mut reader),
        #[cfg(feature = "debug-hooks")]
        Command::Debug { hook } => {
            let (action, args) = match hook {
                DebugHook::InjectGesture { fingers, dx, dy, phase } => {
                    ("inject_gesture", serde_json::json!({ "fingers": fingers, "dx": dx, "dy": dy, "phase": phase }))
                }
                DebugHook::SimulateWake => ("simulate_wake", serde_json::json!({})),
            };
            one_shot(
                &mut write,
                &mut reader,
                &ClientMessage::Command { id: 2, action: action.to_string(), args },
                false,
            )
        }
        Command::Action(args) => {
            let Some(action) = args.first() else {
                eprintln!("missing action");
                return ExitCode::from(1);
            };
            let mut body = serde_json::Map::new();
            if let Some(n) = args.get(1).and_then(|text| text.parse::<u64>().ok()) {
                body.insert("n".to_string(), serde_json::json!(n));
            }
            one_shot(
                &mut write,
                &mut reader,
                &ClientMessage::Command { id: 2, action: action.clone(), args: serde_json::Value::Object(body) },
                false,
            )
        }
    }
}

fn one_shot(write: &mut UnixStream, reader: &mut BufReader<UnixStream>, message: &ClientMessage, data_only: bool) -> ExitCode {
    if let Err(err) = send(write, message) {
        eprintln!("{err}");
        return ExitCode::from(1);
    }
    match read_message(reader) {
        Ok(response) => print_response(&response, data_only),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn subscribe(write: &mut UnixStream, reader: &mut BufReader<UnixStream>) -> ExitCode {
    let message = ClientMessage::Subscribe {
        id: 2,
        events: ["state", "focus", "indicator", "tour", "log"].into_iter().map(str::to_string).collect(),
    };
    if let Err(err) = send(write, &message) {
        eprintln!("{err}");
        return ExitCode::from(1);
    }
    let mut stdout = std::io::stdout().lock();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return ExitCode::SUCCESS,
            Ok(_) => {
                if write!(stdout, "{line}").is_err() || stdout.flush().is_err() {
                    return ExitCode::from(1);
                }
            }
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        }
    }
}

fn print_response(response: &ServerMessage, data_only: bool) -> ExitCode {
    let failed = matches!(response, ServerMessage::Result { ok: false, .. });
    if data_only {
        if let ServerMessage::Result { data: Some(data), .. } = response {
            println!("{data}");
        } else {
            println!("{}", serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string()));
        }
    } else {
        println!("{}", serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string()));
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn send(write: &mut UnixStream, message: &ClientMessage) -> Result<(), String> {
    write.write_all(encode(message).as_bytes()).map_err(|err| err.to_string())
}

fn read_message(reader: &mut BufReader<UnixStream>) -> Result<ServerMessage, String> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(|err| err.to_string())?;
    if read == 0 {
        return Err("socket closed".to_string());
    }
    serde_json::from_str(line.trim()).map_err(|err| err.to_string())
}
