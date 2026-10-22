use crate::{CoreError, Result, Transcript};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_TERMINOLOGY_PROFILE_BYTES: usize = 512 * 1024;
pub const MAX_TERMINOLOGY_CANDIDATES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminologyProfile {
    pub schema_version: u32,
    pub id: String,
    pub revision: String,
    pub rules: Vec<TerminologyRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminologyRule {
    pub id: String,
    pub canonical: String,
    pub source_forms: Vec<String>,
    pub kind: TerminologyKind,
    pub context_cues: Vec<String>,
    #[serde(default)]
    pub exclusion_cues: Vec<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TerminologyKind {
    Acronym,
    Contextual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminologyCandidate {
    pub id: String,
    pub rule_id: String,
    pub segment_id: String,
    pub segment_index: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub source_text: String,
    pub replacement: String,
    pub context_cues: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TerminologyOutcome {
    Accept,
    Keep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminologyDecision {
    pub candidate_id: String,
    pub outcome: TerminologyOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminologyDraft {
    pub source: Transcript,
    pub profile: TerminologyProfile,
    pub decisions: Vec<TerminologyDecision>,
    pub receipt: serde_json::Value,
}

fn invalid() -> CoreError {
    CoreError::InvalidInput("용어 보정 설정 또는 결과가 올바르지 않습니다.".into())
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && value.len() <= max
        && !value.chars().any(char::is_control)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
}

impl TerminologyProfile {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || !identifier(&self.id)
            || !identifier(&self.revision)
            || self.rules.len() > 256
            || serde_json::to_vec(self).map_err(|_| invalid())?.len()
                > MAX_TERMINOLOGY_PROFILE_BYTES
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut forms = BTreeSet::new();
        for rule in &self.rules {
            if !identifier(&rule.id)
                || !ids.insert(&rule.id)
                || !text(&rule.canonical, 256)
                || !rule
                    .canonical
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
                || !rule
                    .canonical
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
                || rule.source_forms.is_empty()
                || rule.source_forms.len() > 8
                || rule.context_cues.len() > 16
                || rule.exclusion_cues.len() > 16
                || rule.description.len() > 1024
                || rule.description.chars().any(char::is_control)
                || rule
                    .context_cues
                    .iter()
                    .chain(&rule.exclusion_cues)
                    .any(|cue| {
                        !text(cue, 128)
                            || cue.chars().filter(|c| c.is_alphanumeric()).count() < 2
                            || cue == &rule.canonical
                            || rule.source_forms.contains(cue)
                    })
            {
                return Err(invalid());
            }
            if rule.kind == TerminologyKind::Contextual && rule.context_cues.is_empty() {
                return Err(invalid());
            }
            if rule.kind == TerminologyKind::Acronym
                && (!(2..=16).contains(&rule.canonical.len())
                    || !rule.canonical.bytes().all(|c| c.is_ascii_uppercase()))
            {
                return Err(invalid());
            }
            for form in &rule.source_forms {
                if !text(form, 256)
                    || form == &rule.canonical
                    || !forms.insert(form)
                    || !form.chars().next().is_some_and(char::is_alphanumeric)
                    || !form.chars().next_back().is_some_and(char::is_alphanumeric)
                {
                    return Err(invalid());
                }
                if rule.kind == TerminologyKind::Acronym {
                    let letters: Vec<_> = form.split(' ').collect();
                    if letters.len() < 2
                        || letters
                            .iter()
                            .any(|s| s.len() != 1 || !s.as_bytes()[0].is_ascii_uppercase())
                        || letters.concat() != rule.canonical
                    {
                        return Err(invalid());
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn transcript_digest(transcript: &Transcript) -> Result<String> {
    crate::summary::transcript_digest(transcript)
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn trailing_particle(tail: &str) -> Option<&'static str> {
    const PARTICLES: &[&str] = &[
        "으로는",
        "에서는",
        "에게는",
        "로는",
        "에는",
        "은",
        "는",
        "이",
        "가",
        "을",
        "를",
        "의",
        "에",
        "에서",
        "으로",
        "로",
        "와",
        "과",
        "도",
        "만",
        "까지",
        "부터",
    ];
    PARTICLES.iter().copied().find(|particle| {
        tail.strip_prefix(particle)
            .is_some_and(|rest| !rest.chars().next().is_some_and(word))
    })
}

fn boundary(value: &str, start: usize, end: usize) -> bool {
    if value[..start].chars().next_back().is_some_and(word) {
        return false;
    }
    let tail = &value[end..];
    if !tail.chars().next().is_some_and(word) {
        return true;
    }
    trailing_particle(tail).is_some()
}

fn hangul_final(letter: char) -> Option<u32> {
    ('가'..='힣')
        .contains(&letter)
        .then(|| (letter as u32 - '가' as u32) % 28)
}

fn preserves_particle(value: &str, end: usize, source: &str, replacement: &str) -> bool {
    let source_last = source.chars().next_back();
    let replacement_last = replacement.chars().next_back();
    if source_last == replacement_last {
        return true;
    }
    let particle = trailing_particle(&value[end..]);
    let condition: fn(u32) -> bool = match particle {
        Some("이" | "가" | "은" | "는" | "을" | "를" | "과" | "와") => |last| last != 0,
        Some("으로" | "로" | "으로는" | "로는") => |last| last != 0 && last != 8,
        _ => return true,
    };
    match (
        source_last.and_then(hangul_final),
        replacement_last.and_then(hangul_final),
    ) {
        (Some(source), Some(replacement)) => condition(source) == condition(replacement),
        _ => false,
    }
}

fn matches<'a>(value: &'a str, needle: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    value.match_indices(needle).filter_map(move |(start, _)| {
        let end = start + needle.len();
        boundary(value, start, end).then_some((start, end))
    })
}

fn intersects(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

pub fn has_literal_evidence(
    candidate: &TerminologyCandidate,
    source: &Transcript,
    ids: &[String],
) -> bool {
    let Some(segment) = source.segments.get(candidate.segment_index) else {
        return false;
    };
    if segment.id != candidate.segment_id
        || candidate.byte_start >= candidate.byte_end
        || segment.text.get(candidate.byte_start..candidate.byte_end)
            != Some(candidate.source_text.as_str())
    {
        return false;
    }
    let first = candidate.segment_index.saturating_sub(1);
    let last = candidate
        .segment_index
        .saturating_add(1)
        .min(source.segments.len() - 1);
    (first..=last).any(|index| {
        let segment = &source.segments[index];
        ids.contains(&segment.id)
            && candidate.context_cues.iter().any(|cue| {
                !cue.is_empty()
                    && matches(&segment.text, cue).any(|range| {
                        index != candidate.segment_index
                            || !intersects(range, (candidate.byte_start, candidate.byte_end))
                    })
            })
    })
}

fn longer_spaced_acronym(value: &str, start: usize, end: usize) -> bool {
    let left = &value[..start];
    let left_trimmed = left.trim_end_matches(char::is_whitespace);
    if left_trimmed.len() != left.len() {
        if let Some((index, letter)) = left_trimmed.char_indices().next_back() {
            if letter.is_ascii_uppercase() && boundary(left_trimmed, index, left_trimmed.len()) {
                return true;
            }
        }
    }
    let right = &value[end..];
    let right_trimmed = right.trim_start_matches(char::is_whitespace);
    right_trimmed.len() != right.len()
        && right_trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        && boundary(right_trimmed, 0, 1)
}

pub fn discover_candidates(
    profile: &TerminologyProfile,
    source: &Transcript,
) -> Result<Vec<TerminologyCandidate>> {
    profile.validate()?;
    if source.version.revision != 0 {
        return Ok(Vec::new());
    }
    if source.segments.len() > 100_000
        || source.segments.iter().map(|s| s.text.len()).sum::<usize>() > 2 * 1024 * 1024
    {
        return Err(invalid());
    }
    let source_hash = transcript_digest(source)?;
    let profile_bytes = serde_json::to_vec(profile).map_err(|_| invalid())?;
    let profile_hash = format!("{:x}", Sha256::digest(profile_bytes));
    let mut candidates = Vec::new();
    for (segment_index, segment) in source.segments.iter().enumerate() {
        // Automatic processing never revisits user corrections or a previous correction revision.
        if segment.text != segment.original_text {
            continue;
        }
        let canonical_ranges: Vec<_> = profile
            .rules
            .iter()
            .flat_map(|rule| matches(&segment.text, &rule.canonical))
            .collect();
        for rule in &profile.rules {
            if rule.kind == TerminologyKind::Contextual
                && rule.exclusion_cues.iter().any(|cue| {
                    let first = segment_index.saturating_sub(1);
                    let last = (segment_index + 1).min(source.segments.len() - 1);
                    (first..=last)
                        .any(|index| matches(&source.segments[index].text, cue).next().is_some())
                })
            {
                continue;
            }
            for form in &rule.source_forms {
                for (byte_start, byte_end) in matches(&segment.text, form) {
                    if !preserves_particle(&segment.text, byte_end, form, &rule.canonical) {
                        continue;
                    }
                    if rule.kind == TerminologyKind::Acronym
                        && longer_spaced_acronym(&segment.text, byte_start, byte_end)
                    {
                        continue;
                    }
                    if canonical_ranges
                        .iter()
                        .any(|range| intersects(*range, (byte_start, byte_end)))
                    {
                        continue;
                    }
                    let context_cues: Vec<_> = rule
                        .context_cues
                        .iter()
                        .filter(|cue| {
                            let first = segment_index.saturating_sub(1);
                            let last = (segment_index + 1).min(source.segments.len() - 1);
                            (first..=last).any(|index| {
                                matches(&source.segments[index].text, cue).any(|range| {
                                    index != segment_index
                                        || !intersects(range, (byte_start, byte_end))
                                })
                            })
                        })
                        .cloned()
                        .collect();
                    if rule.kind == TerminologyKind::Contextual && context_cues.is_empty() {
                        continue;
                    }
                    let identity = serde_json::json!([
                        source_hash,
                        profile_hash,
                        rule.id,
                        segment.id,
                        segment_index,
                        byte_start,
                        byte_end,
                        form,
                        rule.canonical
                    ]);
                    candidates.push(TerminologyCandidate {
                        id: format!(
                            "{:x}",
                            Sha256::digest(serde_json::to_vec(&identity).map_err(|_| invalid())?)
                        ),
                        rule_id: rule.id.clone(),
                        segment_id: segment.id.clone(),
                        segment_index,
                        byte_start,
                        byte_end,
                        source_text: form.clone(),
                        replacement: rule.canonical.clone(),
                        context_cues,
                    });
                    if candidates.len() > MAX_TERMINOLOGY_CANDIDATES {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    candidates.sort_by_key(|c| (c.segment_index, c.byte_start, c.byte_end));
    // An overlapping interpretation is uncertain, so neither candidate can be accepted.
    let ambiguous: BTreeSet<_> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, a)| {
            candidates
                .iter()
                .enumerate()
                .any(|(other, b)| {
                    index != other
                        && a.segment_index == b.segment_index
                        && intersects((a.byte_start, a.byte_end), (b.byte_start, b.byte_end))
                })
                .then_some(index)
        })
        .collect();
    Ok(candidates
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| (!ambiguous.contains(&i)).then_some(c))
        .collect())
}

pub(crate) fn prepare(draft: &TerminologyDraft) -> Result<(Transcript, serde_json::Value)> {
    if draft.decisions.len() > MAX_TERMINOLOGY_CANDIDATES
        || serde_json::to_vec(&draft.receipt)
            .map_err(|_| invalid())?
            .len()
            > 128 * 1024
    {
        return Err(invalid());
    }
    let candidates = discover_candidates(&draft.profile, &draft.source)?;
    let mut decisions = BTreeMap::new();
    for decision in &draft.decisions {
        if decisions
            .insert(decision.candidate_id.as_str(), decision.outcome)
            .is_some()
            || !candidates
                .iter()
                .any(|candidate| candidate.id == decision.candidate_id)
        {
            return Err(invalid());
        }
    }
    let mut after = draft.source.clone();
    for candidate in candidates.iter().rev() {
        if decisions.get(candidate.id.as_str()) == Some(&TerminologyOutcome::Accept) {
            let segment = &mut after.segments[candidate.segment_index];
            if segment.text.get(candidate.byte_start..candidate.byte_end)
                != Some(candidate.source_text.as_str())
            {
                return Err(invalid());
            }
            segment.text.replace_range(
                candidate.byte_start..candidate.byte_end,
                &candidate.replacement,
            );
        }
    }
    if after
        .segments
        .iter()
        .map(|segment| segment.text.len())
        .sum::<usize>()
        > 2 * 1024 * 1024
    {
        return Err(invalid());
    }
    let provenance = serde_json::json!({
        "schemaVersion":1,
        "ruleVersion":"fixed-spans-v1",
        "sourceSha256":transcript_digest(&draft.source)?,
        "source":draft.source,
        "profile":draft.profile,
        "candidates":candidates,
        "decisions":draft.decisions,
        "receipt":draft.receipt,
    });
    Ok((after, provenance))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TranscriptSegment, TranscriptVersion};

    fn profile() -> TerminologyProfile {
        TerminologyProfile {
            schema_version: 1,
            id: "synthetic".into(),
            revision: "1".into(),
            rules: vec![
                TerminologyRule {
                    id: "name".into(),
                    canonical: "가상도구".into(),
                    source_forms: vec!["가상 도구".into()],
                    kind: TerminologyKind::Contextual,
                    context_cues: vec!["설계".into()],
                    exclusion_cues: vec![],
                    description: String::new(),
                },
                TerminologyRule {
                    id: "letters".into(),
                    canonical: "ABC".into(),
                    source_forms: vec!["A B C".into()],
                    kind: TerminologyKind::Acronym,
                    context_cues: vec![],
                    exclusion_cues: vec![],
                    description: String::new(),
                },
            ],
        }
    }

    fn transcript(texts: &[&str]) -> Transcript {
        Transcript {
            version: TranscriptVersion {
                id: "11111111-1111-4111-8111-111111111111".into(),
                revision: 0,
            },
            source_session_id: "source".into(),
            model: "synthetic".into(),
            model_revision: "1".into(),
            source: None,
            speakers: vec![],
            segments: texts
                .iter()
                .enumerate()
                .map(|(index, text)| TranscriptSegment {
                    id: format!("00000000-0000-4000-8000-{index:012}"),
                    start: index as f64,
                    end: index as f64 + 1.0,
                    original_text: (*text).into(),
                    text: (*text).into(),
                    speaker_id: None,
                    source_index: Some(index as u32),
                })
                .collect(),
        }
    }

    fn accepted(source: Transcript, profile: TerminologyProfile) -> TerminologyDraft {
        let decisions = discover_candidates(&profile, &source)
            .unwrap()
            .into_iter()
            .map(|candidate| TerminologyDecision {
                candidate_id: candidate.id,
                outcome: TerminologyOutcome::Accept,
            })
            .collect();
        TerminologyDraft {
            source,
            profile,
            decisions,
            receipt: serde_json::json!({"test":true}),
        }
    }

    #[test]
    fn configured_spans_preserve_particles_numbers_negation_and_punctuation() {
        let source = transcript(&[
            "설계 검토",
            "가상 도구를 바꾸지 않습니다. 10개, 2.5%, A B C는 그대로입니다.",
        ]);
        let draft = accepted(source.clone(), profile());
        let (result, _) = prepare(&draft).unwrap();
        assert_eq!(
            result.segments[1].text,
            "가상도구를 바꾸지 않습니다. 10개, 2.5%, ABC는 그대로입니다."
        );
        assert_eq!(
            result.segments[1].original_text,
            source.segments[1].original_text
        );
        assert_eq!(result.version, source.version);
        assert_eq!(result.segments[1].start, source.segments[1].start);
        assert_eq!(
            result.segments[1].source_index,
            source.segments[1].source_index
        );
    }

    #[test]
    fn changed_hangul_endings_cannot_damage_attached_particles() {
        let mut p = profile();
        p.rules[0].source_forms = vec!["모듈론".into()];
        p.rules[0].canonical = "모듈로".into();
        for particle in [
            "이",
            "가",
            "은",
            "는",
            "을",
            "를",
            "과",
            "와",
            "으로",
            "로",
            "으로는",
            "로는",
        ] {
            let value = format!("설계 모듈론{particle} 유지");
            assert!(
                discover_candidates(&p, &transcript(&[&value]))
                    .unwrap()
                    .is_empty(),
                "{particle}"
            );
        }
        for particle in ["에", "의", "에서", "에는", "에서는", "도", "까지"] {
            let value = format!("설계 모듈론{particle} 유지");
            let draft = accepted(transcript(&[&value]), p.clone());
            assert_eq!(draft.decisions.len(), 1, "{particle}");
            assert_eq!(
                prepare(&draft).unwrap().0.segments[0].text,
                format!("설계 모듈로{particle} 유지")
            );
        }
        p.rules[0].source_forms = vec!["모듈로".into()];
        p.rules[0].canonical = "모듈론".into();
        assert!(
            discover_candidates(&p, &transcript(&["설계 모듈로는 유지"]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unchanged_particle_conditions_and_rieul_exception_remain_eligible() {
        let mut p = profile();
        p.rules[0].source_forms = vec!["가상 틀".into()];
        p.rules[0].canonical = "가상도구".into();
        assert_eq!(
            discover_candidates(&p, &transcript(&["설계 가상 틀로 유지"]))
                .unwrap()
                .len(),
            1
        );
        assert!(
            discover_candidates(&p, &transcript(&["설계 가상 틀은 유지"]))
                .unwrap()
                .is_empty()
        );
        p.rules[0].source_forms = vec!["가상 틀".into()];
        p.rules[0].canonical = "가상판".into();
        assert_eq!(
            discover_candidates(&p, &transcript(&["설계 가상 틀은 유지"]))
                .unwrap()
                .len(),
            1
        );
        assert!(
            discover_candidates(&p, &transcript(&["설계 가상 틀로 유지"]))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            discover_candidates(&p, &transcript(&["A B C는 유지"]))
                .unwrap()
                .len(),
            1
        );
        p.rules[0].source_forms = vec!["TOOL".into()];
        p.rules[0].canonical = "BOX".into();
        assert!(discover_candidates(&p, &transcript(&["설계 TOOL은 유지"]))
            .unwrap()
            .is_empty());
        assert_eq!(
            discover_candidates(&p, &transcript(&["설계 TOOL에 유지"]))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn candidates_are_stable_and_bound_to_profile_source_and_utf8_span() {
        let source = transcript(&["설계 가상 도구, A B C"]);
        let first = discover_candidates(&profile(), &source).unwrap();
        assert_eq!(first, discover_candidates(&profile(), &source).unwrap());
        for candidate in &first {
            assert_eq!(
                source.segments[candidate.segment_index]
                    .text
                    .get(candidate.byte_start..candidate.byte_end),
                Some(candidate.source_text.as_str())
            );
        }
        let mut changed = source.clone();
        changed.segments[0].text.push('.');
        changed.segments[0].original_text.push('.');
        assert_ne!(
            first[0].id,
            discover_candidates(&profile(), &changed).unwrap()[0].id
        );
        let mut changed_profile = profile();
        changed_profile.revision = "2".into();
        assert_ne!(
            first[0].id,
            discover_candidates(&changed_profile, &source).unwrap()[0].id
        );
    }

    #[test]
    fn names_need_external_cues_and_explicit_acceptance() {
        assert!(
            discover_candidates(&profile(), &transcript(&["가상 도구를 사용합니다."]))
                .unwrap()
                .is_empty()
        );
        let source = transcript(&["설계 가상 도구"]);
        let mut draft = accepted(source.clone(), profile());
        draft.decisions.clear();
        assert_eq!(
            prepare(&draft).unwrap().0.segments[0].text,
            source.segments[0].text
        );
        draft.decisions.push(TerminologyDecision {
            candidate_id: discover_candidates(&draft.profile, &source).unwrap()[0]
                .id
                .clone(),
            outcome: TerminologyOutcome::Keep,
        });
        assert_eq!(
            prepare(&draft).unwrap().0.segments[0].text,
            source.segments[0].text
        );
    }

    #[test]
    fn configured_exclusions_override_positive_context_in_same_or_adjacent_utterances() {
        let mut p = profile();
        p.rules[0].exclusion_cues = vec!["활동명".into()];
        for texts in [
            vec!["설계 가상 도구는 활동명 표기입니다."],
            vec!["활동명 표기입니다.", "설계 가상 도구를 유지합니다."],
            vec!["설계 가상 도구를 유지합니다.", "활동명 표기입니다."],
        ] {
            assert!(discover_candidates(&p, &transcript(&texts))
                .unwrap()
                .is_empty());
        }
        let mut acronym_only = p.clone();
        acronym_only.rules[1].exclusion_cues = vec!["활동명".into()];
        assert_eq!(
            discover_candidates(&acronym_only, &transcript(&["활동명 A B C는 유지"]))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn distant_or_embedded_exclusion_words_do_not_block_an_eligible_name() {
        let mut p = profile();
        p.rules[0].exclusion_cues = vec!["활동명".into()];
        for texts in [
            vec!["활동명 표기", "다른 안건", "설계 가상 도구를 유지합니다."],
            vec!["활동명세 검토", "설계 가상 도구를 유지합니다."],
        ] {
            assert_eq!(
                discover_candidates(&p, &transcript(&texts)).unwrap().len(),
                1
            );
        }
        for cue in ["가상도구", "가상 도구", "a"] {
            p.rules[0].exclusion_cues = vec![cue.into()];
            assert!(p.validate().is_err());
        }
        p.rules[0].exclusion_cues = vec!["활동명".into(); 17];
        assert!(p.validate().is_err());
    }

    #[test]
    fn excluded_names_cannot_be_reintroduced_by_a_forged_accept_decision() {
        let source = transcript(&["설계 가상 도구를 유지합니다.", "활동명 표기입니다."]);
        let mut draft = accepted(source.clone(), profile());
        assert_eq!(draft.decisions.len(), 1);
        draft.profile.rules[0].exclusion_cues = vec!["활동명".into()];
        assert!(discover_candidates(&draft.profile, &source)
            .unwrap()
            .is_empty());
        assert!(prepare(&draft).is_err());
        assert_eq!(source.segments[0].text, "설계 가상 도구를 유지합니다.");
    }

    #[test]
    fn cue_cannot_be_the_source_phrase_and_distant_context_does_not_qualify() {
        let mut invalid_profile = profile();
        invalid_profile.rules[0].context_cues = vec!["가상 도구".into()];
        assert!(invalid_profile.validate().is_err());
        let source = transcript(&["설계", "관련 없는 말", "다른 이야기", "가상 도구"]);
        assert!(discover_candidates(&profile(), &source).unwrap().is_empty());
    }

    #[test]
    fn literal_matching_does_not_expand_short_aliases_or_embedded_words() {
        let mut p = profile();
        p.rules[0].source_forms = vec!["프로".into()];
        let source = transcript(&["설계 프로필, 프로세스, 프로답다, 초프로, 가상도구, ABC"]);
        assert!(discover_candidates(&p, &source).unwrap().is_empty());
    }

    #[test]
    fn existing_canonical_spelling_is_never_a_candidate() {
        let mut p = profile();
        p.rules.push(TerminologyRule {
            id: "longer".into(),
            canonical: "가상 도구 모음".into(),
            source_forms: vec!["다른 도구 모음".into()],
            kind: TerminologyKind::Contextual,
            context_cues: vec!["설계".into()],
            exclusion_cues: vec![],
            description: String::new(),
        });
        assert!(
            discover_candidates(&p, &transcript(&["설계 가상 도구 모음, 가상도구와 ABC"]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn overlapping_interpretations_and_unknown_duplicate_decisions_are_rejected() {
        let mut p = profile();
        p.rules.push(TerminologyRule {
            id: "overlap".into(),
            canonical: "장치".into(),
            source_forms: vec!["도구".into()],
            kind: TerminologyKind::Contextual,
            context_cues: vec!["설계".into()],
            exclusion_cues: vec![],
            description: String::new(),
        });
        assert!(discover_candidates(&p, &transcript(&["설계 가상 도구"]))
            .unwrap()
            .is_empty());
        let mut draft = accepted(transcript(&["A B C"]), profile());
        draft.decisions.push(draft.decisions[0].clone());
        assert!(prepare(&draft).is_err());
        draft.decisions = vec![TerminologyDecision {
            candidate_id: "unknown".into(),
            outcome: TerminologyOutcome::Accept,
        }];
        assert!(prepare(&draft).is_err());
    }

    #[test]
    fn arbitrary_acronym_substitution_and_unknown_profile_fields_are_invalid() {
        let mut p = profile();
        p.rules[1].source_forms = vec!["A B D".into()];
        assert!(p.validate().is_err());
        let mut value = serde_json::to_value(profile()).unwrap();
        value["confidence"] = 0.9.into();
        assert!(serde_json::from_value::<TerminologyProfile>(value).is_err());
    }

    #[test]
    fn a_longer_spaced_acronym_does_not_get_partially_compacted() {
        for text in ["A B C D", "X A B C", "A B C D는", "X A B C Y"] {
            assert!(discover_candidates(&profile(), &transcript(&[text]))
                .unwrap()
                .is_empty());
        }
        assert_eq!(
            discover_candidates(&profile(), &transcript(&["ABC 도구와 A B C는 유지"]))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn literal_evidence_requires_selected_nearby_words_not_embedded_cues() {
        let mut source = transcript(&["설계 검토", "가상 도구", "관련 없는 말"]);
        let candidate = discover_candidates(&profile(), &source).unwrap().remove(0);
        assert!(has_literal_evidence(
            &candidate,
            &source,
            &[source.segments[0].id.clone()]
        ));
        assert!(!has_literal_evidence(
            &candidate,
            &source,
            &[source.segments[2].id.clone()]
        ));
        source.segments[0].text = "설계안 검토".into();
        assert!(!has_literal_evidence(
            &candidate,
            &source,
            &[source.segments[0].id.clone()]
        ));
        source.segments[0].text = "설계는 유지".into();
        assert!(has_literal_evidence(
            &candidate,
            &source,
            &[source.segments[0].id.clone()]
        ));
    }

    #[test]
    fn literal_evidence_ignores_source_span_and_invalid_candidate_addresses() {
        let source = transcript(&["가상 도구", "설계 검토"]);
        let mut candidate = discover_candidates(&profile(), &source).unwrap().remove(0);
        candidate.context_cues = vec!["가상 도구".into()];
        let ids = vec![source.segments[0].id.clone(), source.segments[1].id.clone()];
        assert!(!has_literal_evidence(&candidate, &source, &ids));
        candidate.context_cues = vec!["설계".into()];
        assert!(has_literal_evidence(&candidate, &source, &ids));
        candidate.byte_start = 1;
        assert!(!has_literal_evidence(&candidate, &source, &ids));
        candidate.byte_start = 0;
        candidate.segment_index = usize::MAX;
        assert!(!has_literal_evidence(&candidate, &source, &ids));
    }

    #[test]
    fn automatic_processing_never_revisits_edits_or_noninitial_revisions() {
        let mut source = transcript(&["A B C"]);
        source.segments[0].text = "직접 수정한 A B C".into();
        assert!(discover_candidates(&profile(), &source).unwrap().is_empty());
        source.segments[0].text = source.segments[0].original_text.clone();
        source.version.revision = 1;
        assert!(discover_candidates(&profile(), &source).unwrap().is_empty());
    }
}
