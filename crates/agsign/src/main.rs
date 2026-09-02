//! agsign — update manifest signer (M21, host tool).
//!
//! Signs update manifests for agupd with ed25519. The phone-side
//! contract: a detached signature in `<manifest>.sig` (base64 of the
//! 64-byte ed25519 signature over the RAW manifest bytes — no JSON
//! canonicalization anywhere), verified by agupd against a public key
//! compiled into the binary before anything is parsed, downloaded, or
//! written to a partition.
//!
//! The private key is a local secret (.local/keys/, gitignored); the
//! public key is committed in agupd's source and rotated only by
//! shipping an update signed with the old key that embeds the new one
//! (key rotation is a v2 problem — one key for now).
//!
//! Usage:
//!   agsign keygen <dir>              # writes <dir>/agupd.key (0600 hex
//!                                    #   seed) + <dir>/agupd.pub (base64)
//!   agsign sign   <key> <file>       # writes <file>.sig
//!   agsign verify <pub> <file> [sig] # exit 0 = valid
use std::fs;
use std::process::exit;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

fn die(msg: &str) -> ! {
    eprintln!("agsign: {msg}");
    exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keygen") if args.len() == 3 => keygen(&args[2]),
        Some("sign") if args.len() == 4 => sign(&args[2], &args[3]),
        Some("verify") if args.len() >= 4 => verify(&args[2], &args[3], args.get(4).map(String::as_str)),
        _ => {
            eprintln!("usage: agsign keygen <dir> | sign <key> <file> | verify <pub> <file> [sig]");
            exit(2);
        }
    }
}

fn keygen(dir: &str) {
    use rand_core::{OsRng, RngCore};
    fs::create_dir_all(dir).unwrap_or_else(|e| die(&format!("mkdir {dir}: {e}")));
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let key_path = format!("{dir}/agupd.key");
    let pub_path = format!("{dir}/agupd.pub");
    // seed as hex so the file is printable and diff-friendly
    let seed_hex: String = sk.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
    fs::write(&key_path, format!("{seed_hex}\n")).unwrap_or_else(|e| die(&format!("write {key_path}: {e}")));
    set_0600(&key_path);
    let pub_b64 = B64.encode(sk.verifying_key().as_ref());
    fs::write(&pub_path, format!("{pub_b64}\n")).unwrap_or_else(|e| die(&format!("write {pub_path}: {e}")));
    println!("private: {key_path} (0600 — never leaves this machine)");
    println!("public : {pub_path}");
    println!("embed in agupd: AGUPD_PUBKEY_B64 = \"{pub_b64}\"");
}

fn set_0600(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

fn load_key(path: &str) -> SigningKey {
    let hex = fs::read_to_string(path)
        .unwrap_or_else(|e| die(&format!("read {path}: {e}")))
        .trim()
        .to_string();
    if hex.len() != 64 {
        die(&format!("{path}: want 64 hex chars (32-byte seed), got {}", hex.len()));
    }
    let mut seed = [0u8; 32];
    for (i, slot) in seed.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| die(&format!("{path}: bad hex")));
    }
    SigningKey::from_bytes(&seed)
}

fn sign(key_path: &str, file: &str) {
    let sk = load_key(key_path);
    let bytes = fs::read(file).unwrap_or_else(|e| die(&format!("read {file}: {e}")));
    let sig = sk.sign(&bytes);
    let out = format!("{file}.sig");
    fs::write(&out, B64.encode(sig.to_bytes())).unwrap_or_else(|e| die(&format!("write {out}: {e}")));
    println!("signed {file} -> {out}");
}

fn verify(pub_path: &str, file: &str, sig_path: Option<&str>) {
    let pub_b64 = fs::read_to_string(pub_path)
        .unwrap_or_else(|e| die(&format!("read {pub_path}: {e}")))
        .trim()
        .to_string();
    let key_bytes = B64.decode(pub_b64).unwrap_or_else(|e| die(&format!("pubkey: {e}")));
    let key_bytes: [u8; 32] = key_bytes.try_into().unwrap_or_else(|v: Vec<u8>| die(&format!("pubkey: want 32 bytes, got {}", v.len())));
    let vk = VerifyingKey::from_bytes(&key_bytes).unwrap_or_else(|e| die(&format!("pubkey: {e}")));

    let sig_file = sig_path.unwrap_or(&format!("{file}.sig")).to_string();
    let sig_b64 = fs::read_to_string(&sig_file)
        .unwrap_or_else(|e| die(&format!("read {sig_file}: {e}")))
        .trim()
        .to_string();
    let sig_bytes = B64.decode(&sig_b64).unwrap_or_else(|e| die(&format!("sig: {e}")));
    let sig_bytes: [u8; 64] = sig_bytes.try_into().unwrap_or_else(|v: Vec<u8>| die(&format!("sig: want 64 bytes, got {}", v.len())));
    let sig = Signature::from_bytes(&sig_bytes);

    let bytes = fs::read(file).unwrap_or_else(|e| die(&format!("read {file}: {e}")));
    match vk.verify(&bytes, &sig) {
        Ok(()) => println!("valid"),
        Err(e) => die(&format!("INVALID: {e}")),
    }
}
