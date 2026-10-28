use crate::engine::Engine;
use crate::store;
use crate::{CoreError, Result, Transcript, TranscriptSegment, TranscriptVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const MAX_SUMMARY_CITATION_REFERENCES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SummaryProvenance {
    pub model: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub runtime: String,
    pub prompt_version: String,
    pub template: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Summary {
    pub id: String,
    pub text: String,
    pub created_at: u64,
    pub source_version: TranscriptVersion,
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_sha256: Option<String>,
    pub provenance: SummaryProvenance,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryView {
    pub document: Summary,
    pub stale: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryCitation {
    pub reference: u32,
    pub segment: TranscriptSegment,
    pub speaker_name: Option<String>,
    pub current_segment_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryCitations {
    pub summary_id: String,
    pub source_version: TranscriptVersion,
    pub source_session_id: String,
    pub current_version: Option<TranscriptVersion>,
    pub audio_available: bool,
    pub entries: Vec<SummaryCitation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryArchive {
    document: Summary,
    source: Transcript,
    #[serde(default)]
    notes: Option<String>,
    #[serde(rename = "receipt")]
    _receipt: serde_json::Value,
}

fn invalid_citations() -> CoreError {
    CoreError::Storage("요약에 사용한 음성 기록의 연결을 확인할 수 없습니다.".into())
}

fn citation_bytes(root: &Path, relative: &Path) -> Result<Vec<u8>> {
    let path = store::checked_path(root, relative)?;
    let file = store::open_read(&path)?;
    const LIMIT: u64 = 16 * 1024 * 1024;
    if file.metadata()?.len() > LIMIT {
        return Err(invalid_citations());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid_citations());
    }
    Ok(bytes)
}

fn validate_citation_source(source: &Transcript) -> Result<()> {
    store::validate_id(&source.version.id)?;
    store::validate_id(&source.source_session_id)?;
    if source.segments.is_empty() || source.segments.len() > 100_000 || source.speakers.len() > 4096
    {
        return Err(invalid_citations());
    }
    let mut speakers = BTreeSet::new();
    for speaker in &source.speakers {
        if speaker.id.is_empty()
            || speaker.id.len() > 128
            || !speaker
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
            || !speakers.insert(&speaker.id)
            || speaker.name.trim().is_empty()
            || speaker.name.len() > 256
            || speaker.name.chars().any(char::is_control)
        {
            return Err(invalid_citations());
        }
    }
    let mut segments = BTreeSet::new();
    for segment in &source.segments {
        store::validate_id(&segment.id)?;
        if !segments.insert(&segment.id)
            || !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < 0.0
            || segment.end < segment.start
            || segment.text.len() > 2 * 1024 * 1024
            || segment.original_text.len() > 2 * 1024 * 1024
            || segment.text.contains('\0')
            || segment.original_text.contains('\0')
            || segment
                .speaker_id
                .as_ref()
                .is_some_and(|id| !speakers.contains(id))
        {
            return Err(invalid_citations());
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryInput {
    pub source: Transcript,
    pub notes: String,
    pub expected_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDraft {
    pub source: Transcript,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub expected_id: Option<String>,
    pub text: String,
    pub provenance: SummaryProvenance,
    pub receipt: serde_json::Value,
}
pub fn transcript_digest(doc: &Transcript) -> Result<String> {
    let bytes = serde_json::to_vec(doc).map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
pub(crate) fn notes_digest(notes: &str) -> String {
    format!("{:x}", Sha256::digest(notes.as_bytes()))
}

pub(crate) fn is_stale(
    summary: &Summary,
    transcript: Option<&Transcript>,
    notes: &str,
) -> Result<bool> {
    let Some(transcript) = transcript else {
        return Ok(true);
    };
    Ok(transcript.version != summary.source_version
        || transcript_digest(transcript)? != summary.source_sha256
        || summary
            .notes_sha256
            .as_ref()
            .is_some_and(|expected| expected != &notes_digest(notes)))
}
fn location(id: &str) -> Result<PathBuf> {
    store::validate_id(id)?;
    Ok(Path::new("meetings").join(id).join("summaries"))
}
impl Engine {
    pub fn summary_input(&self, id: &str) -> Result<SummaryInput> {
        self.require_active_meeting(id)?;
        let source = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("요약할 음성 기록이 없습니다.".into()))?;
        Ok(SummaryInput {
            source,
            notes: self.meetings[id].meeting.notes.clone(),
            expected_id: self.summary(id)?.map(|view| view.document.id),
        })
    }

    pub fn summary(&self, id: &str) -> Result<Option<SummaryView>> {
        let directory = location(id)?;
        if !self.meetings.contains_key(id) {
            return Err(CoreError::MeetingNotFound);
        }
        let path = store::checked_path(&self.store.root, &directory.join("current.json"))?;
        if !path.exists() {
            return Ok(None);
        }
        let document: Summary = store::read_json(&path)?;
        let current = self.transcript(id)?;
        let stale = is_stale(
            &document,
            current.as_ref(),
            &self.meetings[id].meeting.notes,
        )?;
        Ok(Some(SummaryView { document, stale }))
    }
    pub fn summary_citations(
        &self,
        id: &str,
        summary_id: &str,
        references: &[u32],
    ) -> Result<SummaryCitations> {
        self.require_active_meeting(id)?;
        store::validate_id(id)?;
        store::validate_id(summary_id)?;
        if references.is_empty() || references.len() > MAX_SUMMARY_CITATION_REFERENCES {
            return Err(CoreError::InvalidInput(
                "근거 번호는 1개 이상 32개 이하로 요청해 주세요.".into(),
            ));
        }
        let directory = location(id)?;
        let active_bytes = citation_bytes(&self.store.root, &directory.join("current.json"))?;
        let active: Summary =
            serde_json::from_slice(&active_bytes).map_err(|_| invalid_citations())?;
        if active.id != summary_id {
            return Err(CoreError::InvalidState(
                "요약이 변경되었습니다. 다시 열어 주세요.".into(),
            ));
        }
        let archive: SummaryArchive = serde_json::from_slice(&citation_bytes(
            &self.store.root,
            &directory.join(summary_id).join("run.json"),
        )?)
        .map_err(|_| invalid_citations())?;
        if serde_json::to_value(&archive.document).map_err(|_| invalid_citations())?
            != serde_json::to_value(&active).map_err(|_| invalid_citations())?
            || archive.source.version != active.source_version
            || transcript_digest(&archive.source)? != active.source_sha256
            || active.notes_sha256.as_ref().is_some_and(|expected| {
                archive
                    .notes
                    .as_deref()
                    .is_none_or(|notes| notes_digest(notes) != *expected)
            })
        {
            return Err(invalid_citations());
        }
        validate_citation_source(&archive.source)?;
        if references
            .iter()
            .any(|reference| *reference == 0 || *reference as usize > archive.source.segments.len())
        {
            return Err(CoreError::InvalidInput(
                "요약의 근거 번호가 음성 기록 범위를 벗어났습니다.".into(),
            ));
        }
        let current = if self.meetings[id].meeting.audio.is_some() {
            self.transcript(id)?
        } else {
            None
        };
        if let Some(current) = &current {
            validate_citation_source(current)?;
        }
        let mut targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(current) = &current {
            if current.version == active.source_version
                && transcript_digest(current)? == active.source_sha256
            {
                targets = current
                    .segments
                    .iter()
                    .map(|segment| (segment.id.clone(), vec![segment.id.clone()]))
                    .collect();
            } else if current.source.as_ref().is_some_and(|source| {
                source.projector_version == crate::nemotron::WORD_PROJECTOR_VERSION
            }) {
                let raw_path = Path::new("meetings")
                    .join(id)
                    .join("transcripts")
                    .join(&current.version.id)
                    .join("raw.json");
                targets = crate::nemotron::direct_parent_citation_targets(
                    &citation_bytes(&self.store.root, &raw_path)?,
                    current,
                    &archive.source,
                )?
                .unwrap_or_default();
            }
        }
        let audio_available = self.meetings[id]
            .meeting
            .audio
            .as_ref()
            .is_some_and(|audio| {
                audio.session_id == archive.source.source_session_id
                    && self.audio_path(id).is_ok()
                    && archive
                        .source
                        .segments
                        .iter()
                        .all(|s| s.end <= audio.frames as f64 / f64::from(audio.sample_rate))
            });
        let entries = references
            .iter()
            .map(|reference| {
                let segment = archive.source.segments[*reference as usize - 1].clone();
                let speaker_name = segment.speaker_id.as_ref().and_then(|id| {
                    archive
                        .source
                        .speakers
                        .iter()
                        .find(|speaker| &speaker.id == id)
                        .map(|speaker| speaker.name.clone())
                });
                let current_segment_ids = targets.get(&segment.id).cloned().unwrap_or_default();
                SummaryCitation {
                    reference: *reference,
                    segment,
                    speaker_name,
                    current_segment_ids,
                }
            })
            .collect();
        let result = SummaryCitations {
            summary_id: active.id,
            source_version: active.source_version,
            source_session_id: archive.source.source_session_id,
            current_version: current.map(|doc| doc.version),
            audio_available,
            entries,
        };
        if serde_json::to_vec(&result)
            .map_err(|_| invalid_citations())?
            .len()
            > 1024 * 1024
        {
            return Err(CoreError::InvalidInput(
                "근거 내용이 너무 큽니다. 요청할 근거 수를 줄여 주세요.".into(),
            ));
        }
        if citation_bytes(&self.store.root, &directory.join("current.json"))? != active_bytes {
            return Err(CoreError::InvalidState(
                "요약이 변경되었습니다. 다시 열어 주세요.".into(),
            ));
        }
        Ok(result)
    }
    pub fn commit_summary(&mut self, id: &str, draft: SummaryDraft) -> Result<Summary> {
        self.require_active_meeting(id)?;
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        let directory = location(id)?;
        if !self.meetings.contains_key(id) {
            return Err(CoreError::MeetingNotFound);
        }
        if draft.text.trim().is_empty()
            || draft.text.len() > 2 * 1024 * 1024
            || draft.text.contains('\0')
        {
            return Err(CoreError::InvalidInput(
                "요약 내용이 올바르지 않습니다.".into(),
            ));
        }
        if let Some(notes) = &draft.notes {
            store::validate_text("", notes)?;
        }
        let source_sha256 = transcript_digest(&draft.source)?;
        let doc = Summary {
            id: Uuid::new_v4().to_string(),
            text: draft.text.clone(),
            created_at: store::now(),
            source_version: draft.source.version.clone(),
            source_sha256,
            notes_sha256: draft.notes.as_deref().map(notes_digest),
            provenance: draft.provenance.clone(),
        };
        let relative = directory.join(&doc.id);
        store::ensure_directory(&store::checked_path(&self.store.root, &relative)?)?;
        let mut archive =
            serde_json::json!({"document":doc,"source":draft.source,"receipt":draft.receipt});
        if let Some(notes) = &draft.notes {
            archive["notes"] = serde_json::json!(notes);
        }
        let mut file = store::create_new(&store::checked_path(
            &self.store.root,
            &relative.join("run.json"),
        )?)?;
        serde_json::to_writer(&mut file, &archive)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        let current = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("전사가 없습니다.".into()))?;
        if transcript_digest(&current)? != doc.source_sha256
            || draft
                .notes
                .as_ref()
                .is_some_and(|notes| notes != &self.meetings[id].meeting.notes)
            || self.summary(id)?.map(|v| v.document.id) != draft.expected_id
        {
            return Err(CoreError::InvalidState("음성 기록, 메모 또는 회의록이 변경되어 새 결과를 적용하지 않았습니다. 기존 내용은 유지했습니다.".into()));
        }
        store::write_json(&self.store.root, &directory.join("current.json"), &doc)?;
        Ok(doc)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::tests::{output, setup};
    fn draft(source: Transcript, expected_id: Option<String>, text: &str) -> SummaryDraft {
        SummaryDraft {
            source,
            notes: None,
            expected_id,
            text: text.into(),
            provenance: SummaryProvenance {
                model: "test".into(),
                model_revision: "1".into(),
                model_sha256: "a".repeat(64),
                runtime: "test".into(),
                prompt_version: "test".into(),
                template: "minutes".into(),
            },
            receipt: serde_json::json!({"finish_reason":"stop"}),
        }
    }
    #[test]
    fn summaries_reopen_without_changing_audio_notes_or_transcript() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let root = engine.store.root.clone();
        let audio = std::fs::read(engine.audio_path(&id).unwrap()).unwrap();
        let before = transcript_digest(&source).unwrap();
        let one = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let two = engine
            .commit_summary(&id, draft(source, Some(one.id.clone()), "새 요약 [1]"))
            .unwrap();
        assert!(root
            .join(location(&id).unwrap())
            .join(one.id)
            .join("run.json")
            .is_file());
        drop(engine);
        let engine = Engine::open(root).unwrap();
        let saved = engine.summary(&id).unwrap().unwrap();
        assert_eq!(saved.document.id, two.id);
        assert!(!saved.stale);
        assert_eq!(
            transcript_digest(&engine.transcript(&id).unwrap().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(engine.audio_path(&id).unwrap()).unwrap(),
            audio
        );
    }
    #[test]
    fn changed_source_or_competing_summary_keeps_prior_result() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let one = engine
            .commit_summary(&id, draft(source.clone(), None, "첫 요약"))
            .unwrap();
        assert!(engine
            .commit_summary(&id, draft(source.clone(), None, "늦은 결과"))
            .is_err());
        engine
            .edit_transcript(&id, source.version.clone(), vec!["정정".into()])
            .unwrap();
        assert!(engine
            .commit_summary(&id, draft(source, Some(one.id.clone()), "오래된 원문"))
            .is_err());
        let saved = engine.summary(&id).unwrap().unwrap();
        assert_eq!(saved.document.id, one.id);
        assert!(saved.stale);
        assert!(engine
            .commit_summary(
                &id,
                draft(
                    engine.transcript(&id).unwrap().unwrap(),
                    Some(saved.document.id),
                    "  "
                )
            )
            .is_err());
    }
    #[test]
    fn restored_transcript_keeps_summary_and_rejects_in_flight_old_source() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let second = engine
            .commit_transcript(&id, Some(first.version.clone()), output())
            .unwrap();
        let summary = engine
            .commit_summary(&id, draft(second.clone(), None, "기존 요약"))
            .unwrap();
        let old_result = draft(second.clone(), Some(summary.id.clone()), "늦은 새 요약");
        engine
            .restore_transcript(&id, second.version, first.version)
            .unwrap();
        let view = engine.summary(&id).unwrap().unwrap();
        assert!(view.stale);
        assert_eq!(view.document.id, summary.id);
        assert_eq!(view.document.text, "기존 요약");
        assert!(engine.commit_summary(&id, old_result).is_err());
        assert_eq!(
            engine.summary(&id).unwrap().unwrap().document.id,
            summary.id
        );
    }

    #[test]
    fn notes_context_is_frozen_in_the_archive_and_changes_make_it_stale() {
        let (_temp, mut engine, id, _) = setup();
        engine
            .update_meeting(&id, "회의".into(), "처음 작성한 안건".into())
            .unwrap();
        let transcript = engine.commit_transcript(&id, None, output()).unwrap();
        let input = engine.summary_input(&id).unwrap();
        assert_eq!(input.notes, "처음 작성한 안건");
        assert_eq!(input.source.version, transcript.version);
        assert!(input.expected_id.is_none());
        let mut candidate = draft(input.source, input.expected_id, "회의록 [1]");
        candidate.notes = Some(input.notes);
        let saved = engine.commit_summary(&id, candidate).unwrap();
        assert_eq!(
            saved.notes_sha256.as_deref(),
            Some(notes_digest("처음 작성한 안건").as_str())
        );
        assert_eq!(saved.source_sha256, transcript_digest(&transcript).unwrap());
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        let root = engine.store.root.clone();
        let archive = root
            .join(location(&id).unwrap())
            .join(&saved.id)
            .join("run.json");
        let before = std::fs::read(&archive).unwrap();
        let run: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(run["notes"], "처음 작성한 안건");
        engine
            .update_meeting(&id, "바꾼 제목".into(), "처음 작성한 안건".into())
            .unwrap();
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        engine
            .update_meeting(&id, "바꾼 제목".into(), "수정한 안건".into())
            .unwrap();
        assert!(engine.summary(&id).unwrap().unwrap().stale);
        assert_eq!(std::fs::read(&archive).unwrap(), before);
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        let summary = reopened.summary(&id).unwrap().unwrap();
        assert!(summary.stale);
        assert_eq!(summary.document.id, saved.id);
        assert_eq!(std::fs::read(archive).unwrap(), before);
    }

    #[test]
    fn changed_notes_reject_late_summary_without_replacing_the_previous_result() {
        let (_temp, mut engine, id, _) = setup();
        engine.commit_transcript(&id, None, output()).unwrap();
        let input = engine.summary_input(&id).unwrap();
        let mut first = draft(input.source, input.expected_id, "기존 회의록 [1]");
        first.notes = Some(input.notes);
        let previous = engine.commit_summary(&id, first).unwrap();
        let input = engine.summary_input(&id).unwrap();
        assert_eq!(input.expected_id.as_deref(), Some(previous.id.as_str()));
        let mut late = draft(input.source, input.expected_id, "늦게 끝난 회의록 [1]");
        late.notes = Some(input.notes);
        let current = engine
            .store
            .root
            .join(location(&id).unwrap())
            .join("current.json");
        let before = std::fs::read(&current).unwrap();
        engine
            .update_meeting(&id, "회의".into(), "새로 추가한 메모".into())
            .unwrap();
        assert!(matches!(
            engine.commit_summary(&id, late),
            Err(CoreError::InvalidState(_))
        ));
        assert_eq!(std::fs::read(current).unwrap(), before);
        let summary = engine.summary(&id).unwrap().unwrap();
        assert_eq!(summary.document.id, previous.id);
        assert!(summary.stale);
    }

    #[test]
    fn empty_notes_are_bound_but_legacy_transcript_only_summaries_remain_readable() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let mut candidate = draft(source.clone(), None, "빈 메모의 회의록");
        candidate.notes = Some(String::new());
        let current = engine.commit_summary(&id, candidate).unwrap();
        assert_eq!(current.notes_sha256, Some(notes_digest("")));
        engine
            .update_meeting(&id, "회의".into(), "추가한 메모".into())
            .unwrap();
        assert!(engine.summary(&id).unwrap().unwrap().stale);
        let legacy = engine
            .commit_summary(&id, draft(source, Some(current.id), "이전 형식의 회의록"))
            .unwrap();
        assert!(serde_json::to_value(&legacy)
            .unwrap()
            .get("notesSha256")
            .is_none());
        let parsed: Summary =
            serde_json::from_value(serde_json::to_value(&legacy).unwrap()).unwrap();
        assert!(parsed.notes_sha256.is_none());
        engine
            .update_meeting(&id, "회의".into(), "다시 수정한 메모".into())
            .unwrap();
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        let summary = reopened.summary(&id).unwrap().unwrap();
        assert_eq!(summary.document.id, legacy.id);
        assert!(!summary.stale);
    }

    fn archived_run(engine: &Engine, id: &str, summary: &Summary) -> PathBuf {
        engine
            .store
            .root
            .join(location(id).unwrap())
            .join(&summary.id)
            .join("run.json")
    }

    #[test]
    fn citations_read_the_frozen_source_and_current_target_without_writes() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let summary = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let archive = archived_run(&engine, &id, &summary);
        let before = std::fs::read(&archive).unwrap();
        let result = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert_eq!(result.summary_id, summary.id);
        assert_eq!(result.source_version, source.version);
        assert_eq!(result.current_version, Some(source.version));
        assert_eq!(result.entries[0].segment.id, source.segments[0].id);
        assert_eq!(result.entries[0].segment.text, "원문");
        assert_eq!(
            result.entries[0].speaker_name,
            Some(source.speakers[0].name.clone())
        );
        assert_eq!(
            result.entries[0].current_segment_ids,
            vec![source.segments[0].id.clone()]
        );
        assert!(result.audio_available);
        assert_eq!(std::fs::read(archive).unwrap(), before);
        let value = serde_json::to_value(result).unwrap();
        assert!(value.get("sourceVersion").is_some());
        assert!(value["entries"][0].get("currentSegmentIds").is_some());
    }

    #[test]
    fn stale_citations_map_only_a_verified_direct_parent_generation() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine
            .commit_nemotron_transcript(&id, None, crate::nemotron::tests::raw())
            .unwrap();
        let summary = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let turns = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(source.clone()))
            .unwrap();
        let result = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert_eq!(result.entries[0].segment.text, source.segments[0].text);
        assert_eq!(result.entries[0].segment.id, source.segments[0].id);
        assert_eq!(
            result.entries[0].current_segment_ids,
            turns.segments[..2]
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.current_version, Some(turns.version.clone()));
        assert!(engine.summary(&id).unwrap().unwrap().stale);
        let next = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(turns))
            .unwrap();
        let unmapped = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert_eq!(unmapped.current_version, Some(next.version));
        assert_eq!(unmapped.entries[0].segment.text, source.segments[0].text);
        assert!(unmapped.entries[0].current_segment_ids.is_empty());
        assert!(unmapped.audio_available);
    }

    #[test]
    fn changed_revision_or_unrelated_generation_never_reuses_reference_ordinals() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let summary = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let edited = engine
            .edit_transcript(&id, source.version, vec!["수정한 문장".into()])
            .unwrap();
        let result = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert_eq!(result.entries[0].segment.text, "원문");
        assert!(result.entries[0].current_segment_ids.is_empty());
        engine
            .commit_transcript(&id, Some(edited.version), output())
            .unwrap();
        assert!(engine
            .summary_citations(&id, &summary.id, &[1])
            .unwrap()
            .entries[0]
            .current_segment_ids
            .is_empty());
    }

    #[test]
    fn citation_requests_reject_invalid_ranges_and_a_late_summary_id() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let first = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        for references in [vec![], vec![0], vec![2], vec![1, 2], vec![1; 33]] {
            assert!(matches!(
                engine.summary_citations(&id, &first.id, &references),
                Err(CoreError::InvalidInput(_))
            ));
        }
        let second = engine
            .commit_summary(&id, draft(source, Some(first.id.clone()), "새 요약 [1]"))
            .unwrap();
        assert!(matches!(
            engine.summary_citations(&id, &first.id, &[1]),
            Err(CoreError::InvalidState(_))
        ));
        assert!(engine.summary_citations(&id, &second.id, &[1]).is_ok());
    }

    #[test]
    fn missing_audio_or_replaced_session_keeps_excerpts_without_playing_another_recording() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let summary = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let audio = engine.audio_path(&id).unwrap();
        std::fs::rename(&audio, audio.with_extension("unavailable")).unwrap();
        let missing = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert!(!missing.audio_available);
        assert_eq!(missing.entries[0].segment.text, source.segments[0].text);
        let meeting = &mut engine.meetings.get_mut(&id).unwrap().meeting;
        meeting.transcript_id = None;
        meeting.audio.as_mut().unwrap().session_id = Uuid::new_v4().to_string();
        let replaced = engine.summary_citations(&id, &summary.id, &[1]).unwrap();
        assert!(!replaced.audio_available);
        assert!(replaced.current_version.is_none());
        assert!(replaced.entries[0].current_segment_ids.is_empty());
        engine.meetings.get_mut(&id).unwrap().meeting.audio = None;
        assert!(
            !engine
                .summary_citations(&id, &summary.id, &[1])
                .unwrap()
                .audio_available
        );
    }

    #[test]
    fn citations_reject_changed_archive_identity_and_source_digest() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let summary = engine
            .commit_summary(&id, draft(source, None, "요약 [1]"))
            .unwrap();
        let path = archived_run(&engine, &id, &summary);
        let before = std::fs::read(&path).unwrap();
        for kind in 0..3 {
            let mut value: serde_json::Value = serde_json::from_slice(&before).unwrap();
            match kind {
                0 => value["source"]["segments"][0]["text"] = "바뀐 원문".into(),
                1 => value["document"]["sourceVersion"]["revision"] = 99.into(),
                _ => value["source"]["sourceSessionId"] = Uuid::new_v4().to_string().into(),
            }
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(engine.summary_citations(&id, &summary.id, &[1]).is_err());
        }
    }

    #[test]
    fn direct_parent_targets_reject_changed_raw_or_current_atom_ids() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine
            .commit_nemotron_transcript(&id, None, crate::nemotron::tests::raw())
            .unwrap();
        let summary = engine
            .commit_summary(&id, draft(source.clone(), None, "요약 [1]"))
            .unwrap();
        let turns = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(source))
            .unwrap();
        let directory = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&turns.version.id);
        let current = directory.join("current.json");
        let before = std::fs::read(&current).unwrap();
        let mut altered = turns;
        altered.segments[0].id = Uuid::new_v4().to_string();
        std::fs::write(&current, serde_json::to_vec(&altered).unwrap()).unwrap();
        assert!(engine.summary_citations(&id, &summary.id, &[1]).is_err());
        std::fs::write(current, before).unwrap();
        let raw = directory.join("raw.json");
        let mut bytes = std::fs::read(&raw).unwrap();
        bytes.push(b' ');
        std::fs::write(raw, bytes).unwrap();
        assert!(engine.summary_citations(&id, &summary.id, &[1]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn citation_archive_paths_never_follow_links() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let summary = engine
            .commit_summary(&id, draft(source, None, "요약 [1]"))
            .unwrap();
        let path = archived_run(&engine, &id, &summary);
        let retained = path.with_file_name("retained.json");
        std::fs::rename(&path, &retained).unwrap();
        std::os::unix::fs::symlink(retained, path).unwrap();
        assert!(engine.summary_citations(&id, &summary.id, &[1]).is_err());
    }
}
