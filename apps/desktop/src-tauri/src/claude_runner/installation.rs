//! Presence only: no signature subprocess, Claude execution or credential reads.
use std::{fs, io::ErrorKind, path::Path};

fn native_presence(path: &Path) -> &'static str {
    match fs::metadata(path) {
        Ok(value) if value.is_file() => "present",
        Ok(_) => "unknown",
        Err(error) if error.kind() == ErrorKind::NotFound => "missing",
        Err(_) => "unknown",
    }
}

pub(super) fn installation_state_at(root: &Path, home: Option<&Path>) -> &'static str {
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let native = home.map_or("unknown", |home| {
        native_presence(&home.join(".local/bin").join(name))
    });
    if native == "present" {
        return native;
    }
    match fs::symlink_metadata(root) {
        Ok(value) if value.file_type().is_dir() => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return native,
        _ => return "unknown",
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return native,
        Err(_) => return "unknown",
    };
    let mut uncertain = native == "unknown";
    for (index, entry) in entries.enumerate() {
        if index >= 128 {
            return "unknown";
        }
        let Ok(entry) = entry else { return "unknown" };
        let file_name = entry.file_name();
        let Some(version) = file_name.to_str() else {
            continue;
        };
        if semver::Version::parse(version).is_err() {
            continue;
        }
        // App-owned version directories and binaries must not be links.
        let Ok(kind) = entry.file_type() else {
            return "unknown";
        };
        if !kind.is_dir() {
            uncertain = true;
            continue;
        }
        match fs::symlink_metadata(entry.path().join(name)) {
            Ok(value) if value.file_type().is_file() => return "present",
            Ok(_) => uncertain = true,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => uncertain = true,
        }
    }
    if uncertain {
        "unknown"
    } else {
        "missing"
    }
}

pub fn installation_state(root: &Path) -> &'static str {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    installation_state_at(root, home.as_deref().map(Path::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_presence_never_claims_authentication_or_executes_a_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let root = temp.path().join("runtime");
        assert_eq!(installation_state_at(&root, Some(&home)), "missing");
        let name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let prepared = root.join("2.1.0");
        fs::create_dir_all(&prepared).unwrap();
        fs::write(prepared.join(name), b"not executable or signed").unwrap();
        assert_eq!(installation_state_at(&root, Some(&home)), "present");
        assert_eq!(
            installation_state_at(&temp.path().join("missing"), None),
            "unknown"
        );
    }

    #[test]
    fn a_broken_native_installation_is_not_reported_as_absent() {
        let temp = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        fs::create_dir_all(temp.path().join(".local/bin").join(name)).unwrap();
        assert_eq!(
            installation_state_at(&temp.path().join("missing"), Some(temp.path())),
            "unknown"
        );
    }
}
