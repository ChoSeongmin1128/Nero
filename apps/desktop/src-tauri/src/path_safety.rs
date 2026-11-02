use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) fn safe_ancestors(path: &Path) -> std::io::Result<()> {
    if !path.is_absolute() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                let blocked = meta.file_type().is_symlink();
                #[cfg(windows)]
                let blocked = {
                    use std::os::windows::fs::MetadataExt;
                    blocked || meta.file_attributes() & 0x400 != 0
                };
                if blocked {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod safe_ancestor_tests {
    use super::*;

    #[test]
    fn accepts_canonical_absolute_path_and_missing_descendant() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        #[cfg(windows)]
        assert!(root.to_string_lossy().starts_with(r"\\?\"));
        assert!(safe_ancestors(&root.join("missing").join("file")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join("link")).unwrap();
        assert!(safe_ancestors(&root.join("link").join("missing")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_junction_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir(&target).unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(safe_ancestors(&link.join("missing")).is_err());
    }
}
