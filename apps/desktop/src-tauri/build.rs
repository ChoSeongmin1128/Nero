fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repository root");
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
