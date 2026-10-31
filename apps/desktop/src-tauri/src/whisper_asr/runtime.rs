//! Installed package trust and target identities. Never probes a device or loads a model.
use super::*;
use std::collections::BTreeMap;

pub(super) struct Runtime {
    pub(super) cpu: PathBuf,
    pub(super) vulkan: Option<PathBuf>,
    pub(super) probe: Option<PathBuf>,
    pub(super) identity: String,
    pub(super) receipt_sha: Option<String>,
    files: BTreeMap<PathBuf, String>,
    windows_root: Option<PathBuf>,
}

fn inventory(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| failure())?;
    if meta.is_symlink() {
        return Err(failure());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(failure());
        }
    }
    if meta.is_file() {
        files.push(path.to_path_buf());
    } else if meta.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|_| failure())? {
            inventory(&entry.map_err(|_| failure())?.path(), files)?;
        }
    } else {
        return Err(failure());
    }
    Ok(())
}

fn plain_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() || relative.contains(['\\', ':']) {
        return Err(failure());
    }
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(failure());
        }
        path.push(component);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| failure())?;
        if meta.is_symlink() {
            return Err(failure());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                // FILE_ATTRIBUTE_REPARSE_POINT
                return Err(failure());
            }
        }
    }
    plain_file(&path)?;
    Ok(path)
}

impl Runtime {
    pub(super) fn open(cli: PathBuf) -> Result<Self, String> {
        let spec: Value = serde_json::from_str(RUNTIME).map_err(|_| failure())?;
        let mut result = Self {
            cpu: cli.clone(),
            vulkan: None,
            probe: None,
            identity: spec["identity"].as_str().ok_or_else(failure)?.into(),
            receipt_sha: None,
            files: BTreeMap::new(),
            windows_root: None,
        };
        // Preserve the existing signed Metal package contract. Developer ID changes
        // Mach-O bytes after its build receipt; the live executable hash is still
        // frozen for the session and checked after every window.
        if !cfg!(windows) {
            result.files.insert(cli.clone(), file_hash(&cli)?);
            return Ok(result);
        }
        let root = cli.parent().and_then(Path::parent).ok_or_else(failure)?;
        result.windows_root = Some(root.to_path_buf());
        if cli != root.join("bin/whisper-cli.exe") {
            return Err(failure());
        }
        let receipt_path = plain_path(root, "runtime.json")?;
        let raw = read(&receipt_path, 1024 * 1024)?;
        let receipt: Value = serde_json::from_slice(&raw).map_err(|_| failure())?;
        if receipt["runtime"] != spec
            || receipt["target"] != "windows-x64"
            || receipt["sourceArchiveHashesVerified"] != true
        {
            return Err("Whisper 실행 패키지를 다시 준비해 주세요.".into());
        }
        let entries = receipt["files"].as_array().ok_or_else(failure)?;
        if entries.is_empty() || entries.len() > 256 {
            return Err(failure());
        }
        for entry in entries {
            let path = plain_path(root, entry["path"].as_str().ok_or_else(failure)?)?;
            let hash = entry["sha256"].as_str().ok_or_else(failure)?;
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || plain_file(&path)?.metadata().map_err(|_| failure())?.len()
                    != entry["bytes"].as_u64().ok_or_else(failure)?
                || result.files.insert(path, hash.into()).is_some()
            {
                return Err(failure());
            }
        }
        let vulkan = root.join("vulkan/bin/whisper-cli.exe");
        let probe = root.join("bin/nero-vulkan-probe.exe");
        for required in [
            &cli,
            &vulkan,
            &probe,
            &root.join("share/licenses/whisper.cpp/LICENSE"),
            &root.join("share/licenses/vulkan-sdk/LICENSE.txt"),
        ] {
            if !result.files.contains_key(required) {
                return Err(failure());
            }
        }
        result.identity = spec["targets"]["windows-x64"]["identity"]
            .as_str()
            .ok_or_else(failure)?
            .into();
        result.receipt_sha = Some(format!("{:x}", Sha256::digest(&raw)));
        result
            .files
            .insert(receipt_path, result.receipt_sha.clone().unwrap());
        result.vulkan = Some(vulkan);
        result.probe = Some(probe);
        result.verify()?;
        Ok(result)
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        if let Some(root) = &self.windows_root {
            let mut actual = Vec::new();
            inventory(root, &mut actual)?;
            actual.sort();
            if actual != self.files.keys().cloned().collect::<Vec<_>>() {
                return Err(failure());
            }
        }
        for (path, expected) in &self.files {
            if file_hash(path)? != *expected {
                return Err(
                    "Whisper 실행 패키지가 처리 중 변경됐습니다. 원본은 보관했습니다.".into(),
                );
            }
        }
        Ok(())
    }

    pub(super) fn evidence(
        &self,
        executable: &Path,
        stderr: &Path,
        probe: &Path,
    ) -> Result<Value, String> {
        Ok(json!({"runtimeIdentity":self.identity,
            "runtimeReceiptSha256":self.receipt_sha.as_ref().ok_or_else(failure)?,
            "executableSha256":self.files.get(executable).ok_or_else(failure)?,
            "stderrSha256":file_hash(stderr)?, "probeResultSha256":file_hash(probe)?}))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> PathBuf {
        let names = [
            "bin/whisper-cli.exe",
            "bin/nero-vulkan-probe.exe",
            "vulkan/bin/whisper-cli.exe",
            "share/licenses/whisper.cpp/LICENSE",
            "share/licenses/vulkan-sdk/LICENSE.txt",
        ];
        let mut files = Vec::new();
        for name in names {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"synthetic package fixture").unwrap();
            files.push(json!({"path":name,"bytes":25,"sha256":file_hash(&path).unwrap()}));
        }
        let spec: Value = serde_json::from_str(RUNTIME).unwrap();
        save(
            &root.join("runtime.json"),
            &json!({"runtime":spec,"target":"windows-x64",
            "sourceArchiveHashesVerified":true,"files":files}),
        )
        .unwrap();
        root.join("bin/whisper-cli.exe")
    }

    #[test]
    fn frozen_runtime_rejects_unlisted_dlls_and_replaced_probe_without_loading_either() {
        let temp = tempfile::tempdir().unwrap();
        let cli = fixture(temp.path());
        let runtime = Runtime::open(cli).unwrap();
        let extra = temp.path().join("bin/ggml-cpu-unlisted.dll");
        std::fs::write(&extra, b"unlisted").unwrap();
        assert!(runtime.verify().is_err());
        std::fs::remove_file(extra).unwrap();
        runtime.verify().unwrap();
        std::fs::write(temp.path().join("bin/nero-vulkan-probe.exe"), b"changed").unwrap();
        assert!(runtime.verify().is_err());
    }
}
