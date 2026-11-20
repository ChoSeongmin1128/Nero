use crate::{Meeting, Transcript};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    pub context_chars: usize,
    pub excerpt_chars: usize,
    pub matches_per_meeting: usize,
    pub speaker_names: bool,
}

impl SearchOptions {
    pub const SCREEN: Self = Self {
        context_chars: 32,
        excerpt_chars: 100,
        matches_per_meeting: 1,
        speaker_names: true,
    };
    pub const AGENT: Self = Self {
        context_chars: 40,
        excerpt_chars: 160,
        matches_per_meeting: 20,
        speaker_names: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchField {
    Title,
    Notes,
    Transcript,
    Speaker,
    Summary,
}

impl SearchField {
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Notes => "notes",
            Self::Transcript => "transcript",
            Self::Speaker => "speaker",
            Self::Summary => "summary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub field: SearchField,
    pub excerpt: String,
    // The transcript segment: the matching one, or a matching speaker's first.
    pub segment: Option<usize>,
}

pub fn fold_query(query: &str) -> String {
    query.trim().to_lowercase()
}

// Fields are searched in order: the meeting's title and notes, its transcript, then its
// summary. A caller reads each document only while `full` is false.
pub struct MeetingSearch<'a> {
    query: &'a str,
    options: SearchOptions,
    pub matches: Vec<SearchMatch>,
    pub truncated: bool,
}

impl<'a> MeetingSearch<'a> {
    pub fn new(query: &'a str, options: SearchOptions) -> Self {
        Self {
            query,
            options,
            matches: Vec::new(),
            truncated: false,
        }
    }

    pub fn full(&self) -> bool {
        self.matches.len() >= self.options.matches_per_meeting
    }

    pub fn meeting(&mut self, meeting: &Meeting) {
        self.field(SearchField::Title, &meeting.title, None);
        self.field(SearchField::Notes, &meeting.notes, None);
    }

    pub fn transcript(&mut self, transcript: &Transcript) {
        for (index, segment) in transcript.segments.iter().enumerate() {
            if !self.field(SearchField::Transcript, &segment.text, Some(index)) {
                return;
            }
        }
        if !self.options.speaker_names {
            return;
        }
        for speaker in &transcript.speakers {
            let first = transcript
                .segments
                .iter()
                .position(|segment| segment.speaker_id.as_deref() == Some(speaker.id.as_str()));
            if !self.field(SearchField::Speaker, &speaker.name, first) {
                return;
            }
        }
    }

    pub fn summary(&mut self, text: &str) {
        self.field(SearchField::Summary, text, None);
    }

    // False once a match had to be left out.
    fn field(&mut self, field: SearchField, text: &str, segment: Option<usize>) -> bool {
        let Some(excerpt) = excerpt(text, self.query, self.options) else {
            return true;
        };
        if self.full() {
            self.truncated = true;
            return false;
        }
        self.matches.push(SearchMatch {
            field,
            excerpt,
            segment,
        });
        true
    }
}

// The text around the first match of the folded query, in characters, so Korean text
// is never split. None when the text does not contain it.
pub fn excerpt(text: &str, query: &str, options: SearchOptions) -> Option<String> {
    let offset = text.to_lowercase().find(query)?;
    let chars: Vec<char> = text.chars().collect();
    let mut folded_bytes = 0;
    let start = chars
        .iter()
        .position(|c| {
            folded_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
            folded_bytes > offset
        })
        .unwrap_or(0)
        .saturating_sub(options.context_chars);
    let end = start.saturating_add(options.excerpt_chars).min(chars.len());
    Some(format!(
        "{}{}{}",
        if start > 0 { "..." } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "..." } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TranscriptSegment, TranscriptSpeaker, TranscriptVersion};

    #[test]
    fn excerpts_keep_each_callers_window() {
        let long = format!("{}İstanbul 회의 결정{}", "가".repeat(50), "나".repeat(200));
        assert_eq!(
            excerpt(&long, "회의", SearchOptions::SCREEN).unwrap(),
            format!(
                "...{}İstanbul 회의 결정{}...",
                "가".repeat(23),
                "나".repeat(63)
            )
        );
        assert_eq!(
            excerpt(&long, "회의", SearchOptions::AGENT).unwrap(),
            format!(
                "...{}İstanbul 회의 결정{}...",
                "가".repeat(31),
                "나".repeat(115)
            )
        );
        assert_eq!(
            excerpt("Weekly SYNC notes", "sync", SearchOptions::SCREEN).unwrap(),
            "Weekly SYNC notes"
        );
        assert_eq!(excerpt("Weekly", "sync", SearchOptions::AGENT), None);
        assert_eq!(excerpt("", "", SearchOptions::SCREEN).unwrap(), "");
    }

    fn transcript() -> Transcript {
        let segment = |index: usize, text: &str, speaker: &str| TranscriptSegment {
            id: format!("segment-{index}"),
            start: index as f64,
            end: index as f64 + 1.0,
            original_text: text.into(),
            text: text.into(),
            speaker_id: Some(speaker.into()),
            source_index: Some(index as u32),
        };
        Transcript {
            version: TranscriptVersion {
                id: "transcript".into(),
                revision: 0,
            },
            source_session_id: "session".into(),
            model: "fixture".into(),
            model_revision: "1".into(),
            source: None,
            speakers: vec![TranscriptSpeaker {
                id: "b".into(),
                name: "예산 담당".into(),
                name_origin: None,
            }],
            segments: vec![
                segment(0, "인사", "a"),
                segment(1, "예산 확인", "b"),
                segment(2, "예산 승인", "b"),
            ],
            speaker_analysis: None,
        }
    }

    #[test]
    fn the_screen_takes_the_first_match_and_the_agent_every_match_up_to_its_limit() {
        let transcript = transcript();
        let mut screen = MeetingSearch::new("예산", SearchOptions::SCREEN);
        screen.transcript(&transcript);
        assert_eq!(screen.matches.len(), 1);
        assert_eq!(screen.matches[0].field, SearchField::Transcript);
        assert_eq!(screen.matches[0].segment, Some(1));
        assert!(screen.full());

        let mut speaker = MeetingSearch::new("담당", SearchOptions::SCREEN);
        speaker.transcript(&transcript);
        assert_eq!(speaker.matches[0].field, SearchField::Speaker);
        assert_eq!(speaker.matches[0].segment, Some(1));
        let mut agent = MeetingSearch::new("담당", SearchOptions::AGENT);
        agent.transcript(&transcript);
        assert!(agent.matches.is_empty());

        let limited = SearchOptions {
            matches_per_meeting: 2,
            ..SearchOptions::AGENT
        };
        let mut agent = MeetingSearch::new("예산", limited);
        agent.transcript(&transcript);
        agent.summary("예산 요약");
        assert_eq!(agent.matches.len(), 2);
        assert!(agent.truncated);
        let mut agent = MeetingSearch::new("예산", SearchOptions::AGENT);
        agent.transcript(&transcript);
        agent.summary("예산 요약");
        assert_eq!(
            agent
                .matches
                .iter()
                .map(|found| (found.field, found.segment))
                .collect::<Vec<_>>(),
            [
                (SearchField::Transcript, Some(1)),
                (SearchField::Transcript, Some(2)),
                (SearchField::Summary, None)
            ]
        );
        assert!(!agent.truncated);
    }
}
