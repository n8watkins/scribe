//! Verifies an updater artifact the way tauri-plugin-updater 2.14.1 does.
//!
//! Usage: bundle-bridge-verify <file> <signature (the .sig file content, base64)>
//!        <pubkey (base64, as in plugins.updater.pubkey)> <announced version>
//!        <require-signed-version: true|false>
//!
//! The signature and pubkey arguments are file paths whose content is read and
//! trimmed, so the exact strings latest.json and tauri.conf.json carry are used.
//! Exit code 0 means the plugin would accept the update, 1 that it would refuse.
//! The logic below is copied from the plugin (verify_signature,
//! verify_signed_version, signed_version, base64_to_string), keeping its order:
//! the trusted comment is read only after the global signature is verified.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use std::str::FromStr;

fn base64_to_string(base64_string: &str) -> Result<String, String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(base64_string)
        .map_err(|e| format!("base64: {e}"))?;
    String::from_utf8(decoded).map_err(|_| "signature is not UTF-8".to_string())
}

fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
}

fn verify_signed_version(
    trusted_comment: &str,
    announced_version: &str,
    require_signed_version: bool,
) -> Result<(), String> {
    let Some(signed) = signed_version(trusted_comment) else {
        return if require_signed_version {
            Err("MissingSignedVersion".into())
        } else {
            Ok(())
        };
    };
    let matches = match (
        Version::from_str(signed.trim_start_matches('v')),
        Version::from_str(announced_version.trim_start_matches('v')),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => signed == announced_version,
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "SignedVersionMismatch signed={signed} announced={announced_version}"
        ))
    }
}

fn verify_signature(
    data: &[u8],
    release_signature: &str,
    pub_key: &str,
    announced_version: &str,
    require_signed_version: bool,
) -> Result<String, String> {
    let pub_key_decoded = base64_to_string(pub_key)?;
    let public_key = PublicKey::decode(&pub_key_decoded).map_err(|e| format!("pubkey: {e}"))?;
    let signature_decoded = base64_to_string(release_signature)?;
    let signature =
        Signature::decode(&signature_decoded).map_err(|e| format!("signature: {e}"))?;
    public_key
        .verify(data, &signature, true)
        .map_err(|e| format!("verify: {e}"))?;
    verify_signed_version(
        signature.trusted_comment(),
        announced_version,
        require_signed_version,
    )?;
    Ok(signature.trusted_comment().to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        eprintln!("usage: bundle-bridge-verify <file> <sig file> <pubkey file> <announced version> <require-signed-version>");
        std::process::exit(2);
    }
    let read = |path: &str| {
        std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
            .trim()
            .to_string()
    };
    let data = std::fs::read(&args[0]).unwrap_or_else(|e| panic!("cannot read {}: {e}", args[0]));
    let signature = read(&args[1]);
    let pubkey = read(&args[2]);
    let require = args[4] == "true";
    match verify_signature(&data, &signature, &pubkey, &args[3], require) {
        Ok(comment) => {
            println!("ACCEPT trusted comment: {}", comment.replace('\t', " | "));
        }
        Err(e) => {
            println!("REFUSE {e}");
            std::process::exit(1);
        }
    }
}
