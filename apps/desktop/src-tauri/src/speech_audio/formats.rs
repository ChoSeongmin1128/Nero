use nero_core::AUDIO_IMPORT_EXTENSIONS as IMPORT_EXTENSIONS;
use std::path::Path;

pub(crate) fn import_extensions() -> &'static [&'static str] {
    IMPORT_EXTENSIONS
}

pub(crate) fn validate_import(source: &Path) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if IMPORT_EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
    {
        Ok(())
    } else {
        Err("지원하지 않는 음성 파일입니다. WAV, M4A, MP3, AAC, AIFF, CAF, FLAC, OGG 파일을 선택해 주세요.".into())
    }
}

pub(super) fn is_wave(source: &Path) -> bool {
    source
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case("wav"))
}

pub(super) fn portable_on_macos(source: &Path) -> bool {
    source.extension().is_some_and(|value| {
        value.eq_ignore_ascii_case("flac") || value.eq_ignore_ascii_case("ogg")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_and_import_share_case_insensitive_supported_formats() {
        for extension in import_extensions() {
            assert!(
                validate_import(Path::new(&format!("녹음.{}", extension.to_uppercase()))).is_ok()
            );
        }
        for name in ["audio", "audio.exe", "audio.mp4", "audio.mp3.exe"] {
            assert!(validate_import(Path::new(name)).is_err());
        }
    }
}
