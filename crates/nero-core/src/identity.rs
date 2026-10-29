use serde::Deserialize;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const APP_IDENTIFIER: &str = "io.github.choseongmin1128.nero.desktop";

const LEGACY_IDENTIFIER: &str = "io.github.choseongmin1128.grove.desktop";
const RECEIPT: &str = ".nero-identity-migration.json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    from_identifier: String,
    to_identifier: String,
    source_data: String,
    destination_data: String,
    status: String,
}

pub fn legacy_path(root: &Path) -> bool {
    root.file_name()
        .is_some_and(|name| name == LEGACY_IDENTIFIER)
}

pub fn migrated_data_dir(source: &Path) -> Option<PathBuf> {
    if !source.is_absolute()
        || !legacy_path(source)
        || source
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || fs::symlink_metadata(source).is_ok()
        || source.try_exists().ok()?
    {
        return None;
    }
    let destination = source.with_file_name(APP_IDENTIFIER);
    let metadata = fs::symlink_metadata(&destination).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let path = destination.join(RECEIPT);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let value: Receipt = serde_json::from_slice(&bytes).ok()?;
    if value.schema_version != 1
        || value.from_identifier != LEGACY_IDENTIFIER
        || value.to_identifier != APP_IDENTIFIER
        || value.source_data != source.to_str()?
        || value.destination_data != destination.to_str()?
        || value.status != "completed"
    {
        return None;
    }
    Some(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redirect_requires_missing_source_and_matching_completed_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join(LEGACY_IDENTIFIER);
        let target = source.with_file_name(APP_IDENTIFIER);
        assert!(migrated_data_dir(&source).is_none());
        assert!(!target.exists());
        fs::create_dir(&target).unwrap();
        let receipt = json!({"schemaVersion":1,"fromIdentifier":LEGACY_IDENTIFIER,
            "toIdentifier":APP_IDENTIFIER,"sourceData":source,
            "destinationData":target,"status":"completed"});
        fs::write(target.join(RECEIPT), receipt.to_string()).unwrap();
        assert_eq!(migrated_data_dir(&source), Some(target.clone()));
        for (key, value) in [
            ("status", json!("pending")),
            ("sourceData", json!("/other")),
            ("destinationData", json!("/other")),
            ("fromIdentifier", json!("other")),
            ("toIdentifier", json!("other")),
            ("schemaVersion", json!(2)),
            ("extra", json!(true)),
        ] {
            let mut invalid = receipt.clone();
            invalid[key] = value;
            fs::write(target.join(RECEIPT), invalid.to_string()).unwrap();
            assert!(migrated_data_dir(&source).is_none(), "{key}");
        }
        fs::write(target.join(RECEIPT), receipt.to_string()).unwrap();
        fs::create_dir(&source).unwrap();
        assert!(migrated_data_dir(&source).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn linked_receipt_or_source_never_redirects() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join(LEGACY_IDENTIFIER);
        let target = source.with_file_name(APP_IDENTIFIER);
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(temp.path().join("absent"), target.join(RECEIPT)).unwrap();
        assert!(migrated_data_dir(&source).is_none());
        std::os::unix::fs::symlink(temp.path().join("absent"), &source).unwrap();
        assert!(migrated_data_dir(&source).is_none());
    }
}
