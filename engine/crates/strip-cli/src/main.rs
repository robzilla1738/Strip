//! `strip` talks to the engine over the Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    /// Run an action id from the shortcut table.
    Run {
        action: String,
        n: Option<u32>,
    },
    Query {
        what: String,
        #[arg(long)]
        json: bool,
    },
    Subscribe {
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut stream = match UnixStream::connect(&cli.socket) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("socket: {err}");
            return ExitCode::from(1);
        }
    };
    if exchange(&mut stream, &serde_json::json!({"id": 1, "type": "hello", "client": "cli", "protocol": 1})).is_err() {
        return ExitCode::from(1);
    }
    let request = match cli.command {
        Command::Query { what, json: _ } => serde_json::json!({"id": 2, "type": "query", "what": what}),
        Command::Run { action, n } => {
            let mut args = serde_json::Map::new();
            if let Some(n) = n {
                args.insert("n".to_string(), serde_json::json!(n));
            }
            serde_json::json!({"id": 2, "type": "command", "action": action, "args": args})
        }
        Command::Subscribe { json: _ } => serde_json::json!({"id": 2, "type": "subscribe", "events": ["state", "focus", "indicator", "tour", "log"]}),
    };
    match exchange(&mut stream, &request) {
        Ok(response) => {
            if let Some(data) = response.get("data") {
                println!("{data}");
            } else {
                println!("{response}");
            }
            if response.get("ok").and_then(|value| value.as_bool()) == Some(false) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn exchange(stream: &mut UnixStream, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut line = serde_json::to_string(request).map_err(|err| err.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|err| err.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|err| err.to_string())?);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(|err| err.to_string())?;
    serde_json::from_str(response.trim()).map_err(|err| err.to_string())
}
