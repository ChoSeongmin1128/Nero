use crate::word_speakers::{
    self, QWEN_WORD_PROJECTOR_VERSION, WHISPER_WORD_PROJECTOR_VERSION, WORD_PROJECTOR_VERSION,
};
use crate::{apple_speech, nemotron, qwen, whisper, Result, Transcript};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrProvider {
    MossChunks,
    Nemotron,
    Whisper,
    Qwen,
    AppleSpeech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Projection {
    Sentences,
    WordSpeakers,
}

type Decoder = fn(&[u8], f64, &str, &str) -> Result<Transcript>;

const ALL: [AsrProvider; 5] = [
    AsrProvider::MossChunks,
    AsrProvider::Nemotron,
    AsrProvider::Whisper,
    AsrProvider::Qwen,
    AsrProvider::AppleSpeech,
];

// The pinned Nemotron repository, read from the same manifest its output is checked against.
// None when the manifest names none, so no model, the empty one included, matches it.
fn nemotron_model() -> Option<&'static str> {
    static MODEL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str::<serde_json::Value>(include_str!(
                "../../../config/models/nemotron-asr.json"
            ))
            .ok()
            .and_then(|manifest| manifest["repository"].as_str().map(str::to_owned))
            .filter(|repository| !repository.is_empty())
        })
        .as_deref()
}

impl AsrProvider {
    // MOSS output is told apart by its format, not by a model name.
    pub(crate) fn of_model(model: &str) -> Option<Self> {
        if model == apple_speech::MODEL || model == apple_speech::DICTATION_MODEL {
            Some(Self::AppleSpeech)
        } else if model == whisper::MODEL {
            Some(Self::Whisper)
        } else if model == qwen::MODEL {
            Some(Self::Qwen)
        } else if Some(model) == nemotron_model() {
            Some(Self::Nemotron)
        } else {
            None
        }
    }

    // MOSS chunk projectors have their own versions in the MOSS reader.
    pub(crate) fn of_projector(version: &str) -> Option<(Self, Projection)> {
        ALL.into_iter().find_map(|provider| {
            if provider.sentence_projector() == Some(version) {
                Some((provider, Projection::Sentences))
            } else if provider.word_projector() == Some(version) {
                Some((provider, Projection::WordSpeakers))
            } else {
                None
            }
        })
    }

    fn sentence_projector(self) -> Option<&'static str> {
        match self {
            Self::MossChunks => None,
            Self::Nemotron => Some(nemotron::PROJECTOR_VERSION),
            Self::Whisper => Some(whisper::PROJECTOR_VERSION),
            Self::Qwen => Some(qwen::PROJECTOR_VERSION),
            Self::AppleSpeech => Some(apple_speech::PROJECTOR),
        }
    }

    pub(crate) fn word_projector(self) -> Option<&'static str> {
        match self {
            Self::Nemotron => Some(WORD_PROJECTOR_VERSION),
            Self::Whisper => Some(WHISPER_WORD_PROJECTOR_VERSION),
            Self::Qwen => Some(QWEN_WORD_PROJECTOR_VERSION),
            Self::MossChunks | Self::AppleSpeech => None,
        }
    }

    // Words with native times, so speakers can be assigned word by word.
    pub(crate) fn word_speakers(self) -> bool {
        self.word_projector().is_some()
    }

    // Whisper and the Qwen aligner can emit zero-length words; those keep an unknown speaker.
    pub(crate) fn zero_length_unknown(self) -> bool {
        matches!(self, Self::Whisper | Self::Qwen)
    }

    // Only Whisper marks words that carry their native segment's context instead of times.
    pub(crate) fn native_segment_context(self) -> bool {
        self == Self::Whisper
    }

    // Recognizers read against the recording's duration; MOSS chunks need its frames.
    pub(crate) fn decoder(self) -> Option<Decoder> {
        match self {
            Self::MossChunks => None,
            Self::Nemotron => Some(nemotron::decode_asr),
            Self::Whisper => Some(whisper::decode_asr),
            Self::Qwen => Some(qwen::decode_asr),
            Self::AppleSpeech => Some(apple_speech::decode),
        }
    }
}

pub(crate) fn word_speaker_projector(version: &str) -> bool {
    matches!(
        AsrProvider::of_projector(version),
        Some((_, Projection::WordSpeakers))
    )
}

fn routing(raw: &[u8]) -> Result<serde_json::Value> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(word_speakers::derived_invalid());
    }
    serde_json::from_slice(raw).map_err(|_| word_speakers::derived_invalid())
}

// The decoder follows the model the output names. Any other model is read as Nemotron
// output, which then refuses it.
fn native_decoder(value: &serde_json::Value) -> Decoder {
    value["model"]
        .as_str()
        .and_then(AsrProvider::of_model)
        .and_then(AsrProvider::decoder)
        .unwrap_or(nemotron::decode_asr)
}

pub(crate) fn decode_native(
    raw: &[u8],
    duration: f64,
    session: &str,
    id: &str,
) -> Result<Transcript> {
    let decode = native_decoder(&routing(raw)?);
    decode(raw, duration, session, id)
}

// A stored generation of a word-timed recognizer: its native output, or a speaker
// generation derived from it. The routing tree is parsed once and dropped before the
// typed decoder reads the output.
pub(crate) fn decode_stored(
    raw: &[u8],
    duration: f64,
    session_id: &str,
    id: &str,
) -> Result<Transcript> {
    let value = routing(raw)?;
    if value["schemaVersion"] == 2 {
        drop(value);
        return word_speakers::decode_derived(raw, duration, session_id, id);
    }
    let decode = native_decoder(&value);
    drop(value);
    decode(raw, duration, session_id, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_and_projectors_keep_their_providers_and_fallbacks() {
        assert_eq!(
            AsrProvider::of_model(apple_speech::DICTATION_MODEL),
            Some(AsrProvider::AppleSpeech)
        );
        let pinned = serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../config/models/nemotron-asr.json"
        ))
        .unwrap()["repository"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!pinned.is_empty());
        assert_eq!(AsrProvider::of_model(&pinned), Some(AsrProvider::Nemotron));
        assert_eq!(AsrProvider::of_model(""), None);
        assert_eq!(
            AsrProvider::of_model("OpenMOSS-Team/MOSS-Transcribe-Diarize"),
            None
        );
        let projectors = [
            (
                "nemotron-asr-sentences-v1",
                AsrProvider::Nemotron,
                Projection::Sentences,
            ),
            (
                "nemotron-word-speakers-v1",
                AsrProvider::Nemotron,
                Projection::WordSpeakers,
            ),
            (
                "whisper-native-token-sentences-v1",
                AsrProvider::Whisper,
                Projection::Sentences,
            ),
            (
                "whisper-word-speakers-v1",
                AsrProvider::Whisper,
                Projection::WordSpeakers,
            ),
            (
                "qwen3-aligned-words-sentences-v1",
                AsrProvider::Qwen,
                Projection::Sentences,
            ),
            (
                "qwen3-word-speakers-v1",
                AsrProvider::Qwen,
                Projection::WordSpeakers,
            ),
            (
                "apple-native-results-v1",
                AsrProvider::AppleSpeech,
                Projection::Sentences,
            ),
        ];
        for (version, provider, projection) in projectors {
            assert_eq!(
                AsrProvider::of_projector(version),
                Some((provider, projection))
            );
        }
        for version in [
            "moss-chunk-speakers-v1",
            "moss-recording-speakers-v1",
            "moss-independent-chunks-projector-v3",
        ] {
            assert_eq!(AsrProvider::of_projector(version), None);
        }
        let word_speakers: Vec<_> = ALL.into_iter().filter(|p| p.word_speakers()).collect();
        assert_eq!(
            word_speakers,
            [
                AsrProvider::Nemotron,
                AsrProvider::Whisper,
                AsrProvider::Qwen
            ]
        );
    }
}
