//! SHA-256 helpers for release assets and `SHA256SUMS` files.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// GitHub asset `digest` values look like `sha256:<hex>`.
pub fn parse_digest(digest: Option<&str>) -> Option<String> {
    let digest = digest?.trim();
    let hex = digest
        .strip_prefix("sha256:")
        .or_else(|| digest.strip_prefix("SHA256:"))
        .unwrap_or(digest);
    if is_hex64(hex) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

pub fn is_sums_filename(name: &str) -> bool {
    let base = file_base(name);
    matches!(
        base.as_str(),
        "sha256sums"
            | "sha256sums.txt"
            | "sha256sum"
            | "sha256sum.txt"
            | "checksums"
            | "checksums.txt"
    )
}

pub fn is_checksum_name(name: &str) -> bool {
    let base = file_base(name);
    is_sums_filename(&base)
        || base.ends_with(".sha256")
        || base.ends_with(".sha256sum")
        || base.ends_with(".sha512")
        || base.ends_with(".checksum")
}

/// Pull the hex digest for `filename` out of a sums file or a one-line sidecar.
pub fn parse_checksum_file(text: &str, filename: &str) -> Option<String> {
    let trimmed = text.trim();
    if is_hex64(trimmed) {
        return Some(trimmed.to_ascii_lowercase());
    }
    let target = filename.to_ascii_lowercase();
    let base = file_base(filename);
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, hex)) = parse_bsd_line(line) {
            let name_l = name.to_ascii_lowercase();
            if name_l == target || name_l == base || name_l.ends_with(&format!("/{base}")) {
                return Some(hex);
            }
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hex) = parts.next() else {
            continue;
        };
        if !is_hex64(hex) {
            continue;
        }
        let rest = line[hex.len()..].trim().trim_start_matches('*').trim();
        if rest.is_empty() {
            continue;
        }
        let name_l = rest.to_ascii_lowercase();
        if name_l == target || name_l == base || name_l.ends_with(&format!("/{base}")) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

fn parse_bsd_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("SHA256 ")?;
    let (name, hex) = rest.split_once('=')?;
    let name = name
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let hex = hex.trim();
    if name.is_empty() || !is_hex64(hex) {
        None
    } else {
        Some((name.to_string(), hex.to_ascii_lowercase()))
    }
}

fn file_base(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_abc() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parses_digest_field_and_sums_formats() {
        assert_eq!(
            parse_digest(Some(
                "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            ))
            .as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let sums = "\
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  tool.AppImage
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef *other.zip
";
        assert_eq!(
            parse_checksum_file(sums, "tool.AppImage").as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let bsd = "SHA256 (tool.AppImage) = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n";
        assert!(parse_checksum_file(bsd, "dist/tool.AppImage").is_some());
        assert!(parse_checksum_file(sums, "missing.bin").is_none());
    }

    #[test]
    fn sidecar_hex_only() {
        assert_eq!(
            parse_checksum_file(
                "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD\n",
                "tool"
            )
            .as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }
}
