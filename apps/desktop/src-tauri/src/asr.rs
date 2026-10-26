use crate::processing::{cancelled, plain_file, Model, Processing};
use crate::summary_failure::{Failure, FailureKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const MANIFEST: &str = include_str!("../../../../config/models/nemotron-asr.json");
pub const LABEL: &str = "Nemotron35-ASR";
const RUNTIME: &str = "nemo-speech-0.2.0@6a3ca369370782acee0dd155e34394ba60b920c4";
const POLICY: &str = "whole-recording-stateful-v1";
const COMPLETION: &str = "native-stream-finish-exit0-v1";
const RESULT_LIMIT: u64 = 16 * 1024 * 1024;
const SAMPLE_RATE: u32 = 16000;

pub fn executable(process: &Processing, gpu: bool) -> PathBuf {
    process
        .resources
        .join("native/diarization")
        .join(if gpu || cfg!(target_os = "macos") {
            "gpu"
        } else {
            "cpu"
        })
        .join("bin")
        .join(if cfg!(windows) {
            "nemo-speech.exe"
        } else {
            "nemo-speech"
        })
}

pub fn supported(process: &Processing) -> bool {
    (cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "windows", target_arch = "x86_64")))
        && (executable(process, true).is_file() || executable(process, false).is_file())
}

fn failure(kind: FailureKind) -> Failure {
    Failure::new(
        kind,
        match kind {
            FailureKind::Cancelled => "음성 인식을 취소했습니다.",
            FailureKind::Allocation => "음성 인식에 필요한 메모리를 확보하지 못했습니다.",
            FailureKind::BackendUnavailable => "음성 인식 처리 장치를 사용할 수 없습니다.",
            FailureKind::ModelNotReady => "음성 인식 모델을 다시 준비해 주세요.",
            FailureKind::Input => "음성 인식에 사용할 녹음 파일을 읽지 못했습니다.",
            FailureKind::Output | FailureKind::Protocol => "음성 인식 결과를 읽지 못했습니다.",
            FailureKind::Storage => "음성 인식 결과를 저장하지 못했습니다.",
            FailureKind::Cleanup => {
                "음성 인식 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요."
            }
            FailureKind::SourceChanged => {
                "처리 중 녹음 파일이 변경됐습니다. 현재 기록은 유지했습니다."
            }
            _ => "음성 인식을 완료하지 못했습니다.",
        },
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> {
    cancelled(cancel).map_err(|_| failure(FailureKind::Cancelled))
}

fn digest(path: &Path) -> Result<String, Failure> {
    let mut input = plain_file(path).map_err(|_| failure(FailureKind::Input))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let count = input
            .read(&mut block)
            .map_err(|_| failure(FailureKind::Input))?;
        if count == 0 {
            break;
        }
        hash.update(&block[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn audio_frames(audio: &Path, cancel: &AtomicBool) -> Result<u64, Failure> {
    let input = plain_file(audio).map_err(|_| failure(FailureKind::Input))?;
    let mut reader =
        hound::WavReader::new(BufReader::new(input)).map_err(|_| failure(FailureKind::Input))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != SAMPLE_RATE {
        return Err(failure(FailureKind::Input));
    }
    let expected = u64::from(reader.duration());
    if expected == 0 {
        return Err(failure(FailureKind::Input));
    }
    let mut frames = 0u64;
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            for sample in reader.samples::<f32>() {
                if !sample.map_err(|_| failure(FailureKind::Input))?.is_finite() {
                    return Err(failure(FailureKind::Input));
                }
                frames += 1;
                if frames % u64::from(SAMPLE_RATE) == 0 {
                    check_cancel(cancel)?;
                }
            }
        }
        (hound::SampleFormat::Int, 16) => {
            for sample in reader.samples::<i16>() {
                sample.map_err(|_| failure(FailureKind::Input))?;
                frames += 1;
                if frames % u64::from(SAMPLE_RATE) == 0 {
                    check_cancel(cancel)?;
                }
            }
        }
        _ => return Err(failure(FailureKind::Input)),
    }
    if frames != expected {
        return Err(failure(FailureKind::Input));
    }
    check_cancel(cancel)?;
    Ok(frames)
}

fn backend(gpu: bool) -> &'static str {
    if gpu {
        if cfg!(target_os = "macos") {
            "metal"
        } else {
            "vulkan"
        }
    } else {
        "cpu"
    }
}

fn arguments(audio: &Path, model: &Path, output: &Path, gpu: bool) -> Vec<std::ffi::OsString> {
    [
        Path::new("--json"),
        Path::new("--verbose"),
        Path::new("transcribe"),
        audio,
        Path::new("--model"),
        model,
        Path::new("--backend"),
        Path::new(backend(gpu)),
        Path::new("--language"),
        Path::new("ko-KR"),
        Path::new("--format"),
        Path::new("json"),
        Path::new("--concurrency"),
        Path::new("1"),
        Path::new("--no-batching"),
        Path::new("--no-warmup"),
        Path::new("--endpointing=false"),
        Path::new("--asr.streaming.rnnt_right_context"),
        Path::new("13"),
        Path::new("--stream"),
        Path::new("--output"),
        output,
    ]
    .iter()
    .map(|value| value.as_os_str().to_os_string())
    .collect()
}

fn record(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| failure(FailureKind::Storage))?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|_| failure(FailureKind::Storage))?;
    file.write_all(b"\n")
        .map_err(|_| failure(FailureKind::Storage))?;
    file.sync_all().map_err(|_| failure(FailureKind::Storage))
}

fn read_result(path: &Path) -> Result<(Vec<u8>, Value), Failure> {
    let file = plain_file(path).map_err(|_| failure(FailureKind::Output))?;
    if file
        .metadata()
        .map_err(|_| failure(FailureKind::Output))?
        .len()
        > RESULT_LIMIT
    {
        return Err(failure(FailureKind::Output));
    }
    let mut raw = Vec::new();
    file.take(RESULT_LIMIT + 1)
        .read_to_end(&mut raw)
        .map_err(|_| failure(FailureKind::Output))?;
    if raw.len() as u64 > RESULT_LIMIT {
        return Err(failure(FailureKind::Output));
    }
    let result = serde_json::from_slice(&raw).map_err(|_| failure(FailureKind::Output))?;
    Ok((raw, result))
}

fn envelope(
    spec: &Model,
    native: &Value,
    frames: u64,
    input_sha: &str,
    native_sha: &str,
    gpu: bool,
) -> Result<Value, Failure> {
    let native_duration = native
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| failure(FailureKind::Output))?;
    let words = native
        .get("words")
        .and_then(Value::as_array)
        .ok_or_else(|| failure(FailureKind::Output))?;
    let text = native
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| failure(FailureKind::Output))?;
    let recognized_words: Option<Vec<_>> = words
        .iter()
        .map(|word| word.get("word").and_then(Value::as_str))
        .collect();
    if recognized_words
        .ok_or_else(|| failure(FailureKind::Output))?
        .join(" ")
        != text
    {
        return Err(failure(FailureKind::Output));
    }
    Ok(json!({
        "schemaVersion": 1, "model": spec.repository, "modelRevision": spec.revision,
        "runtime": RUNTIME, "processingPolicy": POLICY,
        "inputAudioFrames": frames, "audioSampleRate": SAMPLE_RATE,
        "audioDurationSeconds": frames as f64 / f64::from(SAMPLE_RATE),
        "inputAudioSha256": input_sha, "completed": true,
        "completionEvidence": COMPLETION, "nativeDurationSeconds": native_duration,
        "nativeResultSha256": native_sha, "backend": backend(gpu), "text": text, "words": words,
    }))
}

pub fn recognize(
    process: &Processing,
    audio: &Path,
    path: &Path,
    cancel: &AtomicBool,
    model_directory: &Path,
) -> Result<Vec<u8>, Failure> {
    check_cancel(cancel)?;
    let spec: Model = serde_json::from_str(MANIFEST).expect("bundled ASR manifest");
    let item = spec
        .files
        .first()
        .ok_or_else(|| failure(FailureKind::ModelNotReady))?;
    let model = model_directory.join(&item.name);
    plain_file(&model).map_err(|_| failure(FailureKind::ModelNotReady))?;
    let input_sha = digest(audio)?;
    let frames = audio_frames(audio, cancel)?;
    if digest(audio)? != input_sha {
        return Err(failure(FailureKind::SourceChanged));
    }
    let mut gpu = executable(process, true).is_file();
    record(
        &path.join("recognition-request.json"),
        &json!({
            "runtime": RUNTIME, "model": spec.repository, "modelRevision": spec.revision,
            "processingPolicy": POLICY, "inputAudioFrames": frames, "audioSampleRate": SAMPLE_RATE,
            "inputAudioSha256": input_sha, "language": "ko-KR", "stream": true,
            "endpointing": false, "concurrency": 1, "batching": false, "warmup": false,
            "rnntRightContext": 13, "requestedBackend": backend(gpu),
        }),
    )?;
    let mut output = path.join(if gpu {
        "native.json"
    } else {
        "native-cpu.json"
    });
    let run = |gpu: bool, output: &Path| {
        crate::diarization::run(
            process,
            &executable(process, gpu),
            &arguments(audio, &model, output, gpu),
            &path.join(if gpu { "asr-gpu.log" } else { "asr-cpu.log" }),
            cancel,
            gpu,
            Some(audio),
            "transcribe",
        )
        .map_err(|error| failure(error.kind))
    };
    if let Err(error) = run(gpu, &output) {
        if !gpu || error.kind != FailureKind::BackendUnavailable {
            return Err(error);
        }
        check_cancel(cancel)?;
        gpu = false;
        output = path.join("native-cpu.json");
        run(gpu, &output)?;
    }
    check_cancel(cancel)?;
    if digest(audio)? != input_sha {
        return Err(failure(FailureKind::SourceChanged));
    }
    let (raw, native) = read_result(&output)?;
    let native_sha = format!("{:x}", Sha256::digest(&raw));
    let result = envelope(&spec, &native, frames, &input_sha, &native_sha, gpu)?;
    record(
        &path.join("recognition-completion.json"),
        &json!({
            "runtime": RUNTIME, "processingPolicy": POLICY, "completionEvidence": COMPLETION,
            "exitCode": 0, "inputAudioSha256": input_sha, "inputAudioFrames": frames,
            "nativeResultSha256": native_sha, "backend": backend(gpu),
            "nativeOutput": output.file_name().and_then(|value| value.to_str()),
        }),
    )?;
    serde_json::to_vec(&result).map_err(|_| failure(FailureKind::Output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    #[test]
    #[ignore = "requires explicitly supplied local audio, shared models and packaged runtime"]
    fn local_full_recording_acceptance() {
        let configured = |name: &str| PathBuf::from(std::env::var(name).expect(name));
        let audio = configured("NERO_ACCEPTANCE_AUDIO");
        let model_dir = configured("NERO_ACCEPTANCE_ASR_MODEL_DIR");
        let speaker_dir = configured("NERO_ACCEPTANCE_SPEAKER_MODEL_DIR");
        let resources = configured("NERO_ACCEPTANCE_RESOURCES");
        let work = configured("NERO_ACCEPTANCE_WORK");
        std::fs::create_dir(&work).unwrap();
        let library = work.join("library");
        std::fs::create_dir(&library).unwrap();
        let process = Processing::new(library.clone(), resources);
        let core = grove_core::CoreHandle::open(&library).unwrap();
        let meeting = core.import_audio("Acceptance", &audio, &audio).unwrap();
        let job = work.join("job");
        std::fs::create_dir(&job).unwrap();
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let raw = recognize(&process, &audio, &job, &cancel, &model_dir).unwrap();
        let recognition_seconds = started.elapsed().as_secs_f64();
        std::fs::write(job.join("recognition.json"), &raw).unwrap();
        let source = core
            .commit_nemotron_transcript(&meeting.id, None, raw)
            .unwrap();
        let speaker_started = std::time::Instant::now();
        let draft = crate::diarization::analyze_with_model(
            &process,
            source,
            &audio,
            &job,
            &cancel,
            &speaker_dir,
        )
        .unwrap();
        let final_doc = core.apply_diarization(&meeting.id, draft).unwrap();
        assert!(!final_doc.segments.is_empty());
        let reread = core.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!(reread.version, final_doc.version);
        let report = json!({
            "recognitionSeconds": recognition_seconds,
            "speakerSeconds": speaker_started.elapsed().as_secs_f64(),
            "segments": final_doc.segments.len(), "speakers": final_doc.speakers.len(),
            "unassigned": final_doc.segments.iter().filter(|s| s.speaker_id.is_none()).count(),
            "source": final_doc.source, "sourceModel": final_doc.model,
            "lastEnd": final_doc.segments.last().unwrap().end,
            "sourceAudioSha256": digest(&audio).unwrap(),
            "scope": "ASR plus global speaker analysis, isolated store, no human gold or installed GUI qualification"
        });
        record(&work.join("report.json"), &report).unwrap();
        println!("{}", report);
    }

    #[test]
    #[ignore = "resumes only speaker analysis against an explicitly supplied isolated recognition store"]
    fn local_speaker_acceptance_from_saved_recognition() {
        let configured = |name: &str| PathBuf::from(std::env::var(name).expect(name));
        let audio = configured("NERO_ACCEPTANCE_AUDIO");
        let speaker_dir = configured("NERO_ACCEPTANCE_SPEAKER_MODEL_DIR");
        let work = configured("NERO_ACCEPTANCE_WORK");
        let library = work.join("library");
        let process = Processing::new(library.clone(), configured("NERO_ACCEPTANCE_RESOURCES"));
        let core = grove_core::CoreHandle::open(&library).unwrap();
        let meeting = core.list_meetings().unwrap().pop().unwrap();
        let source = core.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!(source.model, "nvidia/nemotron-3.5-asr-streaming-0.6b");
        let started = std::time::Instant::now();
        let draft = crate::diarization::analyze_with_model(
            &process,
            source,
            &audio,
            &work.join("job"),
            &AtomicBool::new(false),
            &speaker_dir,
        )
        .unwrap();
        let doc = core.apply_diarization(&meeting.id, draft).unwrap();
        record(
            &work.join("speaker-resume-report.json"),
            &json!({
                "speakerSeconds": started.elapsed().as_secs_f64(),
                "segments": doc.segments.len(), "speakers": doc.speakers.len(),
                "unassigned": doc.segments.iter().filter(|s| s.speaker_id.is_none()).count(),
                "lastEnd": doc.segments.last().unwrap().end,
                "sourceAudioSha256": digest(&audio).unwrap(),
                "recognitionReexecuted": false
            }),
        )
        .unwrap();
    }

    #[test]
    fn native_arguments_preserve_full_file_and_one_stateful_stream() {
        let arguments = arguments(
            Path::new("complete.wav"),
            Path::new("model.gguf"),
            Path::new("native.json"),
            false,
        );
        let values: Vec<_> = arguments
            .iter()
            .map(|value| value.to_string_lossy())
            .collect();
        assert_eq!(values[2], "transcribe");
        assert_eq!(values[3], "complete.wav");
        for flag in [
            "--stream",
            "--endpointing=false",
            "--no-batching",
            "--no-warmup",
        ] {
            assert!(values.iter().any(|value| value == flag));
        }
        assert!(values
            .windows(2)
            .any(|pair| pair == ["--asr.streaming.rnnt_right_context", "13"]));
        assert!(!values
            .iter()
            .any(|value| value.contains("diarize") || value.contains("speech-context")));
    }

    #[test]
    fn completion_evidence_does_not_claim_measured_processing_frames_or_eos() {
        let spec: Model = serde_json::from_str(MANIFEST).unwrap();
        let native = json!({"duration": 1.23, "text": "예시", "words": [{"word": "예시", "start": 0.0, "end": 1.0}]});
        let result = envelope(&spec, &native, 19753, "input-hash", "raw-hash", false).unwrap();
        assert_eq!(result["completionEvidence"], COMPLETION);
        assert_eq!(result["audioDurationSeconds"], 19753.0 / 16000.0);
        assert_eq!(result["nativeDurationSeconds"], 1.23);
        assert!(result.get("processedAudioFrames").is_none());
        assert!(result.get("eosReached").is_none());
        assert!(result.get("speakers").is_none());
        assert_eq!(result["words"], native["words"]);
    }

    #[test]
    fn wav_validation_counts_actual_frames_and_rejects_empty_input() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("audio.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        for _ in 0..321 {
            writer.write_sample(0.0_f32).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(audio_frames(&path, &AtomicBool::new(false)).unwrap(), 321);
        WavWriter::create(&path, spec).unwrap().finalize().unwrap();
        assert_eq!(
            audio_frames(&path, &AtomicBool::new(false))
                .unwrap_err()
                .kind,
            FailureKind::Input
        );
    }

    #[test]
    fn missing_word_timing_cannot_silently_drop_recognized_text() {
        let spec: Model = serde_json::from_str(MANIFEST).unwrap();
        let native = json!({"duration":1.0,"text":"앞 문장 뒤 문장", "words":[{"word":"앞 문장","start":0.0,"end":0.5}]});
        assert_eq!(
            envelope(&spec, &native, 16000, "input", "native", false)
                .unwrap_err()
                .kind,
            FailureKind::Output
        );
    }
}
