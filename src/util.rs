//! Small helpers: hashing, safe file moves, name handling, UTC timestamps.

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// SHA-256 of a file, as lowercase hex. Streams the file, so large files are fine.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Move a file without ever overwriting an existing destination.
///
/// Tries a fast rename first. If that fails (for example, moving across drives),
/// it copies, verifies the copy by hash, and only then removes the original.
pub fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}", dst.display()),
        ));
    }
    let rename_err = match fs::rename(src, dst) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    if let Err(e) = fs::copy(src, dst) {
        let _ = fs::remove_file(dst);
        return Err(e);
    }
    let verified = matches!((sha256_file(src), sha256_file(dst)), (Ok(a), Ok(b)) if a == b);
    if !verified {
        let _ = fs::remove_file(dst);
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("copy verification failed (rename error was: {rename_err})"),
        ));
    }
    if let Err(e) = fs::remove_file(src) {
        let _ = fs::remove_file(dst);
        return Err(io::Error::new(
            e.kind(),
            format!("could not remove original after copy, left it in place ({e})"),
        ));
    }
    Ok(())
}

/// Split "report.final.pdf" into ("report.final", ".pdf"). Handles .tar.gz style names.
pub fn split_name(name: &str) -> (&str, &str) {
    let lower = name.to_lowercase();
    for compound in [".tar.gz", ".tar.bz2", ".tar.xz"] {
        if lower.ends_with(compound) && name.len() > compound.len() {
            let cut = name.len() - compound.len();
            return (&name[..cut], &name[cut..]);
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// "Document.pdf" + 2 -> "Document (2).pdf"
pub fn numbered_name(name: &str, n: u32) -> String {
    let (stem, ext) = split_name(name);
    format!("{stem} ({n}){ext}")
}

/// Lowercased extension including the dot, e.g. ".pdf". None if there is no extension.
pub fn extension_of(name: &str) -> Option<String> {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => Some(name[i..].to_lowercase()),
        _ => None,
    }
}

/// Seconds since the Unix epoch.
fn unix_parts(t: SystemTime) -> (i64, u32) {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_millis()),
        Err(_) => (0, 0),
    }
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts(t: SystemTime) -> (i64, u32, u32, u32, u32, u32, u32) {
    let (secs, millis) = unix_parts(t);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, rem / 3600, (rem % 3600) / 60, rem % 60, millis)
}

/// ISO 8601 UTC, e.g. "2026-10-22T13:18:00Z"
pub fn iso_timestamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, _) = parts(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Transaction id, e.g. "tx_20261022_131800_042"
pub fn transaction_id(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, ms) = parts(t);
    format!("tx_{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}_{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn numbered_names() {
        assert_eq!(numbered_name("Document.pdf", 1), "Document (1).pdf");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(numbered_name("README", 3), "README (3)");
        assert_eq!(numbered_name(".hidden", 1), ".hidden (1)");
    }

    #[test]
    fn extensions() {
        assert_eq!(extension_of("A.PDF"), Some(".pdf".into()));
        assert_eq!(extension_of("noext"), None);
        assert_eq!(extension_of(".env"), None);
        assert_eq!(extension_of("trailing."), None);
    }

    #[test]
    fn timestamps() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_674_000); // 2026-10-22T13:00:00Z
        assert_eq!(iso_timestamp(t), "2026-10-22T13:00:00Z");
        assert_eq!(iso_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let leap = UNIX_EPOCH + Duration::from_secs(1_709_164_800); // 2024-02-29
        assert_eq!(iso_timestamp(leap), "2024-02-29T00:00:00Z");
    }
}
