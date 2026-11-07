use std::io;
use std::path::Path;

pub fn check(path: &Path, required: u64) -> Result<(), String> {
    let available = fs2::available_space(path).map_err(|_| {
        "저장 공간을 확인하지 못했습니다. 저장소를 확인한 뒤 다시 시도해 주세요.".to_string()
    })?;
    compare(available, required)
}

fn compare(available: u64, required: u64) -> Result<(), String> {
    match nero_core::disk_space::shortfall(available, required) {
        Some(amount) => Err(format!(
            "저장 공간이 부족합니다. 약 {amount}를 더 확보한 뒤 다시 시도해 주세요."
        )),
        None => Ok(()),
    }
}

pub fn write_error(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::StorageFull {
        "다운로드 중 저장 공간이 부족해졌습니다. 공간을 확보한 뒤 이어서 다운로드해 주세요.".into()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn space_boundary_and_mid_download_error_are_actionable() {
        assert!(compare(20, 20).is_ok());
        assert!(compare(20, 0).is_ok());
        assert!(compare(20, 21).unwrap_err().contains("1 MB"));
        assert!(compare(0, 1_000_000_001).unwrap_err().contains("1.1 GB"));
        assert!(write_error(io::Error::from(io::ErrorKind::StorageFull)).contains("이어서"));
    }
}
