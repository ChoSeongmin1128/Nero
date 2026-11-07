use crate::{
    ai_chat_desktop::AiDesktop, meeting_context::MeetingContextStore, notion_context::NotionSource,
    require_main, DesktopState,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};

const MAX_BYTES: u64 = 256 * 1024;
const MAX_TERMS: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub term: String,
    pub source_id: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub revision: u64,
    pub entries: Vec<Entry>,
}
impl Snapshot {
    #[cfg(test)]
    pub fn terms(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.term.clone())
            .collect()
    }
    pub fn sha256(&self) -> Result<String, String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| invalid())?)
        ))
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    term: String,
    source_id: String,
    source_title: String,
    kind: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    snapshot: Snapshot,
    candidates: Vec<Candidate>,
    unavailable_sources: usize,
    reusable: Vec<Reusable>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reusable {
    term: String,
    source_id: String,
    meeting_title: String,
}

fn invalid() -> String {
    "회의 용어집을 확인하지 못했습니다.".into()
}
fn valid_term(term: &str) -> bool {
    !term.is_empty()
        && term == term.trim()
        && term.chars().count() <= 100
        && !term.chars().any(char::is_control)
}
fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.entries.len() > MAX_TERMS {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for entry in &snapshot.entries {
        let parts: Vec<_> = entry.source_id.split(':').collect();
        if !valid_term(&entry.term)
            || !seen.insert(entry.term.to_lowercase())
            || parts.len() != 3
            || parts[0] != "notion"
            || crate::notion_context::normalize_page_id(parts[1])
                .ok()
                .as_deref()
                != Some(parts[1])
            || parts[2].len() != 64
            || !parts[2]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn path(root: &Path, meeting_id: &str) -> Result<PathBuf, String> {
    if !uuid::Uuid::parse_str(meeting_id).is_ok_and(|id| id.to_string() == meeting_id) {
        return Err(invalid());
    }
    let path = root
        .join("domain-glossary")
        .join(format!("{meeting_id}.json"));
    crate::path_safety::safe_ancestors(&path).map_err(|_| invalid())?;
    Ok(path)
}
pub fn delete(root: &Path, meeting_id: &str) -> Result<(), String> {
    let path = path(root, meeting_id)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(invalid()),
    }
}
pub fn load_snapshot(root: &Path, meeting_id: &str) -> Result<Snapshot, String> {
    let path = path(root, meeting_id)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Snapshot::default()),
        Err(_) => return Err(invalid()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| invalid())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    validate(&snapshot)?;
    Ok(snapshot)
}
fn save(
    root: &Path,
    meeting_id: &str,
    expected: u64,
    entries: Vec<Entry>,
    allowed: &BTreeSet<String>,
) -> Result<Snapshot, String> {
    let path = path(root, meeting_id)?;
    let directory = path.parent().ok_or_else(invalid)?;
    fs::create_dir_all(directory).map_err(|_| invalid())?;
    crate::path_safety::safe_ancestors(&path).map_err(|_| invalid())?;
    let lock_path = directory.join("glossary.lock");
    crate::path_safety::safe_ancestors(&lock_path).map_err(|_| invalid())?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(lock_path)
        .map_err(|_| invalid())?;
    lock.lock_exclusive().map_err(|_| invalid())?;
    let before = load_snapshot(root, meeting_id)?;
    if before.revision != expected {
        return Err("용어집이 변경되었습니다. 다시 불러와 주세요.".into());
    }
    // Existing approved entries remain editable after their source is refreshed or removed.
    let previous: BTreeSet<_> = before
        .entries
        .iter()
        .map(|entry| entry.source_id.clone())
        .collect();
    if entries
        .iter()
        .any(|entry| !allowed.contains(&entry.source_id) && !previous.contains(&entry.source_id))
    {
        return Err("회의 자료가 변경되었습니다. 다시 불러와 주세요.".into());
    }
    let result = Snapshot {
        revision: expected.checked_add(1).ok_or_else(invalid)?,
        entries,
    };
    validate(&result)?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| invalid())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| invalid())?;
    }
    serde_json::to_writer(&mut file, &result).map_err(|_| invalid())?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| invalid())?;
    crate::path_safety::safe_ancestors(&path).map_err(|_| invalid())?;
    file.persist(path).map_err(|_| invalid())?;
    Ok(result)
}
fn candidates(source: &NotionSource) -> Vec<Candidate> {
    let mut result = Vec::new();
    let title = source.title.clone().unwrap_or_else(|| source.url.clone());
    let mut add = |term: &str, kind| {
        let term = term.trim();
        if valid_term(term) && result.len() < MAX_TERMS {
            result.push(Candidate {
                term: term.into(),
                source_id: source.source_id.clone(),
                source_title: title.clone(),
                kind,
            });
        }
    };
    if let Some(value) = &source.title {
        add(value, "title");
    }
    let mut fence = None;
    for line in source.body.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            let marker = &line[..3];
            match fence {
                None => fence = Some(marker),
                Some(open) if open == marker => fence = None,
                _ => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
            add(&line[hashes + 1..], "heading");
        }
        let mut parts = line.split('`');
        while parts.next().is_some() {
            let Some(code) = parts.next() else { break };
            if parts.clone().next().is_none() {
                break;
            }
            add(code, "code");
        }
    }
    result
}
fn require_meeting(state: &DesktopState, id: &str) -> Result<(), String> {
    if state
        .core
        .list_meetings()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|m| m.id == id && m.deleted_at.is_none())
    {
        Ok(())
    } else {
        Err("회의를 찾을 수 없습니다.".into())
    }
}
fn reusable_entries(
    root: &Path,
    meeting_id: &str,
    meetings: &[nero_core::Meeting],
    pages: &BTreeSet<String>,
    notion: &crate::notion_context::NotionContextService,
) -> Vec<Reusable> {
    let mut result = Vec::new();
    let mut verified = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for meeting in meetings
        .iter()
        .filter(|m| m.id != meeting_id && m.deleted_at.is_none())
    {
        let Ok(snapshot) = load_snapshot(root, &meeting.id) else {
            continue;
        };
        for entry in snapshot.entries {
            let Some(page) = entry.source_id.split(':').nth(1) else {
                continue;
            };
            if !pages.contains(page)
                || !seen.insert((entry.source_id.clone(), entry.term.to_lowercase()))
            {
                continue;
            }
            if !verified.contains(&entry.source_id) {
                if notion.read_sources(vec![entry.source_id.clone()]).is_err() {
                    continue;
                }
                verified.insert(entry.source_id.clone());
            }
            result.push(Reusable {
                term: entry.term,
                source_id: entry.source_id,
                meeting_title: meeting.title.clone(),
            });
            if result.len() >= MAX_TERMS {
                return result;
            }
        }
    }
    result
}
// A term picked before the meeting's source was refreshed names the earlier revision of a
// page that is still attached. It stays allowed while that revision can be read, so the
// edits being made when the refresh happened can still be saved.
fn earlier_revisions(
    entries: &[Entry],
    allowed: &BTreeSet<String>,
    pages: &BTreeSet<String>,
    readable: impl Fn(&str) -> bool,
) -> Vec<String> {
    entries
        .iter()
        .map(|entry| &entry.source_id)
        .filter(|id| !allowed.contains(*id))
        .filter(|id| {
            id.split(':')
                .nth(1)
                .is_some_and(|page| pages.contains(page))
        })
        .filter(|id| readable(id))
        .cloned()
        .collect()
}
// The glossary screen and agent requests share these; they block, so screens run them
// off the async runtime.
pub(crate) fn view(app: &AppHandle, meeting_id: &str) -> Result<View, String> {
    let state = app.state::<DesktopState>();
    let _lease = state.lifecycle.begin_transition()?;
    require_meeting(&state, meeting_id)?;
    let root = state.processing.root.clone();
    let meetings = state.core.list_meetings().map_err(|e| e.to_string())?;
    let notion = app.state::<AiDesktop>().notion()?;
    let (references, snapshot) =
        crate::meeting_context_desktop::with_meeting(app, meeting_id, || {
            Ok((
                MeetingContextStore::new(root.clone())?.list(meeting_id)?,
                load_snapshot(&root, meeting_id)?,
            ))
        })?;
    let pages = references
        .sources
        .iter()
        .map(|source| source.page_id.clone())
        .collect();
    let reusable = reusable_entries(&root, meeting_id, &meetings, &pages, &notion);
    let mut all = Vec::new();
    let mut unavailable_sources = 0;
    for reference in &references.sources {
        match notion.read_sources(vec![reference.source_id.clone()]) {
            Ok(bundle) => {
                for source in bundle.sources {
                    all.extend(candidates(&source));
                }
            }
            Err(_) => unavailable_sources += 1,
        }
    }
    let mut seen = BTreeSet::new();
    all.retain(|candidate| {
        seen.insert((candidate.source_id.clone(), candidate.term.to_lowercase()))
    });
    crate::meeting_context_desktop::with_meeting(app, meeting_id, || {
        if MeetingContextStore::new(root.clone())?.list(meeting_id)? != references
            || load_snapshot(&root, meeting_id)? != snapshot
        {
            return Err("회의 자료 또는 용어집이 변경되었습니다. 다시 불러와 주세요.".into());
        }
        Ok(View {
            snapshot,
            candidates: all,
            unavailable_sources,
            reusable,
        })
    })
}
pub(crate) fn save_entries(
    app: &AppHandle,
    meeting_id: &str,
    expected_revision: u64,
    entries: Vec<Entry>,
) -> Result<Snapshot, String> {
    let state = app.state::<DesktopState>();
    let _lease = state.lifecycle.begin_transition()?;
    require_meeting(&state, meeting_id)?;
    let root = state.processing.root.clone();
    let meetings = state.core.list_meetings().map_err(|e| e.to_string())?;
    let notion = app.state::<AiDesktop>().notion()?;
    let references = crate::meeting_context_desktop::with_meeting(app, meeting_id, || {
        MeetingContextStore::new(root.clone())?.list(meeting_id)
    })?;
    let pages = references
        .sources
        .iter()
        .map(|s| s.page_id.clone())
        .collect();
    let mut allowed: BTreeSet<_> = references
        .sources
        .iter()
        .map(|s| s.source_id.clone())
        .collect();
    allowed.extend(
        reusable_entries(&root, meeting_id, &meetings, &pages, &notion)
            .into_iter()
            .map(|e| e.source_id),
    );
    let earlier = earlier_revisions(&entries, &allowed, &pages, |id| {
        notion.read_sources(vec![id.to_owned()]).is_ok()
    });
    allowed.extend(earlier);
    crate::meeting_context_desktop::with_meeting(app, meeting_id, || {
        if MeetingContextStore::new(root.clone())?.list(meeting_id)? != references {
            return Err("회의 자료가 변경되었습니다. 다시 불러와 주세요.".into());
        }
        save(&root, meeting_id, expected_revision, entries, &allowed)
    })
}
#[tauri::command]
pub async fn domain_glossary_view(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
) -> Result<View, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || view(&app, &meeting_id))
        .await
        .map_err(|_| invalid())?
}
#[tauri::command]
pub async fn domain_glossary_save(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
    expected_revision: u64,
    entries: Vec<Entry>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        save_entries(&app, &meeting_id, expected_revision, entries)
    })
    .await
    .map_err(|_| invalid())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> NotionSource {
        NotionSource {
            source_id: format!(
                "notion:b0000000-0000-4000-8000-000000000001:{}",
                "a".repeat(64)
            ),
            page_id: "b0000000-0000-4000-8000-000000000001".into(),
            url: "https://notion.so/a".into(),
            title: Some("제품명".into()),
            last_edited_at: None,
            fetched_at: 0,
            sha256: String::new(),
            body: "# 문서 제목\n본문 일반 단어와 `API`\n```\n# 코드 속 제목\n`숨김`\n```".into(),
            truncated: false,
            unknown_block_count: None,
        }
    }
    #[test]
    fn extracts_only_explicit_literals() {
        let c = candidates(&source());
        assert_eq!(
            c.iter().map(|v| v.term.as_str()).collect::<Vec<_>>(),
            vec!["제품명", "문서 제목", "API"]
        );
    }
    #[test]
    fn saves_reviewed_terms_with_provenance_and_detects_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let source = source();
        let allowed = BTreeSet::from([source.source_id.clone()]);
        let entries = vec![Entry {
            term: "편집된 제품명".into(),
            source_id: source.source_id,
        }];
        let snapshot = save(&root, meeting, 0, entries.clone(), &allowed).unwrap();
        assert_eq!(snapshot.terms(), vec!["편집된 제품명"]);
        assert_eq!(load_snapshot(&root, meeting).unwrap(), snapshot);
        assert!(save(&root, meeting, 0, entries.clone(), &allowed).is_err());
        assert!(save(
            &root,
            meeting,
            1,
            vec![entries[0].clone(), entries[0].clone()],
            &allowed
        )
        .is_err());
        assert!(save(&root, meeting, 1, Vec::new(), &BTreeSet::new())
            .unwrap()
            .entries
            .is_empty());
    }
    #[test]
    fn terms_picked_before_a_source_refresh_can_still_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let page = "b0000000-0000-4000-8000-000000000001";
        let earlier = format!("notion:{page}:{}", "a".repeat(64));
        let refreshed = format!("notion:{page}:{}", "b".repeat(64));
        let other = format!(
            "notion:b0000000-0000-4000-8000-000000000002:{}",
            "c".repeat(64)
        );
        let allowed = BTreeSet::from([refreshed.clone()]);
        let pages = BTreeSet::from([page.to_string()]);
        let entry = |term: &str, source_id: &str| Entry {
            term: term.into(),
            source_id: source_id.into(),
        };
        let entries = vec![entry("제품명", &earlier), entry("새 용어", &refreshed)];
        let mut extended = allowed.clone();
        extended.extend(earlier_revisions(&entries, &allowed, &pages, |_| true));
        assert_eq!(
            save(&root, meeting, 0, entries, &extended)
                .unwrap()
                .revision,
            1
        );
        // A detached page or an unreadable revision stays refused.
        let detached = vec![entry("다른 용어", &other)];
        assert!(earlier_revisions(&detached, &allowed, &pages, |_| true).is_empty());
        let unreadable = vec![entry("제품명", &earlier)];
        assert!(earlier_revisions(&unreadable, &allowed, &pages, |_| false).is_empty());
        assert!(save(&root, meeting, 1, detached, &allowed).is_err());
    }
}
