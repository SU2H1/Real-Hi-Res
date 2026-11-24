//! Path handling that matches Python's `os.path`, so the saved results of the
//! two versions are keyed by the very same path strings, and the file walk of
//! `cli.iter_audio_files()`.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// Folder that Fix moves originals into; never scanned.
pub const BACKUP_DIR_NAME: &str = "Real-Hi-Res backups";

/// `os.path.abspath()`: absolute and lexically normalised (".." and "." are
/// resolved without following symbolic links).
pub fn abspath(p: &str) -> String {
    #[cfg(windows)]
    {
        // Python uses GetFullPathNameW, as std::path::absolute does.
        std::path::absolute(p).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
    }
    #[cfg(not(windows))]
    {
        let joined = if p.starts_with('/') {
            p.to_string()
        } else {
            let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            if p.is_empty() {
                cwd
            } else {
                format!("{}/{}", cwd.trim_end_matches('/'), p)
            }
        };
        normpath(&joined)
    }
}

/// `posixpath.normpath()`.
#[cfg(not(windows))]
pub fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") {
        2
    } else if p.starts_with('/') {
        1
    } else {
        0
    };
    let mut parts: Vec<&str> = Vec::new();
    for comp in p.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && parts.is_empty()) || parts.last() == Some(&"..") {
            parts.push(comp);
        } else if !parts.is_empty() {
            parts.pop();
        }
    }
    let body = parts.join("/");
    let out = format!("{}{}", "/".repeat(initial), body);
    if out.is_empty() {
        ".".into()
    } else {
        out
    }
}

/// `os.path.relpath(path, start)` for absolute paths.
pub fn relpath(path: &str, start: &str) -> String {
    let a = abspath(path);
    let b = abspath(start);
    let pa: Vec<&str> = a.split(std::path::MAIN_SEPARATOR).filter(|s| !s.is_empty()).collect();
    let pb: Vec<&str> = b.split(std::path::MAIN_SEPARATOR).filter(|s| !s.is_empty()).collect();
    let common = pa.iter().zip(&pb).take_while(|(x, y)| x == y).count();
    let mut out: Vec<&str> = vec![".."; pb.len() - common];
    out.extend(&pa[common..]);
    if out.is_empty() {
        ".".into()
    } else {
        out.join(std::path::MAIN_SEPARATOR_STR)
    }
}

/// `os.path.splitext(name)[1]`: the extension, leading dots not counting.
pub fn extension(name: &str) -> String {
    let stem_start = name.len() - name.trim_start_matches('.').len();
    match name[stem_start..].rfind('.') {
        Some(i) => name[stem_start + i..].to_string(),
        None => String::new(),
    }
}

pub fn file_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn parent(p: &str) -> String {
    Path::new(p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn stem(p: &str) -> String {
    let name = file_name(p);
    let ext = extension(&name);
    name[..name.len() - ext.len()].to_string()
}

fn wanted(name: &str, exts: &BTreeSet<String>) -> bool {
    exts.contains(&extension(name).to_lowercase()) && !name.starts_with(".realhires-")
}

fn sorted_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect(),
        Err(_) => vec![],
    };
    v.sort();
    v
}

/// Call `f` for every audio file of one folder (sorted, folder by folder, as
/// `os.walk` lists them) or for the file itself.  `f` returns false to stop.
pub fn walk_audio_files(path: &str, recursive: bool, exts: &BTreeSet<String>, f: &mut dyn FnMut(String) -> bool) {
    let p = Path::new(path);
    if p.is_dir() {
        if recursive {
            walk_dir(p, exts, f);
        } else {
            for (name, full) in sorted_entries(p) {
                if full.is_file() && wanted(&name, exts) && !f(full.to_string_lossy().into_owned()) {
                    return;
                }
            }
        }
    } else if p.is_file() {
        f(path.to_string());
    }
}

fn walk_dir(dir: &Path, exts: &BTreeSet<String>, f: &mut dyn FnMut(String) -> bool) -> bool {
    let entries = sorted_entries(dir);
    let mut dirs = vec![];
    for (name, full) in entries {
        // os.walk: a symbolic link to a folder is listed but not entered
        let is_link = std::fs::symlink_metadata(&full).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if full.is_dir() {
            if !is_link && name != BACKUP_DIR_NAME {
                dirs.push(full);
            }
        } else if wanted(&name, exts) && !f(full.to_string_lossy().into_owned()) {
            return false;
        }
    }
    for d in dirs {
        if !walk_dir(&d, exts, f) {
            return false;
        }
    }
    true
}

pub fn iter_audio_files(path: &str, recursive: bool, exts: &BTreeSet<String>) -> Vec<String> {
    let mut out = vec![];
    walk_audio_files(path, recursive, exts, &mut |p| {
        out.push(p);
        true
    });
    out
}

/// The audio files of all `paths`, without duplicates; missing paths are
/// reported through `warn`.
pub fn collect(paths: &[String], recursive: bool, exts: &BTreeSet<String>, warn: &mut dyn FnMut(&str)) -> Vec<String> {
    let mut found = vec![];
    for p in paths {
        if Path::new(p).exists() {
            found.extend(iter_audio_files(p, recursive, exts));
        } else {
            warn(p);
        }
    }
    let mut seen = HashSet::new();
    found.into_iter().filter(|f| seen.insert(abspath(f))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_path_semantics() {
        assert_eq!(extension("a.FLAC"), ".FLAC");
        assert_eq!(extension(".flac"), "");
        assert_eq!(extension("a.b.wav"), ".wav");
        assert_eq!(extension("noext"), "");
        #[cfg(not(windows))]
        {
            assert_eq!(normpath("/a/b/../c/./d/"), "/a/c/d");
            assert_eq!(normpath("//a//b"), "//a/b");
            assert_eq!(normpath("/.."), "/");
            assert_eq!(relpath("/m/a/b.flac", "/m"), "a/b.flac");
        }
    }
}
