fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let root = std::path::Path::new(&manifest)
        .ancestors()
        .nth(3)
        .expect("repository root");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let speech_target = match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    };
    if let Some(target) = speech_target {
        for path in [
            "scripts/native-diarization/prepare.py",
            "scripts/native-diarization/runtime.json",
            "apps/desktop/.generated/native/diarization",
        ] {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
        let checked = std::process::Command::new(python)
            .arg(root.join("scripts/native-diarization/prepare.py"))
            .arg("--verify-staged")
            .arg(root.join("apps/desktop/.generated/native/diarization"))
            .arg("--target")
            .arg(target)
            .arg("--optional")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("Python is required to verify a packaged native speech runtime");
        assert!(
            checked.status.success(),
            "Native speech package verification failed: {}",
            String::from_utf8_lossy(&checked.stderr).trim()
        );
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for path in [
            "scripts/native-moss-harness/verify_worker_package.py",
            "scripts/native-moss-harness/apply-runtime-patch.py",
            "scripts/native-moss-harness/Package.swift",
            "scripts/native-moss-harness/Package.resolved",
            "scripts/native-moss-harness/Sources/MossHarness",
            "scripts/native-moss-harness/patches",
            "apps/desktop/.generated/native/moss",
        ] {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
        let checked = std::process::Command::new(python)
            .arg(root.join("scripts/native-moss-harness/verify_worker_package.py"))
            .arg("verify-staged")
            .arg(root.join("apps/desktop/.generated/native/moss"))
            .arg("--optional")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("Python is required to verify a packaged macOS worker");
        assert!(
            checked.status.success(),
            "MOSS worker package verification failed: {}",
            String::from_utf8_lossy(&checked.stderr).trim()
        );
    }
    tauri_build::build()
}
