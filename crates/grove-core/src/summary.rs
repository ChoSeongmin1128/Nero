use crate::engine::Engine;
use crate::store;
use crate::{CoreError, Result, Transcript, TranscriptVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
}
