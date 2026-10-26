//! Where results go and what they're called. Results never overwrite anything:
//! "Name Suffix.ext", then "Name Suffix (2).ext", and so on.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputDir {
    /// Next to the original (the reference app's behaviour).
    Beside,
    Folder(PathBuf),
}

/// Used when the source's folder can't be written to (read-only media, protected folders).
pub fn fallback_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Downloads").join("File Beaver")
}

pub fn target_dir(src: &Path, out: &OutputDir) -> PathBuf {
    match out {
        OutputDir::Folder(dir) if !dir.as_os_str().is_empty() => dir.clone(),
        _ => src.parent().map(Path::to_path_buf).unwrap_or_else(fallback_dir),
    }
}

/// "<stem><suffix>.<ext>" in `dir`, numbered if taken.
pub fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

pub fn stem(src: &Path) -> String {
    src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())
}

/// Writes `bytes` as a new file named "<stem>.<ext>" (numbered if taken) and returns its path.
/// Written to a temporary name first, so a failure never leaves a half-written result,
/// and moved without replacement, so nothing existing is ever overwritten.
pub fn write_new(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = match std::fs::create_dir_all(dir).and_then(|_| probe_writable(dir)) {
        Ok(()) => dir.to_path_buf(),
        Err(_) => {
            let fb = fallback_dir();
            std::fs::create_dir_all(&fb).with_context(|| format!("can't create {}", fb.display()))?;
            fb
        }
    };
    let tmp = dir.join(format!(".shiftdrop-{}-{}.partial", std::process::id(), nanos()));
    std::fs::write(&tmp, bytes).with_context(|| format!("can't write to {}", dir.display()))?;
    for _ in 0..1000 {
        let dest = unique(&dir, stem, ext);
        let moved = unsafe { MoveFileExW(&long_path(&tmp), &long_path(&dest), MOVEFILE_WRITE_THROUGH) };
        match moved {
            Ok(()) => return Ok(dest),
            // Someone took the name between unique() and the move; pick the next one.
            Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => continue,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e).context("can't save the result");
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    anyhow::bail!("couldn't find a free file name in {}", dir.display())
}

/// Windows APIs need the `\\?\` form for paths over 260 characters
/// (Rust's own file functions add it automatically; direct API calls don't).
pub fn long_path(p: &Path) -> HSTRING {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy();
    if s.starts_with(r"\\?\") {
        HSTRING::from(s.as_ref())
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        HSTRING::from(format!(r"\\?\UNC\{unc}"))
    } else {
        HSTRING::from(format!(r"\\?\{s}"))
    }
}

fn probe_writable(dir: &Path) -> std::io::Result<()> {
    let p = dir.join(format!(".shiftdrop-probe-{}", nanos()));
    std::fs::write(&p, b"")?;
    std::fs::remove_file(p)
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shiftdrop-test-{name}-{}", nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn numbers_instead_of_overwriting() {
        let d = tmpdir("naming");
        let a = write_new(&d, "Page 001", "jpg", b"one").unwrap();
        let b = write_new(&d, "Page 001", "jpg", b"two").unwrap();
        let c = write_new(&d, "Page 001", "jpg", b"three").unwrap();
        assert_eq!(a.file_name().unwrap(), "Page 001.jpg");
        assert_eq!(b.file_name().unwrap(), "Page 001 (2).jpg");
        assert_eq!(c.file_name().unwrap(), "Page 001 (3).jpg");
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        // no temp files left behind
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 3);
    }

    #[test]
    fn beside_uses_source_folder() {
        let src = Path::new(r"C:\photos\a.png");
        assert_eq!(target_dir(src, &OutputDir::Beside), PathBuf::from(r"C:\photos"));
        assert_eq!(target_dir(src, &OutputDir::Folder(PathBuf::from(r"D:\out"))), PathBuf::from(r"D:\out"));
    }
}
