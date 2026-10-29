use nero_core::identity::migrated_data_dir;
use std::io;
use std::path::Path;

const LEGACY_IDENTIFIER: &str = "io.github.choseongmin1128.grove.desktop";
#[cfg(test)]
const IDENTIFIER: &str = nero_core::identity::APP_IDENTIFIER;
const RECEIPT: &str = ".nero-identity-migration.json";

fn invalid() -> io::Error {
    io::Error::other("Nero 데이터 이전이 필요하거나 완료되지 않았습니다. 기존 데이터를 보존한 채 공식 설치 도구로 이전을 완료해 주세요.")
}

pub fn validate_startup(root: &Path) -> io::Result<()> {
    let legacy = root.with_file_name(LEGACY_IDENTIFIER);
    if legacy.try_exists()? || legacy.is_symlink() || root.is_symlink() {
        return Err(invalid());
    }
    if root.try_exists()? && !root.is_dir() {
        return Err(invalid());
    }
    let receipt = root.join(RECEIPT);
    match std::fs::symlink_metadata(&receipt) {
        Ok(_) => {
            if migrated_data_dir(&legacy).as_deref() != Some(root) {
                return Err(invalid());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_old_library_never_creates_an_empty_new_library() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(LEGACY_IDENTIFIER);
        let root = temp.path().join(IDENTIFIER);
        std::fs::create_dir(&old).unwrap();
        assert!(validate_startup(&root).is_err());
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        assert!(validate_startup(&root).is_err());
    }

    #[test]
    fn fresh_install_and_valid_completed_migration_are_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(IDENTIFIER);
        assert!(validate_startup(&root).is_ok());
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        let receipt = json!({"schemaVersion":1,"fromIdentifier":LEGACY_IDENTIFIER,
            "toIdentifier":IDENTIFIER,"sourceData":temp.path().join(LEGACY_IDENTIFIER),
            "destinationData":root,"status":"completed"});
        std::fs::write(root.join(RECEIPT), receipt.to_string()).unwrap();
        assert!(validate_startup(&root).is_ok());
        for (key, value) in [
            ("status", "pending"),
            ("destinationData", "/other"),
            ("fromIdentifier", "other"),
        ] {
            let mut invalid_receipt = receipt.clone();
            invalid_receipt[key] = json!(value);
            std::fs::write(root.join(RECEIPT), invalid_receipt.to_string()).unwrap();
            assert!(validate_startup(&root).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn linked_library_or_receipt_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(IDENTIFIER);
        std::os::unix::fs::symlink(temp.path().join("missing"), &root).unwrap();
        assert!(validate_startup(&root).is_err());
    }
}
