use std::env;
use std::fmt::format;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const EXDEV: i32 = 18; // "Invalid cross-device link" on Linux and macOS

struct Trash {
    files: PathBuf,
    info: Option<PathBuf>, // None on macOS (no .trashinfo files)
}

impl Trash {
    fn locate() -> io::Result<Trash> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        if cfg!(target_os = "macos") {
            let files = home.join(".Trash");
            fs::create_dir_all(&files)?;
            return Ok(Trash { files, info: None });
        }
        let base = match env::var_os("XDG_DATA_HOME") {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => home.join(".local/share"),
        };
        let root = base.join("Trash");
        let files = root.join("files");
        let info = root.join("info");
        fs::create_dir_all(&files)?;
        fs::create_dir_all(&info)?;
        Ok(Trash { files, info: Some(info) })
    }

    fn unique_name(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.files.join(n).symlink_metadata().is_ok()
                || self
                    .info
                    .as_ref()
                    .is_some_and(|i| i.join(format!("{n}.trashinfo")).exists())
        };
        if !taken(base) {
            return base.to_string();
        }
        let mut i = 2u32;
        loop {
            let cand = format!("{base}.{i}");
            if !taken(&cand) {
                return cand;
            }
            i += 1;
        }
    }
}


fn iso_utc(secs: u64) -> String {
    // Howard Hinnant's civil-from-days algorithm.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_utc(secs)
}

fn encode_path(path: &Path) -> String {
    let mut s = String::new();
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{:02X}", b)),
        }
    }
    s
}

fn decode_path(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "refusing to remove '.' or '..'")
    })?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    Ok(fs::canonicalize(parent)?.join(name))
}

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    let ft = md.file_type();
    if ft.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dst)
    } else if ft.is_dir() {
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        fs::set_permissions(dst, md.permissions())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn remove_any(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy_recursive(src, dst)?;
            remove_any(src)
        }
        other => other,
    }
}


fn trash_one(trash: &Trash, target: &Path) -> io::Result<PathBuf> {
    let abs = absolute(target)?;
    let base = abs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".into());
    let name = trash.unique_name(&base);
    let dest = trash.files.join(&name);

    let info_path = trash.info.as_ref().map(|i| i.join(format!("{name}.trashinfo")));
    if let Some(ip) = &info_path {
        let body = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            encode_path(&abs),
            now_iso()
        );
        fs::write(ip, body)?;
    }
    if let Err(e) = move_path(target, &dest) {
        if let Some(ip) = &info_path {
            let _ = fs::remove_file(ip);
        }
        return Err(e);
    }
    Ok(dest)
}

struct Entry {
    name: String,
    date: String,
    original: PathBuf,
}

fn read_entries(trash: &Trash) -> io::Result<Vec<Entry>> {
    let info = trash.info.as_ref().ok_or_else(|| {
        io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform")
    })?;
    let mut out = Vec::new();
    for e in fs::read_dir(info)? {
        let path = e?.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(name) = fname.strip_suffix(".trashinfo") else { continue };
        let text = fs::read_to_string(&path).unwrap_or_default();
        let mut orig = None;
        let mut date = String::new();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Path=") {
                orig = Some(decode_path(v));
            } else if let Some(v) = line.strip_prefix("DeletionDate=") {
                date = v.to_string();
            }
        }
        if let Some(original) = orig {
            out.push(Entry { name: name.to_string(), date, original });
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(out)
}

fn list(trash: &Trash) -> io::Result<()> {
    for e in read_entries(trash)? {
        println!("{}  {}", e.date, e.original.display());
    }
    Ok(())
}

fn undo(trash: &Trash, verbose: bool) -> io::Result<()> {
    let entries = read_entries(trash)?;
    let Some(last) = entries.last() else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "trash is empty"));
    };
    if last.original.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("'{}' already exists; not overwriting", last.original.display()),
        ));
    }
    if let Some(parent) = last.original.parent() {
        fs::create_dir_all(parent)?;
    }
    move_path(&trash.files.join(&last.name), &last.original)?;
    if let Some(info) = &trash.info {
        let _ = fs::remove_file(info.join(format!("{}.trashinfo", last.name)));
    }
    if verbose {
        eprintln!("restored '{}'", last.original.display());
    }
    Ok(())
}


const USAGE: &str = "usage: del [-rRfdv] FILE...\n       del --list | --undo | --help | --version";

fn main() -> ExitCode {
    let mut recursive = false;
    let mut force = false;
    let mut verbose = false;
    let mut do_list = false;
    let mut do_undo = false;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut opts_done = false;

    for arg in env::args_os().skip(1) {
        let s = arg.to_string_lossy().into_owned();
        if opts_done || !s.starts_with('-') || s == "-" {
            files.push(PathBuf::from(arg));
            continue;
        }
        match s.as_str() {
            "--" => opts_done = true,
            "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!("del {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--list" => do_list = true,
            "--undo" => do_undo = true,
            "--recursive" => recursive = true,
            "--force" => force = true,
            "--verbose" => verbose = true,
            _ if s.starts_with("--") => {
                eprintln!("del: unknown option '{s}'\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => {
                for c in s.chars().skip(1) {
                    match c {
                        'r' | 'R' | 'd' => recursive = true,
                        'f' => force = true,
                        'v' => verbose = true,
                        'i' | 'I' => {} // accepted for rm muscle memory; ignored
                        _ => {
                            eprintln!("del: unknown option '-{c}'\n{USAGE}");
                            return ExitCode::from(2);
                        }
                    }
                }
            }
        }
    }

    let trash = match Trash::locate() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("del: cannot open trash: {e}");
            return ExitCode::FAILURE;
        }
    };

    if do_list {
        return report(list(&trash));
    }
    if do_undo {
        return report(undo(&trash, true));
    }
    if files.is_empty() {
        if force {
            return ExitCode::SUCCESS; // rm -f with no operands is not an error
        }
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let mut failed = false;
    for f in &files {
        match fs::symlink_metadata(f) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !force {
                    eprintln!("del: cannot remove '{}': No such file or directory", f.display());
                    failed = true;
                }
                continue;
            }
            Err(e) => {
                eprintln!("del: cannot remove '{}': {e}", f.display());
                failed = true;
                continue;
            }
            Ok(md) if md.is_dir() && !recursive => {
                eprintln!("del: cannot remove '{}': Is a directory (use -r)", f.display());
                failed = true;
                continue;
            }
            Ok(_) => {}
        }
        match trash_one(&trash, f) {
            Ok(dest) => {
                if verbose {
                    println!("trashed '{}' -> '{}'", f.display(), dest.display());
                }
            }
            Err(e) => {
                eprintln!("del: cannot remove '{}': {e}", f.display());
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn report(r: io::Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("del: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_epoch() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00");
        assert_eq!(iso_utc(1_000_000_000), "2001-09-09T01:46:40");
    }

    #[test]
    fn encode_decode_roundtrip() {
        let p = Path::new("/home/me/my files/100%.txt");
        let enc = encode_path(p);
        assert_eq!(enc, "/home/me/my%20files/100%25.txt");
        assert_eq!(decode_path(&enc), p);
    }

    #[test]
    fn decode_trailing_percent_is_safe() {
        assert_eq!(decode_path("/a%"), Path::new("/a%"));
        assert_eq!(decode_path("/a%4"), Path::new("/a%4"));
    }
}
