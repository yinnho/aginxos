//! AginxOS early system agent.
//!
//! Listens on a Unix socket for simple control messages while logging heartbeats.
//! Intended to run under the AginxOS rootfs once the Pixel 5 boots our userspace.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::time;

const DEFAULT_SOCK: &str = "/run/aginxos/agent.sock";
const DEFAULT_LOG: &str = "/var/log/aginxos-agent.log";

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Ping,
    Version,
}

#[derive(Debug, Serialize)]
struct Response {
    ok: bool,
    msg: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let sock_path = std::env::var("AGINXOS_SOCK").unwrap_or_else(|_| DEFAULT_SOCK.into());
    let log_path = std::env::var("AGINXOS_LOG").unwrap_or_else(|_| DEFAULT_LOG.into());

    if let Some(parent) = Path::new(&sock_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Some(parent) = Path::new(&log_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("bind {sock_path}"))?;
    println!("AginxOS agent listening on {sock_path}");

    let log_path_hb = log_path.clone();
    tokio::spawn(async move {
        let mut tick = time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let line = format!(
                "{} heartbeat pid={}\n",
                chrono_like_now(),
                std::process::id()
            );
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path_hb)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(line.as_bytes())
                });
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream).await {
                eprintln!("client error: {e:#}");
            }
        });
    }
}

async fn handle_client(stream: UnixStream) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let req: Request = serde_json::from_str(line.trim())
        .unwrap_or(Request::Ping);

    let resp = match req {
        Request::Ping => Response {
            ok: true,
            msg: "pong".into(),
        },
        Request::Version => Response {
            ok: true,
            msg: format!("AginxOS agent {}", env!("CARGO_PKG_VERSION")),
        },
    };

    let mut stream = reader.into_inner();
    let body = serde_json::to_string(&resp)? + "\n";
    stream.write_all(body.as_bytes()).await?;
    Ok(())
}

fn chrono_like_now() -> String {
    // Keep deps minimal for early bring-up: unix timestamp is enough.
    format!(
        "ts={}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    )
}
