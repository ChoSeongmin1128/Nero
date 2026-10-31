//! Native token bytes and timestamp projection. No process or PCM ownership policy.
use super::{failure, RATE};
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

// Native -ojf may contain BPE fragments that are individually invalid UTF-8.
// Preserve bytes through JSON unescaping and validate only reconstructed text.
fn text_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    struct Bytes;
    impl<'de> Visitor<'de> for Bytes {
        type Value = Vec<u8>;
        fn expecting(&self, out: &mut std::fmt::Formatter) -> std::fmt::Result {
            out.write_str("native text bytes")
        }
        fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<Self::Value, E> {
            Ok(bytes.to_vec())
        }
        fn visit_borrowed_bytes<E: Error>(self, bytes: &'de [u8]) -> Result<Self::Value, E> {
            Ok(bytes.to_vec())
        }
        fn visit_byte_buf<E: Error>(self, bytes: Vec<u8>) -> Result<Self::Value, E> {
            Ok(bytes)
        }
    }
    deserializer.deserialize_bytes(Bytes)
}

#[derive(Deserialize)]
struct Offset {
    from: i64,
    to: i64,
}
#[derive(Deserialize)]
struct Token {
    #[serde(deserialize_with = "text_bytes")]
    text: Vec<u8>,
    offsets: Offset,
    id: u32,
}
#[derive(Deserialize)]
struct Segment {
    #[serde(deserialize_with = "text_bytes")]
    text: Vec<u8>,
    offsets: Offset,
    tokens: Vec<Token>,
}
#[derive(Deserialize)]
struct Native {
    model: Value,
    params: Value,
    transcription: Vec<Segment>,
}

fn within(offset: &Offset, frames: u64) -> bool {
    offset.from >= 0
        && offset.to >= 0
        && (offset.from as u64)
            .checked_mul(16)
            .is_some_and(|value| value <= frames)
        && (offset.to as u64)
            .checked_mul(16)
            .is_some_and(|value| value <= frames)
}

pub(super) fn native_words(raw: &[u8], frames: u64, start: u64) -> Result<Vec<Value>, String> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(failure());
    }
    let native: Native = serde_json::from_slice(raw).map_err(|_| failure())?;
    if native.params["language"] != "ko"
        || native.params["translate"] != false
        || native.model["multilingual"] != true
        || native.model["mels"] != 128
        || native.model["text"]["layer"] != 4
        || native.model["ftype"] != 8
    {
        return Err(failure());
    }
    let mut words = Vec::new();
    let segment_count = native.transcription.len();
    for (index, segment) in native.transcription.into_iter().enumerate() {
        // b5454 cli.cpp::times_o serializes native centiseconds as t * 10 ms.
        // Only its final segment may meet the next 10ms tick after exact PCM EOF.
        // This is retained as segment context, never normalized word timing.
        let terminal_tick = frames.div_ceil(160).checked_mul(160).ok_or_else(failure)?;
        let terminal_context = index + 1 == segment_count
            && terminal_tick > frames
            && terminal_tick - frames < 160
            && segment.offsets.from >= 0
            && segment.offsets.from % 10 == 0
            && segment.offsets.to % 10 == 0
            && (segment.offsets.from as u64)
                .checked_mul(16)
                .is_some_and(|v| v <= frames)
            && u64::try_from(segment.offsets.to)
                .ok()
                .and_then(|v| v.checked_mul(16))
                == Some(terminal_tick);
        if (!within(&segment.offsets, frames) && !terminal_context)
            || segment.offsets.from > segment.offsets.to
        {
            return Err(failure());
        }
        let tokens = segment
            .tokens
            .iter()
            .filter(|token| token.id < 50257)
            .collect::<Vec<_>>();
        let bytes = tokens
            .iter()
            .flat_map(|token| token.text.iter().copied())
            .collect::<Vec<_>>();
        let reconstructed = std::str::from_utf8(&bytes).map_err(|_| failure())?;
        let text = std::str::from_utf8(&segment.text).map_err(|_| failure())?;
        if reconstructed.trim() != text.trim() {
            return Err(failure());
        }
        if text.trim().is_empty() {
            continue;
        }
        if terminal_context {
            if tokens.iter().any(|token| {
                !within(&token.offsets, terminal_tick)
                    || token.offsets.from % 10 != 0
                    || token.offsets.to % 10 != 0
            }) {
                return Err(failure());
            }
            words.push(json!({"word":text.trim(),
                "start":(start + segment.offsets.from as u64 * 16) as f64 / RATE as f64,
                "end":(start + frames) as f64 / RATE as f64,
                "timingScope":"nativeSegmentContext",
                "nativeSegmentEndFrame":start.checked_add(terminal_tick).ok_or_else(failure)?}));
            continue;
        }
        let mut groups: Vec<Vec<&Token>> = Vec::new();
        for token in &tokens {
            if groups.is_empty() || token.text.first().is_some_and(u8::is_ascii_whitespace) {
                groups.push(Vec::new());
            }
            groups.last_mut().unwrap().push(token);
        }
        let mut projected = Vec::new();
        let mut timed = true;
        for group in groups {
            let bytes = group
                .iter()
                .flat_map(|token| token.text.iter().copied())
                .collect::<Vec<_>>();
            let word = std::str::from_utf8(&bytes).map_err(|_| failure())?.trim();
            if word.is_empty() {
                continue;
            }
            let meaningful = group
                .iter()
                .filter(|token| !token.text.iter().all(u8::is_ascii_whitespace))
                .collect::<Vec<_>>();
            let from = meaningful.first().ok_or_else(failure)?.offsets.from;
            let to = meaningful.last().ok_or_else(failure)?.offsets.to;
            // Keep native first/last anchors. Reversed/absent anchors never get
            // repaired by nearest-token lookup or proportional interpolation.
            if from > to
                || meaningful
                    .iter()
                    .any(|token| !within(&token.offsets, frames))
            {
                timed = false;
                break;
            }
            projected.push(
                json!({"word":word,"start":(start + from as u64 * 16) as f64 / RATE as f64,
                "end":(start + to as u64 * 16) as f64 / RATE as f64}),
            );
        }
        if timed {
            if projected.is_empty() {
                return Err(failure());
            }
            words.extend(projected);
        } else {
            // This is one native segment, explicitly NOT word timing. Preserve
            // all its text and actual context bounds, and leave its speaker unknown.
            words.push(json!({"word":text.trim(),"start":(start + segment.offsets.from as u64 * 16) as f64 / RATE as f64,
                "end":(start + segment.offsets.to as u64 * 16) as f64 / RATE as f64,
                "timingScope":"nativeSegmentContext"}));
        }
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(text: &[u8], tokens: &[(&[u8], i64, i64)]) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(
            &json!({"model":{"multilingual":true,"mels":128,"text":{"layer":4},"ftype":8},
            "params":{"language":"ko","translate":false}}),
        )
        .unwrap();
        bytes.pop();
        bytes.extend_from_slice(b",\"transcription\":[{\"text\":\"");
        bytes.extend_from_slice(text);
        bytes.extend_from_slice(b"\",\"offsets\":{\"from\":0,\"to\":1000},\"tokens\":[");
        for (index, (token, from, to)) in tokens.iter().enumerate() {
            if index > 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(b"{\"text\":\"");
            bytes.extend_from_slice(token);
            bytes.extend_from_slice(
                format!(
                    "\",\"id\":{},\"offsets\":{{\"from\":{from},\"to\":{to}",
                    index + 1
                )
                .as_bytes(),
            );
            bytes.extend_from_slice(b"}}");
        }
        bytes.extend_from_slice(b"]}]}");
        bytes
    }

    #[test]
    fn split_korean_token_bytes_reconstruct_before_utf8_validation() {
        let syllable = "가".as_bytes();
        let raw = native(
            syllable,
            &[(&syllable[..2], 0, 500), (&syllable[2..], 500, 1000)],
        );
        assert!(std::str::from_utf8(&raw).is_err());
        let words = native_words(&raw, 16000, 32000).unwrap();
        assert_eq!(words, vec![json!({"word":"가","start":2.0,"end":3.0})]);
    }

    #[test]
    fn incomplete_generated_utf8_or_text_mismatch_is_not_repaired() {
        let broken = &[0xeb, 0xb7];
        assert!(native_words(&native(broken, &[(broken, 0, 1000)]), 16000, 0).is_err());
        assert!(native_words(
            &native("가".as_bytes(), &[("나".as_bytes(), 0, 1000)]),
            16000,
            0
        )
        .is_err());
    }

    #[test]
    fn zero_native_interval_is_preserved_without_invented_duration() {
        let words = native_words(
            &native("가".as_bytes(), &[("가".as_bytes(), 500, 500)]),
            16000,
            0,
        )
        .unwrap();
        assert_eq!(words[0]["start"], 0.5);
        assert_eq!(words[0]["end"], 0.5);
        assert!(words[0].get("confidence").is_none());
    }

    #[test]
    fn reversed_word_anchors_keep_the_whole_native_segment_as_context() {
        let words = native_words(
            &native("가".as_bytes(), &[("가".as_bytes(), 700, 300)]),
            16000,
            0,
        )
        .unwrap();
        assert_eq!(
            words,
            vec![json!({"word":"가","start":0.0,"end":1.0,"timingScope":"nativeSegmentContext"})]
        );
        let raw = native("가".as_bytes(), &[("가".as_bytes(), 0, 1000)]);
        assert!(native_words(&raw, 15840, 0).is_err()); // one whole extra tick, not sub-tick EOF.
    }

    #[test]
    fn a_whitespace_token_does_not_supply_a_word_timestamp() {
        let words = native_words(
            &native(
                " 가".as_bytes(),
                &[(b" ", 0, 100), ("가".as_bytes(), 500, 1000)],
            ),
            16000,
            0,
        )
        .unwrap();
        assert_eq!(words[0]["start"], 0.5);
    }

    fn terminal_fixture() -> Value {
        let mut value: Value =
            serde_json::from_slice(&native("가".as_bytes(), &[("가".as_bytes(), 68000, 69140)]))
                .unwrap();
        value["transcription"][0]["offsets"] = json!({"from":68000,"to":69140});
        value
    }

    #[test]
    fn final_sub_centisecond_overrun_is_audited_context_not_word_timing() {
        let raw = serde_json::to_vec(&terminal_fixture()).unwrap();
        let original = raw.clone();
        let words = native_words(&raw, 1_106_171, 1_773_829).unwrap();
        assert_eq!(raw, original);
        assert_eq!(
            words,
            vec![json!({"word":"가","start":2_861_829_f64/16000.0,
            "end":180.0,"timingScope":"nativeSegmentContext","nativeSegmentEndFrame":2_880_069_u64})]
        );
        assert!(words[0].get("confidence").is_none());
        // One sample short of a full tick is also explicit context, never a
        // fabricated positive-duration native word.
        let raw = native("가".as_bytes(), &[("가".as_bytes(), 500, 1000)]);
        assert_eq!(
            native_words(&raw, 15999, 0).unwrap()[0]["nativeSegmentEndFrame"],
            16000
        );
    }

    #[test]
    fn terminal_context_does_not_relax_other_native_time_or_text_failures() {
        for (pointer, value) in [
            ("/transcription/0/offsets/to", json!(69150)),
            ("/transcription/0/offsets/to", json!(78140)),
            ("/transcription/0/offsets/from", json!(-10)),
            ("/transcription/0/offsets/from", json!(69140)),
            ("/transcription/0/tokens/0/offsets/to", json!(69150)),
            ("/transcription/0/tokens/0/offsets/to", json!(69139)),
        ] {
            let mut raw = terminal_fixture();
            *raw.pointer_mut(pointer).unwrap() = value;
            assert!(
                native_words(&serde_json::to_vec(&raw).unwrap(), 1_106_171, 0).is_err(),
                "{pointer}"
            );
        }
        let mut raw = terminal_fixture();
        raw["transcription"]
            .as_array_mut()
            .unwrap()
            .push(json!({"text":"","offsets":{"from":0,"to":0},"tokens":[]}));
        assert!(native_words(&serde_json::to_vec(&raw).unwrap(), 1_106_171, 0).is_err());
        // An aligned input has no partial terminal tick to normalize.
        assert!(native_words(
            &serde_json::to_vec(&terminal_fixture()).unwrap(),
            1_106_080,
            0
        )
        .is_err());
        let broken = &[0xeb, 0xb7];
        assert!(native_words(&native(broken, &[(broken, 0, 1000)]), 15999, 0).is_err());
    }
}
