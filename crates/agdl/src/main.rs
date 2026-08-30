//! agdl — AginxOS downloader (M10).
//!
//! The base image has no working HTTPS fetcher: busybox wget segfaults on
//! this build and /bin/httpget is HTTP-only. The installer (agpkg sync) needs
//! TLS to pull software from GitHub releases, so this is the smallest
//! possible ureq+rustls fetch: `agdl <url> <out>` streams to `<out>.part`
//! and renames on completion, so a killed download never leaves a
//! truncated file at the real path. `<out>` of `-` streams to stdout.
//!
//! TLS needs a roughly-correct clock — net-bringup ntpd's before agpkg sync
//! runs. Root certs come from webpki-roots (compiled in; no /etc/ssl on the
//! phone).

use std::fs;
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    // agdl [-X METHOD] [-H "k: v"]... [-d @file] <url> [output-file]
    let mut method = String::from("GET");
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data: Option<String> = None;
    let mut pos: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-X" => match args.next() {
                Some(m) => method = m,
                None => return usage(),
            },
            "-H" => match args.next() {
                Some(h) => match h.split_once(':') {
                    Some((k, v)) => headers.push((k.trim().to_string(), v.trim().to_string())),
                    None => return usage(),
                },
                None => return usage(),
            },
            "-d" => match args.next().and_then(|d| d.strip_prefix('@').map(str::to_string)) {
                Some(d) => data = Some(d),
                None => return usage(),
            },
            _ => pos.push(a),
        }
    }
    if pos.is_empty() || pos.len() > 2 {
        return usage();
    }
    let url = &pos[0];
    let out: &str = if pos.len() == 2 { &pos[1] } else { "-" };
    match fetch(&method, url, &headers, data.as_deref(), out) {
        Ok((status, n)) => {
            eprintln!("agdl: {} {} -> {} ({} bytes)", method, url, out, n);
            eprintln!("agdl: HTTP {}", status);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agdl: {} {}: {}", method, url, e);
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: agdl [-X METHOD] [-H \"k: v\"]... [-d @file] <url> [output-file]   (no file = stdout)");
    ExitCode::from(2)
}

fn apply_headers<B>(
    mut rb: ureq::RequestBuilder<B>,
    headers: &[(String, String)],
) -> ureq::RequestBuilder<B> {
    for (k, v) in headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    rb
}

fn fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    data: Option<&str>,
    out: &str,
) -> Result<(u16, u64), Box<dyn std::error::Error>> {
    // Non-2xx must not error: probes want the response body either way.
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let m = method.to_ascii_uppercase();
    let resp = match data {
        Some(d) => {
            let body = fs::read(d)?;
            let rb = match m.as_str() {
                "PUT" => apply_headers(agent.put(url), headers),
                "PATCH" => apply_headers(agent.patch(url), headers),
                _ => apply_headers(agent.post(url), headers),
            };
            rb.send(&body[..])?
        }
        None => {
            let rb = match m.as_str() {
                "HEAD" => apply_headers(agent.head(url), headers),
                "DELETE" => apply_headers(agent.delete(url), headers),
                "OPTIONS" => apply_headers(agent.options(url), headers),
                _ => apply_headers(agent.get(url), headers),
            };
            rb.call()?
        }
    };
    let status = resp.status().as_u16();
    let tmp = format!("{out}.part");
    let stdout = out == "-";
    let mut file = if stdout { None } else { Some(fs::File::create(&tmp)?) };
    let mut reader = resp.into_body().into_reader();
    let mut buf = [0u8; 65536];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        match &mut file {
            Some(f) => f.write_all(&buf[..n])?,
            None => std::io::stdout().write_all(&buf[..n])?,
        }
        total += n as u64;
    }
    if let Some(mut f) = file {
        f.flush()?;
        fs::rename(&tmp, out)?;
    } else {
        std::io::stdout().flush()?;
    }
    Ok((status, total))
}
