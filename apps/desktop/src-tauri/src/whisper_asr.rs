//! Source-bound PCM windows and native token-byte projection; no timestamp interpolation.
use crate::processing::{cancelled, plain_file};
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) const MANIFEST: &str = include_str!("../../../../config/models/whisper-turbo.json");
const RUNTIME: &str = include_str!("../../../../scripts/native-whisper/runtime.json");
const RATE: u64 = 16000;
const CONTEXT: u64 = 3 * RATE;
const OWNED: u64 = 120 * RATE;
const MIN_OWNED: u64 = 90 * RATE;

pub(super) fn executable(resources: &Path) -> PathBuf {
    resources.join("native/whisper/bin").join(if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    })
}
pub(super) fn supported(resources: &Path) -> bool {
    (cfg!(all(target_os = "windows", target_arch = "x86_64"))
        || cfg!(all(target_os = "macos", target_arch = "aarch64")))
        && executable(resources).is_file()
}

fn failure() -> String {
    "Whisper의 원본 글자나 시각을 확인하지 못했습니다. 원음과 기존 기록은 유지했습니다.".into()
}

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

fn native_words(raw: &[u8], frames: u64, start: u64) -> Result<Vec<Value>, String> {
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
    for segment in native.transcription {
        if !within(&segment.offsets, frames) || segment.offsets.from > segment.offsets.to {
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

fn save(path: &Path, value: &Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    plain_file(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(failure());
    }
    Ok(bytes)
}

fn file_hash(path: &Path) -> Result<String, String> {
    crate::speech_audio::digest(path).map_err(|error| error.message)
}

pub(super) struct Buffered {
    cli: PathBuf,
    model: PathBuf,
    cli_sha: String,
    model_sha: String,
    work: PathBuf,
    samples: VecDeque<i16>,
    base: u64,
    received: u64,
    owned: u64,
    hash: Sha256,
    words: Vec<Value>,
    windows: Vec<Value>,
    pub(super) generation: u64,
    inherited_tree: bool,
}

impl Buffered {
    pub(super) fn new(
        cli: PathBuf,
        model: PathBuf,
        work: PathBuf,
        inherited_tree: bool,
    ) -> Result<Self, String> {
        let manifest: Value = serde_json::from_str(MANIFEST).map_err(|_| failure())?;
        let model_sha = file_hash(&model)?;
        if Some(model_sha.as_str()) != manifest["files"][0]["sha256"].as_str()
            || plain_file(&model)?
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                != manifest["files"][0]["bytes"].as_u64().ok_or_else(failure)?
        {
            return Err("Whisper 모델 파일을 다시 준비해 주세요.".into());
        }
        let cli_sha = file_hash(&cli)?;
        Ok(Self {
            cli,
            model,
            cli_sha,
            model_sha,
            work,
            samples: VecDeque::new(),
            base: 0,
            received: 0,
            owned: 0,
            hash: Sha256::new(),
            words: Vec::new(),
            windows: Vec::new(),
            generation: 0,
            inherited_tree,
        })
    }

    pub(super) fn push(&mut self, samples: &[i16], cancel: &AtomicBool) -> Result<(), String> {
        cancelled(cancel)?;
        for sample in samples {
            self.samples.push_back(*sample);
            self.hash.update(sample.to_le_bytes());
        }
        self.received = self
            .received
            .checked_add(samples.len() as u64)
            .ok_or_else(failure)?;
        while self.received >= self.owned + OWNED + CONTEXT {
            let target = self.owned + OWNED;
            let mut cut = target;
            let mut quiet = 0_u64;
            for position in self.owned + MIN_OWNED..target {
                let sample = self.samples[(position - self.base) as usize];
                if i32::from(sample).abs() < 328 {
                    quiet += 1;
                } else {
                    quiet = 0;
                }
                if quiet >= RATE / 3 {
                    cut = position + 1;
                }
            }
            self.window(cut, cut + CONTEXT, cancel)?;
        }
        Ok(())
    }

    fn window(
        &mut self,
        owned_end: u64,
        input_end: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        cancelled(cancel)?;
        let input_start = self.owned.saturating_sub(CONTEXT);
        if input_start < self.base
            || input_end > self.received
            || input_end <= input_start
            || input_end - input_start > 180 * RATE
        {
            return Err(failure());
        }
        let mut input = tempfile::Builder::new()
            .prefix("whisper-window-")
            .suffix(".wav")
            .tempfile_in(&self.work)
            .map_err(|e| e.to_string())?;
        let mut writer = hound::WavWriter::new(
            input.as_file_mut(),
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut pcm = Sha256::new();
        for sample in self
            .samples
            .iter()
            .skip((input_start - self.base) as usize)
            .take((input_end - input_start) as usize)
        {
            writer.write_sample(*sample).map_err(|e| e.to_string())?;
            pcm.update(sample.to_le_bytes());
        }
        writer.finalize().map_err(|e| e.to_string())?;
        input.as_file().sync_all().map_err(|e| e.to_string())?;
        let index = self.windows.len();
        let prefix = self.work.join(format!("whisper-window-{index}"));
        let native_path = prefix.with_extension("json");
        if native_path.exists() {
            return Err("Whisper 원본 결과 경로가 이미 있습니다.".into());
        }
        let run = run_cli(
            &self.cli,
            &self.model,
            input.path(),
            &prefix,
            &self.work,
            cancel,
            self.inherited_tree,
        )?;
        let raw = read(&run.native, 16 * 1024 * 1024)?;
        let projected = native_words(&raw, input_end - input_start, input_start)?;
        let begin = self.words.len();
        for word in projected {
            let center = ((word["start"].as_f64().ok_or_else(failure)? * RATE as f64).round()
                + (word["end"].as_f64().ok_or_else(failure)? * RATE as f64).round())
                * 0.5;
            if center >= self.owned as f64
                && (center < owned_end as f64
                    || (owned_end == self.received && center == owned_end as f64))
            {
                self.words.push(word);
            }
        }
        let receipt = json!({"index":index,"inputStartFrame":input_start,"inputEndFrame":input_end,
            "ownedStartFrame":self.owned,"ownedEndFrame":owned_end,"inputPcmSha256":format!("{:x}",pcm.finalize()),
            "nativeResultSha256":format!("{:x}",Sha256::digest(&raw)),"nativeExitCode":0,
            "actualBackend":run.backend,"backendEvidence":run.evidence,"wordStart":begin,"wordCount":self.words.len()-begin});
        save(
            &self
                .work
                .join(format!("whisper-window-{index}-completion.json")),
            &receipt,
        )?;
        self.windows.push(receipt);
        self.owned = owned_end;
        self.generation += 1;
        let keep = self.owned.saturating_sub(CONTEXT);
        self.samples.drain(..(keep - self.base) as usize);
        self.base = keep;
        if file_hash(&self.cli)? != self.cli_sha || file_hash(&self.model)? != self.model_sha {
            return Err("Whisper 실행기나 모델이 처리 중 변경됐습니다.".into());
        }
        Ok(())
    }

    pub(super) fn preview(&self) -> Value {
        json!({"text":self.words.iter().filter_map(|w|w["word"].as_str()).collect::<Vec<_>>().join(" "),"words":self.words})
    }

    pub(super) fn finish(
        &mut self,
        frames: u64,
        pcm_sha: &str,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        if frames != self.received
            || frames == 0
            || format!("{:x}", self.hash.clone().finalize()) != pcm_sha
        {
            return Err(failure());
        }
        if self.owned < self.received {
            self.window(self.received, self.received, cancel)?;
        }
        let model: Value = serde_json::from_str(MANIFEST).map_err(|_| failure())?;
        let runtime: Value = serde_json::from_str(RUNTIME).map_err(|_| failure())?;
        Ok(
            json!({"schemaVersion":1,"model":model["repository"],"modelRevision":model["revision"],"modelSha256":self.model_sha,
            "runtime":runtime["identity"],"target":if cfg!(windows){"windows-x64"}else{"macos-arm64"},
            "processingPolicy":"whisper-context-windows-v1","completionEvidence":"whisper-cli-windows-exit0-v1",
            "tokenTimestampPolicy":"native-token-span-v1","completed":true,"inputAudioFrames":frames,
            "audioSampleRate":16000,"audioDurationSeconds":frames as f64/RATE as f64,"inputPcmSha256":pcm_sha,
            "windows":self.windows,"words":self.words}),
        )
    }
}

struct Run {
    native: PathBuf,
    backend: &'static str,
    evidence: &'static str,
}

fn run_cli(
    cli: &Path,
    model: &Path,
    audio: &Path,
    prefix: &Path,
    work: &Path,
    cancel: &AtomicBool,
    inherited_tree: bool,
) -> Result<Run, String> {
    let gpu = cfg!(target_os = "macos");
    let state = run_attempt(cli, model, audio, prefix, work, cancel, gpu, inherited_tree)?;
    if state == MetalState::Unavailable {
        let cpu_prefix = prefix.with_file_name(format!(
            "{}-cpu",
            prefix.file_name().ok_or_else(failure)?.to_string_lossy()
        ));
        run_attempt(
            cli,
            model,
            audio,
            &cpu_prefix,
            work,
            cancel,
            false,
            inherited_tree,
        )?;
        Ok(Run {
            native: cpu_prefix.with_extension("json"),
            backend: "cpu",
            evidence: "confirmed-gpu-unavailable",
        })
    } else {
        Ok(Run {
            native: prefix.with_extension("json"),
            backend: if gpu { "metal" } else { "cpu" },
            evidence: if gpu {
                "metal-backend-initialized"
            } else {
                "explicit-no-gpu"
            },
        })
    }
}

fn run_attempt(
    cli: &Path,
    model: &Path,
    audio: &Path,
    prefix: &Path,
    work: &Path,
    cancel: &AtomicBool,
    gpu: bool,
    inherited_tree: bool,
) -> Result<MetalState, String> {
    cancelled(cancel)?;
    let log_path = prefix.with_extension(if gpu { "metal.log" } else { "cpu.log" });
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(cli);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4).max(1));
    command
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-l")
        .arg("ko")
        .args(["-ojf", "-bs", "5", "-bo", "5", "-t"])
        .arg(threads.to_string())
        .arg("-p")
        .arg("1")
        .arg("-of")
        .arg(prefix)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log));
    if !gpu {
        command.arg("-ng");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000);
    }
    let mut child = if inherited_tree {
        crate::process_tree::spawn_inherited(&mut command)
    } else {
        crate::process_tree::spawn(&mut command)
    }?;
    let began = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Whisper 처리를 취소했습니다.".into());
        }
        if began.elapsed() > Duration::from_secs(1800) {
            return Err(
                "Whisper 구간 처리 시간이 너무 오래 걸렸습니다. 원본은 보관했습니다.".into(),
            );
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Whisper 구간 처리를 완료하지 못했습니다. 원본은 보관했습니다.".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let log = read(&log_path, 4 * 1024 * 1024)?;
    let log = String::from_utf8_lossy(&log);
    if gpu {
        // No exit-0/compiled-Metal shortcut: the pinned classifier requires
        // successful device allocation and rejects silent CPU initialization.
        classify_metal(&log)
    } else {
        Ok(MetalState::ExplicitCpu)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MetalState {
    Initialized,
    Unavailable,
    ExplicitCpu,
}

fn classify_metal(log: &str) -> Result<MetalState, String> {
    let lower = log.to_ascii_lowercase();
    if lower.contains("failed") || lower.contains("error:") || lower.contains("out of memory") {
        return Err(
            "Whisper의 Metal 초기화 또는 메모리 확보에 실패했습니다. 원본은 보관했습니다.".into(),
        );
    }
    if log
        .lines()
        .any(|line| line.trim() == "whisper_backend_init_gpu: no GPU found")
    {
        return Ok(MetalState::Unavailable);
    }
    let model = log
        .lines()
        .any(|line| line.starts_with("whisper_model_load:") && line.contains("MTL0 total size ="));
    let backend = log
        .lines()
        .any(|line| line.trim() == "whisper_backend_init_gpu: using MTL0 backend");
    let device = log
        .lines()
        .any(|line| line.starts_with("ggml_metal_init: found device: Apple "));
    let graph = log
        .lines()
        .any(|line| line.trim() == "ggml_metal_init: use graph optimize = true");
    if model && backend && device && graph {
        Ok(MetalState::Initialized)
    } else {
        Err("Whisper의 실제 Metal 실행 여부를 확인하지 못했습니다. 원본은 보관했습니다.".into())
    }
}

pub(super) fn recognize(
    resources: &Path,
    audio: &Path,
    work: &Path,
    cancel: &AtomicBool,
    model_dir: &Path,
) -> Result<Vec<u8>, String> {
    let model: Value = serde_json::from_str(MANIFEST).map_err(|_| failure())?;
    let mut buffered = Buffered::new(
        executable(resources),
        model_dir.join(model["files"][0]["name"].as_str().ok_or_else(failure)?),
        work.to_path_buf(),
        false,
    )?;
    let mut wav = hound::WavReader::new(plain_file(audio)?).map_err(|_| failure())?;
    if wav.spec().channels != 1
        || wav.spec().sample_rate != 16000
        || wav.spec().bits_per_sample != 16
        || wav.spec().sample_format != hound::SampleFormat::Int
    {
        return Err(failure());
    }
    let frames = u64::from(wav.duration());
    let mut samples = wav.samples::<i16>();
    loop {
        let block = samples
            .by_ref()
            .take(16000)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| failure())?;
        if block.is_empty() {
            break;
        }
        buffered.push(&block, cancel)?;
    }
    let sha = format!("{:x}", buffered.hash.clone().finalize());
    let result = buffered.finish(frames, &sha, cancel)?;
    serde_json::to_vec(&result).map_err(|_| failure())
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
        assert!(native_words(&raw, 15999, 0).is_err()); // segment itself exceeds exact PCM.
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

    #[test]
    fn metal_requires_positive_initialization_and_never_hides_allocation_failure() {
        let valid="whisper_model_load:         MTL0 total size = 100.00 MB\nwhisper_backend_init_gpu: using MTL0 backend\nggml_metal_init: found device: Apple Example\nggml_metal_init: use graph optimize = true\n";
        assert_eq!(classify_metal(valid).unwrap(), MetalState::Initialized);
        assert_eq!(
            classify_metal("whisper_backend_init_gpu: no GPU found\n").unwrap(),
            MetalState::Unavailable
        );
        assert!(classify_metal("whisper_backend_init_gpu: using MTL0 backend\n").is_err());
        for failure in [
            "whisper_backend_init_gpu: failed to initialize MTL0 backend",
            "ggml_metal_buffer: failed to allocate Metal buffer",
            "error: failed to allocate context",
            "ggml_metal_library_init: failed",
        ] {
            assert!(classify_metal(&(valid.to_owned() + failure)).is_err());
        }
    }
}
