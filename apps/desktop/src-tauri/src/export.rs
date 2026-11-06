use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILENAME_BYTES: usize = 200;

pub fn validate_contents(contents: &str) -> Result<(), String> {
    if contents.len() > MAX_TEXT_BYTES {
        return Err("내보낼 내용이 너무 큽니다.".into());
    }
    Ok(())
}

fn allowed_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("txt") || extension.eq_ignore_ascii_case("md")
}

pub fn suggested_name(value: &str) -> Result<String, String> {
    let leaf = value.rsplit(['/', '\\']).next().unwrap_or_default();
    let (stem, extension) = leaf
        .rsplit_once('.')
        .ok_or("TXT 또는 Markdown 형식을 선택해 주세요.")?;
    if !allowed_extension(extension) {
        return Err("TXT 또는 Markdown 형식을 선택해 주세요.".into());
    }
    Ok(portable_name(stem, &extension.to_ascii_lowercase()))
}

// Path separators become `_`, so a title never names a folder.
pub fn file_name(stem: &str, extension: &str) -> String {
    portable_name(&stem.replace(['/', '\\'], "_"), extension)
}

fn portable_name(stem: &str, extension: &str) -> String {
    let budget = MAX_FILENAME_BYTES - extension.len() - 2;
    let mut clean = String::new();
    for character in stem.chars() {
        let character = if character.is_control() || "<>:\"|?*".contains(character) {
            '_'
        } else {
            character
        };
        if clean.len() + character.len_utf8() > budget {
            break;
        }
        clean.push(character);
    }
    let clean = clean.trim().trim_end_matches(['.', ' ']);
    let stem = if clean.is_empty() { "회의록" } else { clean };
    let reserved = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(reserved.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (reserved.len() == 4
            && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
            && matches!(reserved.as_bytes()[3], b'1'..=b'9'));
    format!("{}{stem}.{extension}", if reserved { "_" } else { "" })
}

fn validate_destination(path: &Path, allowed: impl Fn(&str) -> bool) -> Result<(), String> {
    if !path.is_absolute()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(allowed)
    {
        return Err("저장할 파일 이름과 형식을 확인해 주세요.".into());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("일반 파일을 선택해 주세요.".into());
        }
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(_) => return Err("저장할 파일을 확인하지 못했습니다.".into()),
    }
    Ok(())
}

fn persist_with(
    path: &Path,
    allowed: impl Fn(&str) -> bool + Copy,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), String> {
    validate_destination(path, allowed)?;
    let parent = path.parent().ok_or("저장 폴더를 선택해 주세요.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "저장 폴더에 파일을 만들지 못했습니다.")?;
    write(temporary.as_file_mut())
        .map_err(|_| "파일을 저장하지 못했습니다. 기존 파일은 변경하지 않았습니다.")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "파일 저장을 확인하지 못했습니다. 기존 파일은 변경하지 않았습니다.")?;
    validate_destination(path, allowed)?;
    temporary
        .persist(path)
        .map_err(|_| "파일을 교체하지 못했습니다. 기존 파일은 변경하지 않았습니다.")?;
    Ok(())
}

pub fn write_text(path: &Path, contents: &str) -> Result<(), String> {
    validate_contents(contents)?;
    persist_with(path, allowed_extension, |file| {
        file.write_all(contents.as_bytes())
    })
}

pub fn copy_file(source: &Path, path: &Path) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .ok_or("원본 파일 형식을 확인하지 못했습니다.")?;
    let mut input = File::open(source).map_err(|_| "원본 파일을 열지 못했습니다.")?;
    persist_with(
        path,
        |chosen| chosen.eq_ignore_ascii_case(extension),
        |file| io::copy(&mut input, file).map(|_| ()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_filename_is_portable_and_never_contains_a_directory() {
        assert_eq!(
            suggested_name("../../notes/회의록.md").unwrap(),
            "회의록.md"
        );
        assert_eq!(suggested_name("C:\\meeting\\CON.TXT").unwrap(), "_CON.txt");
        assert_eq!(
            suggested_name("meeting: agenda?.txt").unwrap(),
            "meeting_ agenda_.txt"
        );
        let name = suggested_name(&format!("{}.md", "회의".repeat(500))).unwrap();
        assert!(name.len() <= MAX_FILENAME_BYTES);
        assert!(name.ends_with(".md"));
        assert!(suggested_name("notes.html").is_err());
        assert!(suggested_name("notes").is_err());
    }

    #[test]
    fn text_size_limit_counts_utf8_bytes() {
        assert!(validate_contents(&"a".repeat(MAX_TEXT_BYTES)).is_ok());
        assert!(validate_contents(&"a".repeat(MAX_TEXT_BYTES + 1)).is_err());
        assert!(validate_contents(&"한".repeat(MAX_TEXT_BYTES / 3 + 1)).is_err());
    }

    #[test]
    fn text_export_atomically_replaces_the_selected_file() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("meeting.md");
        fs::write(&target, "previous").unwrap();
        write_text(&target, "회의 기록\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "회의 기록\n");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_preserves_the_previous_file_and_cleans_temporary_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("meeting.txt");
        fs::write(&target, "previous").unwrap();
        assert!(persist_with(&target, allowed_extension, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected disk failure"))
        })
        .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(write_text(&directory.path().join("meeting.html"), "rejected").is_err());
        assert!(write_text(Path::new("meeting.txt"), "rejected").is_err());
    }

    #[test]
    fn recording_copy_keeps_the_source_format_and_a_title_never_names_a_folder() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("audio.wav");
        fs::write(&source, b"RIFF audio").unwrap();
        let target = directory
            .path()
            .join(file_name("주간/회의: 1차 2026-11-05", "wav"));
        assert_eq!(target.file_name().unwrap(), "주간_회의_ 1차 2026-11-05.wav");
        copy_file(&source, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"RIFF audio");
        assert!(copy_file(&source, &directory.path().join("recording.m4a")).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"RIFF audio");
    }

    #[cfg(unix)]
    #[test]
    fn exporting_to_a_symbolic_link_never_changes_its_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("original.txt");
        let link = directory.path().join("linked.md");
        fs::write(&target, "previous").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_text(&link, "replacement").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "previous");
    }
}
