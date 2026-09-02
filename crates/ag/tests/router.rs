// Integration tests for the ag router, stub-the-world style: AG_CMD_PATH
// points at a throwaway dir of ag-* sh stubs and AG_GROUPS_DESC at a
// nonexistent path, so nothing here touches the host's real /var/bin,
// /usr/bin or /etc. Every assertion goes through the real binary
// (CARGO_BIN_EXE_ag), the same process an agent would spawn.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = testkit::tmp(&format!("router-{name}"));
    for (f, c) in files {
        testkit::write_exec(&d.join(f), c.as_bytes());
    }
    d
}

fn ag(cmd_path: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ag"));
    c.env("AG_CMD_PATH", cmd_path)
        .env("AG_GROUPS_DESC", "/nonexistent/groups.desc")
        .args(args);
    c
}

fn run(cmd_path: &str, args: &[&str]) -> std::process::Output {
    ag(cmd_path, args).output().unwrap()
}

/// Stub that records its argv, one per line, into $RECORD.
const RECORDER: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$RECORD"
"#;

fn recorder_with(header: &str) -> String {
    format!("{RECORDER}{header}\n")
}

fn record_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ag-router-rec-{name}-{}", std::process::id()))
}

fn recorded(p: &Path) -> Vec<String> {
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn exit_code_passthrough() {
    let d = fixture("exit7", &[("ag-exit7", "#!/bin/sh\nexit 7\n")]);
    let out = run(d.to_str().unwrap(), &["exit7"]);
    assert_eq!(out.status.code(), Some(7));
}

#[test]
fn signal_passthrough() {
    use std::os::unix::process::ExitStatusExt;
    let d = fixture("sig", &[("ag-sig", "#!/bin/sh\nkill -TERM $$\n")]);
    let out = run(d.to_str().unwrap(), &["sig"]);
    assert_eq!(out.status.code(), None);
    assert_eq!(out.status.signal(), Some(15));
}

#[test]
fn args_passed_verbatim() {
    let d = fixture(
        "args",
        &[("ag-echo", &recorder_with("# ag:summary=echo args back"))],
    );
    let rec = record_path("args");
    let _ = fs::remove_file(&rec);
    ag(d.to_str().unwrap(), &["echo", "one", "two words"])
        .env("RECORD", &rec)
        .output()
        .unwrap();
    assert_eq!(recorded(&rec), vec!["one", "two words"]);
}

#[test]
fn longest_prefix_consumes_only_prefix_words() {
    let d = fixture(
        "prefix",
        &[
            ("ag-cam-shot", &recorder_with("# ag:summary=shot")),
            ("ag-cam", &recorder_with("# ag:summary=cam")),
        ],
    );
    let rec = record_path("prefix");
    let _ = fs::remove_file(&rec);
    let out = ag(d.to_str().unwrap(), &["cam", "shot", "extra"])
        .env("RECORD", &rec)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(recorded(&rec), vec!["extra"]);
}

#[test]
fn help_flag_anywhere_never_executes_target() {
    let d = fixture(
        "helpflag",
        &[(
            "ag-danger",
            "#!/bin/sh\n# ag:summary=dangerous stub\n# ag:args=<x>\ntouch \"$SENTINEL\"\nexit 9\n",
        )],
    );
    let sent = record_path("helpflag-sentinel");
    let _ = fs::remove_file(&sent);
    for args in [
        vec!["danger", "--help"],
        vec!["danger", "x", "--help"],
        vec!["danger", "x", "-h"],
        vec!["--help", "danger"],
    ] {
        let out = ag(d.to_str().unwrap(), &args)
            .env("SENTINEL", &sent)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "args: {args:?}");
        let so = String::from_utf8_lossy(&out.stdout);
        assert!(so.contains("ag danger"), "args: {args:?}");
        assert!(so.contains("dangerous stub"), "args: {args:?}");
    }
    assert!(!sent.exists(), "target must never run under --help");
}

#[test]
fn required_args_refused_when_bare() {
    let d = fixture(
        "reqargs",
        &[(
            "ag-snd-cap",
            &recorder_with("# ag:summary=cap\n# ag:args=<seconds> [out.wav]"),
        )],
    );
    let rec = record_path("reqargs");
    let _ = fs::remove_file(&rec);
    let out = ag(d.to_str().unwrap(), &["snd-cap"]).env("RECORD", &rec).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!rec.exists(), "refused bare call must not execute");
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(se.contains("requires arguments"), "stderr: {se}");

    // With the required arg it goes through.
    let out = ag(d.to_str().unwrap(), &["snd", "cap", "5"])
        .env("RECORD", &rec)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(recorded(&rec), vec!["5"]);
}

#[test]
fn earlier_dir_shadows_later() {
    let a = fixture("shadow-a", &[("ag-dupe", "#!/bin/sh\necho from-a\n")]);
    let b = fixture("shadow-b", &[("ag-dupe", "#!/bin/sh\necho from-b\n")]);
    let cmd_path = format!("{}:{}", a.display(), b.display());
    let out = run(&cmd_path, &["dupe"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-a");
}

#[test]
fn hidden_off_menu_but_still_routes() {
    let files = [
        ("ag-zz-open", "#!/bin/sh\n# ag:summary=open one\necho ok-open\n"),
        (
            "ag-zz-secret",
            "#!/bin/sh\n# ag:summary=hidden one\n# ag:hidden=true\necho ok-hidden\n",
        ),
    ];
    let d = fixture("hidden", &files);
    let out = run(d.to_str().unwrap(), &["commands"]);
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(so.contains("zz-open"), "menu: {so}");
    assert!(!so.contains("zz-secret"), "menu: {so}");

    let out = run(d.to_str().unwrap(), &["commands", "--all"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("zz-secret"));

    let out = run(d.to_str().unwrap(), &["zz-secret"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok-hidden");
}

#[test]
fn unknown_exit_127_with_did_you_mean() {
    let d = fixture(
        "suggest",
        &[("ag-cam-shot", "#!/bin/sh\n# ag:summary=shot\necho ok\n")],
    );
    let out = run(d.to_str().unwrap(), &["cam-sho"]);
    assert_eq!(out.status.code(), Some(127));
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(se.contains("unknown command"), "stderr: {se}");
    assert!(se.contains("cam-shot"), "stderr: {se}");

    // Prefix-only miss lists the matches instead of a typo guess.
    let out = run(d.to_str().unwrap(), &["cam"]);
    assert_eq!(out.status.code(), Some(127));
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(se.contains("prefix matches"), "stderr: {se}");
}

#[test]
fn json_envelope_roundtrip() {
    let d = fixture(
        "json",
        &[(
            "ag-cam-shot",
            "#!/bin/sh\n# ag:summary=take a still\n# ag:args=<cam> [out]\n# ag:examples=0 /tmp/x.png\ntrue\n",
        )],
    );
    let out = run(d.to_str().unwrap(), &["commands", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["ok"], serde_json::json!(true));
    let data = v["data"].as_array().expect("data array");
    let rec = data
        .iter()
        .find(|r| r["route"] == "cam-shot")
        .expect("cam-shot record");
    assert_eq!(rec["summary"], "take a still");
    assert_eq!(rec["args"], "<cam> [out]");
    assert_eq!(rec["examples"][0], "0 /tmp/x.png");
    assert_eq!(v["meta"]["count"], serde_json::json!(1));
}

#[test]
fn check_clean_registry_exits_zero() {
    let d = fixture(
        "clean",
        &[(
            "ag-ok-one",
            "#!/bin/sh\n# ag:summary=all good\necho ok\n",
        )],
    );
    let out = run(d.to_str().unwrap(), &["commands", "--check"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
}

#[test]
fn check_reports_each_lint_class() {
    let d = fixture(
        "dirty",
        &[
            // missing summary
            ("ag-bad-nosum", "#!/bin/sh\necho x\n"),
            // bad boolean + unknown key
            (
                "ag-bad-bool",
                "#!/bin/sh\n# ag:summary=b\n# ag:hidden=maybe\n# ag:wat=1\necho x\n",
            ),
            // pass-through shim whose target does not exist
            (
                "ag-bad-exec",
                "#!/bin/sh\n# ag:summary=c\n# ag:exec=/nonexistent/target\nexec /nonexistent/target \"$@\"\n",
            ),
        ],
    );
    // compiled command without .agmd sidecar (non-shebang bytes)
    let binp = d.join("ag-bad-bin");
    testkit::write_exec(&binp, b"\x7fELF-not-really\x00\x01");

    let out = run(d.to_str().unwrap(), &["commands", "--check"]);
    assert_eq!(out.status.code(), Some(1));
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(se.contains("missing ag:summary"), "stderr: {se}");
    assert!(se.contains("bad boolean"), "stderr: {se}");
    assert!(se.contains("unknown key"), "stderr: {se}");
    assert!(se.contains("ag:exec target missing"), "stderr: {se}");
    assert!(se.contains(".agmd sidecar"), "stderr: {se}");
}

#[test]
fn check_reports_route_collision() {
    let d = fixture(
        "collide",
        &[
            (
                "ag-alpha",
                "#!/bin/sh\n# ag:summary=a\n# ag:alias=same\necho a\n",
            ),
            (
                "ag-beta",
                "#!/bin/sh\n# ag:summary=b\n# ag:alias=same\necho b\n",
            ),
        ],
    );
    let out = run(d.to_str().unwrap(), &["commands", "--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("collision"));
}

#[test]
fn alias_and_name_override_route() {
    let d = fixture(
        "alias",
        &[
            (
                "ag-net-scan",
                &recorder_with("# ag:summary=scan\n# ag:alias=wifi-scan"),
            ),
            (
                "ag-weird-name",
                &recorder_with("# ag:summary=renamed\n# ag:name=zz-short"),
            ),
        ],
    );
    let rec = record_path("alias");
    let _ = fs::remove_file(&rec);
    let out = ag(d.to_str().unwrap(), &["wifi-scan"]).env("RECORD", &rec).output().unwrap();
    assert_eq!(out.status.code(), Some(0));

    let out = ag(d.to_str().unwrap(), &["zz-short"]).env("RECORD", &rec).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn bare_invocation_shows_menu() {
    let d = fixture("menu", &[("ag-x-one", "#!/bin/sh\n# ag:summary=x\necho ok\n")]);
    let out = run(d.to_str().unwrap(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(so.contains("usage"), "stdout: {so}");
    assert!(so.contains("x-one"), "stdout: {so}");
}

#[test]
fn help_on_builtin_commands() {
    let out = run("/nonexistent", &["commands", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("usage: ag commands"));
}
