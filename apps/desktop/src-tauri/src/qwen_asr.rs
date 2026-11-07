// Qwen3-ASR-ko through the pinned CrispASR CLI: pause chunks, gain, aligner words.
use crate::processing::failure::Failure;
use crate::processing::{cancelled, plain_file};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) const MANIFEST: &str = include_str!("../../../../config/models/qwen3-asr-ko.json");
const RUNTIME: &str = include_str!("../../../../scripts/native-crispasr/runtime.json");
const RATE: usize = 16000;
const TARGET_SECONDS: f64 = 30.0;
const FORWARD_SECONDS: f64 = 3.0;
const PAD: usize = RATE;
const MAX_NATIVE_JSON: u64 = 16 * 1024 * 1024;

pub(super) fn executable(resources: &Path) -> PathBuf {
    resources.join("native/crispasr/bin/crispasr")
}

pub(super) fn supported(resources: &Path) -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
        && executable(resources).is_file()
        && resources.join("native/crispasr/runtime.json").is_file()
}

fn failure() -> String {
    "Qwen 전사의 원본 결과나 단어 시각을 확인하지 못했습니다. 원음과 기존 기록은 유지했습니다."
        .into()
}

fn file_hash(path: &Path) -> Result<String, String> {
    crate::speech_audio::digest(path).map_err(|error| error.message)
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

// Longest quiet run between 15 and 33 seconds after each chunk start, as measured.
pub(super) fn pause_split(pcm: &[f32]) -> Vec<(usize, usize)> {
    let hop = RATE * 20 / 1000;
    let count = pcm.len() / hop;
    let energy: Vec<f64> = (0..count)
        .map(|i| {
            let part = &pcm[i * hop..(i + 1) * hop];
            (part
                .iter()
                .map(|x| f64::from(*x) * f64::from(*x))
                .sum::<f64>()
                / hop as f64
                + 1e-12)
                .sqrt()
        })
        .collect();
    let target = (TARGET_SECONDS * RATE as f64) as usize;
    let mut chunks = Vec::new();
    let mut start = 0;
    while pcm.len() - start > target {
        let lo = (start + (TARGET_SECONDS / 2.0 * RATE as f64) as usize) / hop;
        let hi =
            count.min((start + ((TARGET_SECONDS + FORWARD_SECONDS) * RATE as f64) as usize) / hop);
        let window = &energy[lo..hi];
        let threshold = (percentile(window, 20.0) * 1.5)
            .min(percentile(window, 50.0) * 10_f64.powf(-12.0 / 20.0));
        let (mut best, mut middle, mut run) = (0, None, 0);
        for (i, quiet) in window
            .iter()
            .map(|e| *e < threshold)
            .chain(std::iter::once(false))
            .enumerate()
        {
            if quiet {
                run += 1;
                continue;
            }
            if run > best {
                best = run;
                middle = Some(i - run / 2 - 1);
            }
            run = 0;
        }
        let cut = middle.map_or(start + target, |m| (lo + m) * hop);
        chunks.push((start, cut));
        start = cut;
    }
    chunks.push((start, pcm.len()));
    chunks
}

fn percentile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = q / 100.0 * (sorted.len() - 1) as f64;
    let (low, high) = (rank.floor() as usize, rank.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64)
}

// 0.5 s windows moved toward -20 dB, at most +20 dB, windows under -55 dB left alone.
// Same rule as the evaluation adapter's `window_gain`; it kept the most meeting
// content among the measured gain rules.
pub(super) fn window_gain(x: &[f32]) -> Vec<f32> {
    let target = 10_f64.powf(-20.0 / 20.0);
    let cap = 10_f64.powf(20.0 / 20.0);
    let floor = 10_f64.powf(-55.0 / 20.0);
    let mut out = Vec::with_capacity(x.len());
    for part in x.chunks(RATE / 2) {
        let rms = (part
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            / part.len() as f64)
            .sqrt()
            + 1e-9;
        let gain = if rms > floor {
            (target / rms).min(cap)
        } else {
            1.0
        };
        out.extend(
            part.iter()
                .map(|v| (f64::from(*v) * gain).clamp(-1.0, 1.0) as f32),
        );
    }
    out
}

#[derive(Deserialize)]
struct Native {
    #[serde(default)]
    transcription: Vec<Segment>,
}
#[derive(Deserialize)]
struct Segment {
    text: String,
    #[serde(default)]
    words: Vec<Unit>,
}
#[derive(Deserialize)]
struct Unit {
    text: String,
    t0: i64,
    t1: i64,
}

// Restores the transcript's own words; each aligner unit must match the next text
// characters exactly, and only non-alphanumeric text characters may lack a unit.
pub(super) fn restore_words(raw: &[u8], start: usize, end: usize) -> Result<Vec<Value>, String> {
    let native: Native = serde_json::from_slice(raw).map_err(|_| failure())?;
    let mut words = Vec::new();
    for segment in native.transcription {
        let tokens: Vec<(usize, usize)> = {
            let mut spans = Vec::new();
            let mut offset = 0;
            for piece in segment.text.split_whitespace() {
                let at = segment.text[offset..].find(piece).ok_or_else(failure)? + offset;
                spans.push((at, at + piece.len()));
                offset = at + piece.len();
            }
            spans
        };
        let text: Vec<(usize, char)> = segment.text.char_indices().collect();
        let owner = |byte: usize| tokens.iter().position(|(a, b)| *a <= byte && byte < *b);
        let mut cursor = 0;
        let mut times: Vec<Option<(i64, i64)>> = vec![None; tokens.len()];
        for unit in &segment.words {
            let chars: Vec<char> = unit.text.chars().filter(|c| !c.is_whitespace()).collect();
            if chars.is_empty() {
                continue;
            }
            if unit.t0 < 0 || unit.t1 < unit.t0 {
                return Err(failure());
            }
            let mut word = None;
            for wanted in chars {
                while cursor < text.len() && text[cursor].1 != wanted {
                    if text[cursor].1.is_alphanumeric() {
                        return Err(failure());
                    }
                    cursor += 1;
                }
                if cursor == text.len() {
                    return Err(failure());
                }
                let index = owner(text[cursor].0).ok_or_else(failure)?;
                if word.is_some_and(|w| w != index) {
                    return Err(failure());
                }
                word = Some(index);
                cursor += 1;
            }
            let index = word.ok_or_else(failure)?;
            times[index] = Some(match times[index] {
                Some((first, _)) => (first, unit.t1),
                None => (unit.t0, unit.t1),
            });
        }
        if text[cursor..].iter().any(|(_, c)| c.is_alphanumeric()) {
            return Err(failure());
        }
        let mut pending = String::new();
        for (index, (a, b)) in tokens.iter().enumerate() {
            let piece = &segment.text[*a..*b];
            let Some((t0, t1)) = times[index] else {
                // Punctuation-only text keeps its place on the neighbouring word.
                if piece.chars().any(char::is_alphanumeric) {
                    return Err(failure());
                }
                match words.last_mut() {
                    Some(Value::Object(last)) => {
                        let text = last["word"].as_str().unwrap_or_default().to_owned() + piece;
                        last.insert("word".into(), text.into());
                    }
                    _ => pending.push_str(piece),
                }
                continue;
            };
            let frame = |t: i64| -> Result<usize, String> {
                let local = usize::try_from(t).map_err(|_| failure())? * (RATE / 100);
                Ok((start + local.saturating_sub(PAD)).clamp(start, end))
            };
            let (s, e) = (frame(t0)?, frame(t1)?);
            words.push(json!({"word": std::mem::take(&mut pending) + piece,
                "start": s as f64 / RATE as f64, "end": e as f64 / RATE as f64}));
        }
        if !pending.is_empty() {
            return Err(failure());
        }
    }
    Ok(words)
}

fn classify_metal(log: &str, chunks: usize) -> Result<(), String> {
    let failed = log.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        line.starts_with("error")
            || ((line.starts_with("ggml_")
                || line.starts_with("qwen3")
                || line.starts_with("crispasr"))
                && (lower.contains("failed")
                    || lower.contains("error:")
                    || lower.contains("out of memory")
                    || lower.contains("cannot")))
    });
    let device = log
        .lines()
        .any(|line| line.starts_with("ggml_metal_init: found device: Apple "));
    let graph = log
        .lines()
        .any(|line| line.trim() == "ggml_metal_init: use graph optimize = true");
    let buffer = log
        .lines()
        .any(|line| line.starts_with("ggml_metal_log_allocated_size: allocated buffer, size ="));
    let loaded = log
        .lines()
        .filter(|line| line.starts_with("qwen3_asr: loaded "))
        .count();
    let audio = log
        .lines()
        .filter(|line| line.starts_with("crispasr: audio: "))
        .count();
    if failed || !device || !graph || !buffer || loaded != 1 || audio != chunks {
        return Err(
            "Qwen의 Metal 실행이나 전체 구간 처리를 확인하지 못했습니다. 원본은 보관했습니다."
                .into(),
        );
    }
    Ok(())
}

fn write_chunk(path: &Path, samples: &[f32]) -> Result<String, String> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut pcm = Sha256::new();
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer.write_sample(value).map_err(|e| e.to_string())?;
        pcm.update(value.to_le_bytes());
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(format!("{:x}", pcm.finalize()))
}

pub(super) fn recognize(
    resources: &Path,
    audio: &Path,
    work: &Path,
    cancel: &AtomicBool,
    model_dir: &Path,
) -> Result<Vec<u8>, Failure> {
    let model: Value = serde_json::from_str(MANIFEST).map_err(|_| failure())?;
    let runtime: Value = serde_json::from_str(RUNTIME).map_err(|_| failure())?;
    let files: Vec<PathBuf> = (0..2)
        .map(|i| {
            model["files"][i]["name"]
                .as_str()
                .map(|name| model_dir.join(name))
                .ok_or_else(failure)
        })
        .collect::<Result<_, _>>()?;
    // The transcribe job verified these hashes just before; only the post-run check repeats it.
    let hashes: Vec<String> = (0..2)
        .map(|i| {
            model["files"][i]["sha256"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(failure)
        })
        .collect::<Result<_, _>>()?;
    let cli = executable(resources);
    let cli_hash = file_hash(&cli)?;
    let mut wav = hound::WavReader::new(plain_file(audio)?).map_err(|_| failure())?;
    let spec = wav.spec();
    if spec.channels != 1
        || spec.sample_rate != RATE as u32
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(failure().into());
    }
    let mut input = Sha256::new();
    let mut pcm = Vec::with_capacity(wav.duration() as usize);
    for sample in wav.samples::<i16>() {
        let sample = sample.map_err(|_| failure())?;
        input.update(sample.to_le_bytes());
        pcm.push(f32::from(sample) / 32768.0);
    }
    if pcm.is_empty() {
        return Err(failure().into());
    }
    let input_sha = format!("{:x}", input.finalize());
    let bounds = pause_split(&pcm);
    let directory = work.join("qwen");
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let inputs = tempfile::Builder::new()
        .prefix("qwen-inputs-")
        .tempdir_in(work)
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(&cli);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4).max(1));
    command
        .args(["--backend", "qwen3-1.7b", "-m"])
        .arg(&files[0])
        .arg("-am")
        .arg(&files[1])
        .args(["-l", "ko", "-ojf", "-t"])
        .arg(threads.to_string());
    let mut chunk_hashes = Vec::new();
    for (index, (start, end)) in bounds.iter().enumerate() {
        cancelled(cancel)?;
        let mut padded = vec![0.0_f32; PAD];
        padded.extend_from_slice(&pcm[*start..*end]);
        padded.extend(std::iter::repeat_n(0.0, PAD));
        let path = inputs.path().join(format!("chunk-{index:04}.wav"));
        chunk_hashes.push(write_chunk(&path, &window_gain(&padded))?);
        command
            .arg("-of")
            .arg(directory.join(format!("chunk-{index:04}")));
    }
    for index in 0..bounds.len() {
        command.arg(inputs.path().join(format!("chunk-{index:04}.wav")));
    }
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.starts_with("GGML_") || text.starts_with("CRISPASR_") {
            command.env_remove(&name);
        }
    }
    let log_path = directory.join("crispasr.stderr.log");
    command
        .env("GGML_METAL_PIPELINE_CACHE_DISABLE", "1")
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("crispasr.stdout.log"))
                .map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&log_path)
                .map_err(|e| e.to_string())?,
        ));
    let mut child = crate::process_tree::spawn(&mut command)?;
    let began = Instant::now();
    let limit = Duration::from_secs(600 + 3 * (pcm.len() / RATE) as u64);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Qwen 처리를 취소했습니다.".into());
        }
        if began.elapsed() > limit {
            return Err("Qwen 처리 시간이 너무 오래 걸렸습니다. 원본은 보관했습니다.".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Qwen 구간 처리를 완료하지 못했습니다. 원본은 보관했습니다.".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(child);
    let log = read(&log_path, 64 * 1024 * 1024)?;
    classify_metal(&String::from_utf8_lossy(&log), bounds.len())?;
    let mut words = Vec::new();
    let mut chunks = Vec::new();
    for (index, (start, end)) in bounds.iter().enumerate() {
        let raw = read(
            &directory.join(format!("chunk-{index:04}.json")),
            MAX_NATIVE_JSON,
        )?;
        let restored = restore_words(&raw, *start, *end)?;
        chunks.push(json!({"index":index,"startFrame":start,"endFrame":end,
            "inputPcmSha256":chunk_hashes[index],"nativeResultSha256":format!("{:x}",Sha256::digest(&raw)),
            "nativeExitCode":0,"actualBackend":"metal","backendEvidence":"metal-backend-initialized",
            "wordStart":words.len(),"wordCount":restored.len()}));
        words.extend(restored);
    }
    if file_hash(&cli)? != cli_hash
        || files
            .iter()
            .zip(&hashes)
            .any(|(path, hash)| file_hash(path).ok().as_ref() != Some(hash))
    {
        return Err("Qwen 실행기나 모델이 처리 중 변경됐습니다.".into());
    }
    let frames = pcm.len();
    Ok(serde_json::to_vec(&json!({"schemaVersion":1,"model":model["repository"],"modelRevision":model["revision"],
        "modelSha256":hashes[0],"alignerSha256":hashes[1],"runtime":runtime["identity"],"target":"macos-arm64",
        "processingPolicy":"qwen3-pause-chunks-v1","completionEvidence":"crispasr-exit0-v1",
        "timestampPolicy":"forced-aligner-text-units-v1","gainPolicy":"window-gain-v1","padFrames":PAD,
        "completed":true,"inputAudioFrames":frames,"audioSampleRate":RATE,
        "audioDurationSeconds":frames as f64/RATE as f64,"inputPcmSha256":input_sha,
        "chunks":chunks,"words":words}))
    .map_err(|_| failure())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_split_covers_the_input_and_cuts_inside_the_longest_quiet_run() {
        let mut pcm = vec![0.3_f32; 40 * RATE];
        for sample in &mut pcm[20 * RATE..21 * RATE] {
            *sample = 0.0;
        }
        let chunks = pause_split(&pcm);
        assert_eq!(chunks.first().unwrap().0, 0);
        assert_eq!(chunks.last().unwrap().1, pcm.len());
        assert!(chunks.windows(2).all(|w| w[0].1 == w[1].0));
        let cut = chunks[0].1;
        assert!((20 * RATE..21 * RATE).contains(&cut), "{cut}");
        assert_eq!(pause_split(&vec![0.1; 10 * RATE]), vec![(0, 10 * RATE)]);
    }

    #[test]
    fn window_gain_raises_quiet_windows_within_its_cap_and_leaves_silence() {
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let quiet = vec![0.01_f32; RATE / 2];
        assert!((rms(&window_gain(&quiet)) - 0.1).abs() < 1e-4);
        let faint = vec![0.002_f32; RATE / 2];
        assert!((rms(&window_gain(&faint)) - 0.02).abs() < 1e-5);
        let below_floor = vec![0.001_f32; RATE / 2];
        assert_eq!(window_gain(&below_floor), below_floor);
        assert!(window_gain(&vec![0.0; RATE]).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn aligner_units_restore_text_words_and_reject_mismatches() {
        let raw = r#"{"transcription":[{"text":"예시 30분, 네.","words":[
            {"text":"예","t0":110,"t1":120},{"text":"시","t0":120,"t1":140},
            {"text":" 30","t0":150,"t1":160},{"text":"분","t0":160,"t1":170},
            {"text":" 네","t0":200,"t1":200}]}]}"#;
        let words = restore_words(raw.as_bytes(), 16000, 16000 + 30 * RATE).unwrap();
        assert_eq!(
            words
                .iter()
                .map(|w| w["word"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["예시", "30분,", "네."]
        );
        assert_eq!(words[0]["start"], 1.1);
        assert_eq!(words[2]["start"], words[2]["end"]);
        let missing =
            r#"{"transcription":[{"text":"예시","words":[{"text":"예","t0":0,"t1":1}]}]}"#;
        assert!(restore_words(missing.as_bytes(), 0, RATE).is_err());
        let spanning =
            r#"{"transcription":[{"text":"예 시","words":[{"text":"예시","t0":0,"t1":1}]}]}"#;
        assert!(restore_words(spanning.as_bytes(), 0, RATE).is_err());
        let extra = r#"{"transcription":[{"text":"예","words":[{"text":"예","t0":0,"t1":1},{"text":"시","t0":1,"t1":2}]}]}"#;
        assert!(restore_words(extra.as_bytes(), 0, RATE).is_err());
        let early = r#"{"transcription":[{"text":"예","words":[{"text":"예","t0":10,"t1":20}]}]}"#;
        let clamped = restore_words(early.as_bytes(), 0, RATE).unwrap();
        assert_eq!(clamped[0]["start"], 0.0);
    }

    #[test]
    fn metal_evidence_requires_one_model_load_and_every_chunk() {
        let valid = "ggml_metal_init: found device: Apple M4\nggml_metal_init: use graph optimize = true\nggml_metal_log_allocated_size: allocated buffer, size =  2670.41 MiB\nqwen3_asr: loaded model.gguf\ncrispasr: audio: 1 samples\ncrispasr: audio: 2 samples\n";
        assert!(classify_metal(valid, 2).is_ok());
        assert!(classify_metal(valid, 3).is_err());
        assert!(classify_metal(
            &(valid.to_owned() + "ggml_metal_buffer: failed to allocate\n"),
            2
        )
        .is_err());
        assert!(classify_metal("crispasr: audio: 1 samples\n", 1).is_err());
    }
}
