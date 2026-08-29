//! agdl — AginxOS downloader (M10).
//!
//! The base image has no working HTTPS fetcher: busybox wget segfaults on
//! this build and /bin/httpget is HTTP-only. The installer (agpkg sync) needs
//! TLS to pull software from GitHub releases, so this is the smallest
//! possible ureq+rustls fetch: `agdl <url> <out>` streams to `<out>.part`
//! and renames on completion, so a killed download never leaves a
//! truncated file at the real path.
//!
//! TLS needs a roughly-correct clock — net-bringup ntpd's before agpkg sync
//! runs. Root certs come from webpki-roots (compiled in; no /etc/ssl on the
//! phone).

use std::fs;
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: agdl <url> <output-file>");
        return ExitCode::from(2);
    }
    let (url, out) = (&args[1], &args[2]);
    match download(url, out) {
        Ok(n) => {
            println!("agdl: {} -> {} ({} bytes)", url, out, n);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agdl: {}: {}", url, e);
            ExitCode::FAILURE
        }
    }
}

fn download(url: &str, out: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let resp = ureq::get(url).call()?;
    let tmp = format!("{out}.part");
    let mut file = fs::File::create(&tmp)?;
    let mut reader = resp.into_body().into_reader();
    let mut buf = [0u8; 65536];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        total += n as u64;
    }
    file.flush()?;
    fs::rename(&tmp, out)?;
    Ok(total)
}
