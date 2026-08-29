// Anti-drift codegen: parse lib/core/app_protection_data.sh (the single source
// of truth for protection lists) into Rust constants at build time. The parser
// is fail-closed — any line it does not recognize aborts the build, so a shape
// change in the shell data file can never silently produce empty Rust lists.

use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The six declarations the data file is allowed to contain (five arrays + one scalar).
const ARRAY_NAMES: [&str; 5] = [
    "SYSTEM_CRITICAL_BUNDLES",
    "APPLE_UNINSTALLABLE_APPS",
    "OFFICIAL_UNINSTALLER_RULES",
    "ENDPOINT_SECURITY_BUNDLE_PREFIXES",
    "DATA_PROTECTED_BUNDLES",
];
const SCALAR_NAME: &str = "LAUNCH_AGENT_NAME_COMMON_WORDS";

/// The protection-data file, vendored INSIDE this crate so the build has no
/// dependency on any external project tree — the desktop app is self-contained.
fn data_file_path() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    manifest.join("data/app_protection_data.sh")
}

/// Extract the double-quoted string from an array-entry line, rejecting escapes
/// and interpolation the shell file is contractually free of.
fn parse_quoted_entry(line: &str, path: &Path, lineno: usize) -> String {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('"')
        .unwrap_or_else(|| die(path, lineno, "array entry must start with a double quote"));
    let close = inner
        .find('"')
        .unwrap_or_else(|| die(path, lineno, "array entry missing closing quote"));
    let value = &inner[..close];
    let rest = inner[close + 1..].trim();
    if !(rest.is_empty() || rest.starts_with('#')) {
        die::<()>(
            path,
            lineno,
            "trailing content after array entry must be a comment",
        );
    }
    if value.is_empty() {
        die::<()>(path, lineno, "empty array entry");
    }
    if value.contains('\\') || value.contains('$') || value.contains('`') {
        die::<()>(
            path,
            lineno,
            "escapes/interpolation are not allowed in data entries",
        );
    }
    value.to_string()
}

/// Abort the build with a location-tagged parse error (fail-closed contract).
fn die<T>(path: &Path, lineno: usize, msg: &str) -> T {
    panic!(
        "app_protection_data.sh drift: {} at {}:{} — update build.rs in the same change",
        msg,
        path.display(),
        lineno
    );
}

/// Parse the whole data file into (array name → entries) plus the scalar value.
fn parse(path: &Path, src: &str) -> (Vec<(String, Vec<String>)>, String) {
    let mut arrays: Vec<(String, Vec<String>)> = Vec::new();
    let mut scalar: Option<String> = None;
    let mut current: Option<(String, Vec<String>)> = None;

    for (idx, raw) in src.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw.trim();
        if let Some((name, entries)) = current.as_mut() {
            if line == ")" {
                if entries.is_empty() {
                    die::<()>(path, lineno, "empty array");
                }
                arrays.push(current.take().unwrap());
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let _ = name;
            entries.push(parse_quoted_entry(line, path, lineno));
            continue;
        }

        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Fixed preamble the loader needs; anything else in this shape is drift.
        if line == "set -euo pipefail"
            || line == "if [[ -n \"${MOLE_APP_PROTECTION_DATA_LOADED:-}\" ]]; then"
            || line == "return 0"
            || line == "fi"
            || line == "readonly MOLE_APP_PROTECTION_DATA_LOADED=1"
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("readonly ") {
            if let Some(name) = rest.strip_suffix("=(") {
                if !ARRAY_NAMES.contains(&name) {
                    die::<()>(path, lineno, "unknown readonly array");
                }
                current = Some((name.to_string(), Vec::new()));
                continue;
            }
            if let Some(value) = rest
                .strip_prefix(SCALAR_NAME)
                .and_then(|v| v.strip_prefix("=\""))
                .and_then(|v| v.strip_suffix('"'))
            {
                scalar = Some(value.to_string());
                continue;
            }
        }
        die::<()>(path, lineno, "unrecognized line");
    }

    if current.is_some() {
        die::<()>(path, 0, "unterminated array");
    }
    for name in ARRAY_NAMES {
        if !arrays.iter().any(|(n, _)| n == name) {
            die::<()>(path, 0, &format!("missing array {name}"));
        }
    }
    let scalar = scalar.unwrap_or_else(|| die(path, 0, "missing LAUNCH_AGENT_NAME_COMMON_WORDS"));
    (arrays, scalar)
}

/// Render one `pub static NAME: &[&str]` slice literal.
fn emit_slice(out: &mut String, name: &str, entries: &[String]) {
    writeln!(out, "pub static {name}: &[&str] = &[").unwrap();
    for entry in entries {
        writeln!(out, "    {entry:?},").unwrap();
    }
    writeln!(out, "];").unwrap();
}

fn main() {
    let path = data_file_path();
    println!("cargo:rerun-if-changed={}", path.display());
    let src =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let (arrays, scalar) = parse(&path, &src);

    let mut out =
        String::from("// @generated by mole-core/build.rs from lib/core/app_protection_data.sh\n");
    let digest = Sha256::digest(src.as_bytes());
    writeln!(
        out,
        "pub static DATA_SHA256: &str = {:?};",
        format!("{digest:x}")
    )
    .unwrap();

    for (name, entries) in &arrays {
        if name == "OFFICIAL_UNINSTALLER_RULES" {
            // vendor|prefix1,prefix2|frag1,frag2 — split at build time so runtime never parses.
            writeln!(
                out,
                "pub static OFFICIAL_UNINSTALLER_RULES: &[super::OfficialUninstallerRule] = &["
            )
            .unwrap();
            for entry in entries {
                let parts: Vec<&str> = entry.split('|').collect();
                if parts.len() != 3 {
                    die::<()>(
                        &path,
                        0,
                        "OFFICIAL_UNINSTALLER_RULES row must have 3 |-fields",
                    );
                }
                let prefixes: Vec<&str> = parts[1].split(',').filter(|p| !p.is_empty()).collect();
                let fragments: Vec<&str> = parts[2].split(',').filter(|f| !f.is_empty()).collect();
                writeln!(
                    out,
                    "    super::OfficialUninstallerRule {{ vendor: {:?}, bundle_prefixes: &{prefixes:?}, name_fragments: &{fragments:?} }},",
                    parts[0]
                )
                .unwrap();
            }
            writeln!(out, "];").unwrap();
        } else {
            emit_slice(&mut out, name, entries);
        }
    }
    writeln!(
        out,
        "pub static LAUNCH_AGENT_NAME_COMMON_WORDS: &[&str] = &{:?};",
        scalar.split('|').collect::<Vec<_>>()
    )
    .unwrap();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("app_protection_data.rs"), out).expect("write generated data");
}
