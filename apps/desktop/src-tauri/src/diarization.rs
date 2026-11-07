//! Optional whole-recording speaker analysis. It never treats ASR-local tags as identities.
use crate::failure::{core_failure_kind, Failure, FailureKind};
use crate::native_speech::{self, executable, library, Backend};
use crate::processing::{
    cancelled, directory, plain_file, verify_model_files, JobSpec, Model, ModelSpec, ModelStage,
    Processing, Progress,
};
use crate::speech_audio::AnalysisAudio;
use nero_core::{
    CoreHandle, SpeakerAnalysisDraft, SpeakerPosterior, Transcript, TranscriptVersion,
};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};

pub const MANIFEST: &str = include_str!("../../models/diarization.json");
use nero_core::speech_runtime::current as runtime;
const RESULT_LIMIT: u64 = 16 * 1024 * 1024;
const POSTERIOR_LIMIT: u64 = 128 * 1024 * 1024;

pub(super) const SPEAKER_MODEL: ModelSpec = ModelSpec {
    manifest: MANIFEST,
    label: "Nemotron3",
    stage: ModelStage::Speakers,
};

pub(super) fn model() -> Model {
    SPEAKER_MODEL.model()
}

pub fn availability(process: &Processing) -> (bool, bool, u64) {
    let supported = (cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "windows", target_arch = "x86_64")))
        && ((executable(process, true).is_file() && library(process, true).is_file())
            || (executable(process, false).is_file() && library(process, false).is_file()));
    (
        supported,
        process.model_ready(SPEAKER_MODEL),
        model().bytes(),
    )
}

fn failure(kind: FailureKind) -> Failure {
    Failure::new(
        kind,
        match kind {
            FailureKind::Cancelled => "화자 구분을 취소했습니다.",
            FailureKind::Allocation => "화자 구분에 필요한 메모리를 확보하지 못했습니다.",
            FailureKind::BackendUnavailable => "화자 구분 처리 장치를 사용할 수 없습니다.",
            FailureKind::ModelNotReady => "화자 구분 모델을 다시 준비해 주세요.",
            FailureKind::Input => "화자 구분에 사용할 녹음 파일을 읽지 못했습니다.",
            FailureKind::Output | FailureKind::Protocol => "화자 구분 결과를 읽지 못했습니다.",
            FailureKind::Storage => "화자 구분 결과를 저장하지 못했습니다.",
            FailureKind::Cleanup => {
                "화자 구분 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요."
            }
            FailureKind::SourceChanged => {
                "처리 중 음성 기록이 변경됐습니다. 현재 기록은 유지했습니다."
            }
            _ => "화자 구분을 완료하지 못했습니다.",
        },
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> {
    cancelled(cancel).map_err(|_| failure(FailureKind::Cancelled))
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

fn worker_executable() -> Result<PathBuf, Failure> {
    // Acceptance tests run the built product executable, never the test harness.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("NERO_ACCEPTANCE_SPEAKER_WORKER") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().map_err(|_| failure(FailureKind::EngineUnavailable))
}

fn configuration(gpu: bool) -> Value {
    json!({
        "scope": "recording", "fullAttention": false,
        "backend": Backend::for_gpu(gpu),
        "geometry": {"spkcache":264,"fifo":40,"chunk":340,"updatePeriod":300,
            "leftContext":0,"rightContext":40},
        "postprocessingSource": "nvidia-v3-probability-v1",
        "assignment": "nemo-word-160ms-argmax-v1", "batching": false
    })
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, Failure> {
    let file = plain_file(path).map_err(|_| failure(FailureKind::Output))?;
    if file
        .metadata()
        .map_err(|_| failure(FailureKind::Storage))?
        .len()
        > limit
    {
        return Err(failure(FailureKind::Output));
    }
    let mut data = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| failure(FailureKind::Output))?;
    if data.len() as u64 > limit {
        return Err(failure(FailureKind::Output));
    }
    Ok(data)
}

fn posterior(
    path: &Path,
    audio: &Path,
    weights: &Path,
    native: &Path,
    config: &Value,
    cancel: &AtomicBool,
) -> Result<SpeakerPosterior, Failure> {
    check_cancel(cancel)?;
    let receipt: Value =
        serde_json::from_slice(&read_limited(&path.join("receipt.json"), RESULT_LIMIT)?)
            .map_err(|_| failure(FailureKind::Output))?;
    let frames = crate::speech_audio::audio_frames(audio, cancel)?;
    let native_frames = frames / 160 + 1;
    let expected_bytes = native_frames
        .checked_mul(8 * 4)
        .ok_or_else(|| failure(FailureKind::Output))?;
    if receipt["nativeStreamFinished"] != true
        || receipt["sourceFramesFed"].as_u64() != Some(frames)
        || receipt["backend"] != config["backend"]
        || receipt["nativeFrames"].as_u64() != Some(native_frames)
        || receipt["expectedNativeFrames"].as_u64() != Some(native_frames)
        || receipt["speakerSlots"] != 8
        || receipt["secondsPerFrame"].as_f64() != Some(f64::from(0.01_f32))
        || receipt["probabilityBytes"].as_u64() != Some(expected_bytes)
        || receipt["runtimeVersionString"] != nero_core::speech_runtime::asr_version()
        || receipt["runtimePin"] != nero_core::speech_runtime::pin()
        || receipt["inputAndWeightsAndLibraryUnchanged"] != true
        || receipt["postprocessingPolicy"] != "nemotron-v3-contiguous-threshold-0.5-v1"
        || receipt["threshold"] != 0.5
        || receipt["thresholdComparison"] != ">"
        || receipt["geometry"]
            != json!({"timebaseSeconds":0.08,"spkcache":264,
            "fifo":40,"chunk":340,"updatePeriod":300,"leftContext":0,"rightContext":40})
        || receipt["sourceAudioSha256"].as_str()
            != Some(crate::speech_audio::digest(audio)?.as_str())
        || receipt["modelSha256"].as_str() != Some(crate::speech_audio::digest(weights)?.as_str())
        || receipt["librarySha256"].as_str() != Some(crate::speech_audio::digest(native)?.as_str())
        || receipt["probabilitySHA256"].as_str()
            != Some(crate::speech_audio::digest(&path.join("probabilities.f32le"))?.as_str())
    {
        return Err(failure(FailureKind::Output));
    }
    let probabilities = read_limited(&path.join("probabilities.f32le"), POSTERIOR_LIMIT)?;
    if probabilities.len() as u64 != expected_bytes {
        return Err(failure(FailureKind::Output));
    }
    check_cancel(cancel)?;
    Ok(SpeakerPosterior {
        frames: native_frames,
        speakers: 8,
        seconds_per_frame: f64::from(0.01_f32),
        probabilities,
    })
}

fn analyze(
    process: &Processing,
    source: Transcript,
    audio: &Path,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<SpeakerAnalysisDraft, Failure> {
    let directory = process
        .model_dir_of(SPEAKER_MODEL)
        .map_err(|_| failure(FailureKind::Storage))?;
    analyze_with_model(process, source, audio, path, cancel, &directory)
}

pub(super) fn completed_live(
    process: &Processing,
    source: Transcript,
    audio: &Path,
    path: &Path,
    backend: &Value,
    cancel: &AtomicBool,
) -> Result<SpeakerAnalysisDraft, Failure> {
    let spec = model();
    let item = &spec.files[0];
    let weights = process
        .model_dir_of(SPEAKER_MODEL)
        .map_err(|_| failure(FailureKind::Storage))?
        .join(&item.name);
    let gpu = backend != "cpu";
    let native = library(process, gpu);
    let config = configuration(gpu);
    if &config["backend"] != backend {
        return Err(failure(FailureKind::Protocol));
    }
    Ok(SpeakerAnalysisDraft {
        source,
        model: spec.repository,
        model_revision: spec.revision,
        model_sha256: item.sha256.clone(),
        runtime: runtime().into(),
        configuration: config.clone(),
        raw: read_limited(&path.join("diarization.json"), RESULT_LIMIT)?,
        posterior: Some(posterior(path, audio, &weights, &native, &config, cancel)?),
    })
}

pub(super) fn analyze_with_model(
    process: &Processing,
    source: Transcript,
    audio: &Path,
    path: &Path,
    cancel: &AtomicBool,
    directory: &Path,
) -> Result<SpeakerAnalysisDraft, Failure> {
    let spec = model();
    let item = &spec.files[0];
    verify_model_files(directory, &spec, cancel).map_err(|error| failure(error.kind))?;
    plain_file(audio).map_err(|_| failure(FailureKind::Input))?;
    let mut gpu = executable(process, true).is_file() && library(process, true).is_file();
    let mut attempt = 0u8;
    loop {
        check_cancel(cancel)?;
        let attempt_dir = directory_for_attempt(path, attempt)?;
        let output = attempt_dir.join("diarization.json");
        let config = configuration(gpu);
        let native = library(process, gpu);
        let weights = directory.join(&item.name);
        record(
            &attempt_dir.join("worker-request.json"),
            &json!({
                "schemaVersion":1, "audio":audio, "model":weights,
                "library":native, "output":attempt_dir,
                "backend":config["backend"], "modelSha256":item.sha256,
                "runtimePin":nero_core::speech_runtime::pin()
            }),
        )?;
        record(
            &attempt_dir.join("request.json"),
            &json!({
                "sourceVersion": source.version, "sourceSessionId": source.source_session_id,
                "model": spec.repository, "modelRevision": spec.revision,
                "modelSha256": item.sha256, "runtime": runtime(), "configuration": config
            }),
        )?;
        process.update(|progress| {
            progress.phase = Some("diarizing".into());
            progress.message = "화자 구분 중".into();
        });
        let result = native_speech::run(
            process,
            &worker_executable()?,
            &[
                std::ffi::OsString::from("--nero-speaker-worker"),
                attempt_dir.join("worker-request.json").into_os_string(),
            ],
            &attempt_dir.join("worker.log"),
            cancel,
            gpu,
            Some(audio),
            "diarize",
        )
        .map_err(failure);
        record(
            &attempt_dir.join("outcome.json"),
            &match &result {
                Ok(()) => {
                    json!({"attempt": attempt, "backend": config["backend"], "status": "completed"})
                }
                Err(error) => {
                    json!({"attempt": attempt, "backend": config["backend"], "status": "failed", "failureKind": error.kind})
                }
            },
        )?;
        match result {
            Ok(()) => {
                check_cancel(cancel)?;
                let raw = read_limited(&output, RESULT_LIMIT)?;
                let posterior = posterior(&attempt_dir, audio, &weights, &native, &config, cancel)?;
                return Ok(SpeakerAnalysisDraft {
                    source,
                    model: spec.repository,
                    model_revision: spec.revision,
                    model_sha256: item.sha256.clone(),
                    runtime: runtime().into(),
                    configuration: config,
                    raw,
                    posterior: Some(posterior),
                });
            }
            Err(error)
                if error.kind == FailureKind::BackendUnavailable
                    && gpu
                    && attempt == 0
                    && executable(process, false).is_file() =>
            {
                check_cancel(cancel)?;
                gpu = false;
                attempt += 1;
                process.update(|progress| progress.retry_count = attempt);
            }
            Err(error) => return Err(error),
        }
    }
}

fn directory_for_attempt(path: &Path, attempt: u8) -> Result<PathBuf, Failure> {
    directory(path, &format!("diarization-{attempt}")).map_err(|_| failure(FailureKind::Storage))
}

fn core_failure(error: &nero_core::CoreError) -> FailureKind {
    core_failure_kind(error).unwrap_or(FailureKind::Storage)
}

fn source_failure(error: nero_core::CoreError) -> Failure {
    failure(core_failure(&error))
}

fn check_source(
    core: &CoreHandle,
    meeting_id: &str,
    source: &Transcript,
    audio: &Path,
) -> Result<(), Failure> {
    let current = core.transcript(meeting_id).map_err(source_failure)?;
    if current.as_ref().map(|document| &document.version) != Some(&source.version)
        || core.audio_path(meeting_id).map_err(source_failure)? != audio
    {
        return Err(failure(FailureKind::SourceChanged));
    }
    let snapshot = core.snapshot().map_err(source_failure)?;
    if snapshot.active_session().is_some() {
        return Err(failure(FailureKind::RecordingActive));
    }
    Ok(())
}

pub(super) fn analyze_recording(
    process: &Processing,
    core: &CoreHandle,
    meeting_id: &str,
    source: Transcript,
    audio: &Path,
    prepared: Option<&AnalysisAudio>,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    check_cancel(cancel)?;
    check_source(core, meeting_id, &source, audio)?;
    if !availability(process).1 {
        return Err(failure(FailureKind::ModelNotReady));
    }
    process.update(|progress| {
        progress.phase = Some("diarizing".into());
        progress.message = "화자 구분 중".into();
    });
    let owned;
    let input = match prepared {
        Some(prepared) => prepared,
        None => {
            let work =
                directory(path, "speaker-input").map_err(|_| failure(FailureKind::Storage))?;
            owned = AnalysisAudio::prepare(process, audio, &work, cancel)?;
            &owned
        }
    };
    input.verify(audio, cancel)?;
    record(&path.join("speaker-input.json"), &input.evidence())?;
    let draft = analyze(process, source, input.path(), path, cancel)?;
    input.verify(audio, cancel)?;
    check_cancel(cancel)?;
    check_source(core, meeting_id, &draft.source, audio)?;
    let mut commit_failure = None;
    process
        .commit_transcript_result(cancel, || {
            core.apply_diarization(meeting_id, draft).map_err(|error| {
                commit_failure = Some(core_failure(&error));
                error.to_string()
            })
        })
        .map_err(|_| {
            failure(commit_failure.unwrap_or_else(|| {
                if cancel.load(std::sync::atomic::Ordering::Acquire) {
                    FailureKind::Cancelled
                } else {
                    FailureKind::Storage
                }
            }))
        })?;
    Ok(())
}

impl Processing {
    pub fn prepare_diarization(self: &Arc<Self>) -> Result<Progress, String> {
        self.prepare_named_model(SPEAKER_MODEL, "speaker-model")
    }

    pub fn diarize(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
        expected: TranscriptVersion,
    ) -> Result<Progress, String> {
        self.diarize_bound(core, meeting_id, expected, None, None)
    }
    pub(super) fn diarize_bound(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
        expected: TranscriptVersion,
        required_fingerprint: Option<String>,
        requested_id: Option<String>,
    ) -> Result<Progress, String> {
        let source = core
            .transcript(&meeting_id)
            .map_err(|error| error.to_string())?
            .ok_or("먼저 음성 기록을 만들어 주세요.")?;
        if source.version != expected {
            return Err(
                "음성 기록이 변경됐습니다. 현재 기록을 확인한 뒤 다시 시도해 주세요.".into(),
            );
        }
        let audio = core
            .audio_path(&meeting_id)
            .map_err(|error| error.to_string())?;
        if !availability(self).1 {
            return Err("먼저 화자 구분 모델을 준비해 주세요.".into());
        }
        self.start_job(
            JobSpec {
                kind: "diarize",
                meeting_id: Some(meeting_id.clone()),
                source: Some(core.clone()),
                required_fingerprint,
                requested_id,
                ..JobSpec::default()
            },
            move |this, cancel, path| {
                let result = analyze_recording(
                    &this,
                    &core,
                    &meeting_id,
                    source,
                    &audio,
                    None,
                    &path,
                    &cancel,
                );
                result.map_err(|error| this.report(error))?;
                Ok(Some(meeting_id))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn baseline(process: &Processing) -> (CoreHandle, String, Transcript, PathBuf) {
        let core = CoreHandle::open(&process.root).unwrap();
        let input = process.root.join("synthetic-input.wav");
        let data_size = 16000u32 * 2;
        let mut bytes = Vec::from(b"RIFF".as_slice());
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&32000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        bytes.resize(bytes.len() + data_size as usize, 0);
        fs::write(&input, bytes).unwrap();
        let meeting = core
            .import_audio("Synthetic meeting", &input, &input)
            .unwrap();
        let raw = serde_json::to_vec(&json!({
            "model":"OpenMOSS-Team/MOSS-Transcribe-Diarize",
            "segments":[{"start":0.0,"end":1.0,"text":"예시 문장","speaker_id":"S1"}],
            "hitTokenLimit":false,"hasUnparsedText":false,
            "processedAudioFrames":16000,"inputAudioFrames":16000,"audioDurationSeconds":1.0
        }))
        .unwrap();
        let document = core.commit_transcript(&meeting.id, None, raw).unwrap();
        let audio = core.audio_path(&meeting.id).unwrap();
        (core, meeting.id, document, audio)
    }

    #[test]
    #[ignore = "requires explicitly supplied completed live artifacts; never executes a model"]
    fn local_live_artifacts_publish_and_reopen_without_rerunning_models() {
        let configured = |name| PathBuf::from(std::env::var(name).expect(name));
        let artifacts = configured("NERO_LIVE_ARTIFACTS");
        let weights = configured("NERO_LIVE_SPEAKER_WEIGHTS");
        let native = configured("NERO_LIVE_LIBRARY");
        let audio = artifacts.join("analysis.wav");
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("isolated-library");
        let core = CoreHandle::open(&root).unwrap();
        let meeting = core
            .import_audio("Isolated live acceptance", &audio, &audio)
            .unwrap();
        let raw = fs::read(artifacts.join("recognition.json")).unwrap();
        let source = core
            .commit_nemotron_transcript(&meeting.id, None, raw)
            .unwrap();
        let initial = source.version.clone();
        let spec = model();
        let config = configuration(false);
        let draft = SpeakerAnalysisDraft {
            source,
            model: spec.repository,
            model_revision: spec.revision,
            model_sha256: spec.files[0].sha256.clone(),
            runtime: runtime().into(),
            configuration: config.clone(),
            raw: read_limited(&artifacts.join("speaker/diarization.json"), RESULT_LIMIT).unwrap(),
            posterior: Some(
                posterior(
                    &artifacts.join("speaker"),
                    &audio,
                    &weights,
                    &native,
                    &config,
                    &AtomicBool::new(false),
                )
                .unwrap(),
            ),
        };
        let final_doc = core.apply_diarization(&meeting.id, draft).unwrap();
        assert!(!final_doc.segments.is_empty());
        assert_eq!(
            final_doc
                .source
                .as_ref()
                .unwrap()
                .chunking_policy
                .as_deref(),
            Some("whole-recording-stateful-live-v1")
        );
        let restored = core
            .restore_transcript(&meeting.id, final_doc.version, initial)
            .unwrap();
        drop(core);
        let reopened = CoreHandle::open(&root).unwrap();
        assert_eq!(
            serde_json::to_value(reopened.transcript(&meeting.id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
    }

    #[test]
    #[ignore = "requires explicit recorded audio, saved transcript, shared model and packaged workers"]
    fn local_recorded_audio_speaker_acceptance() {
        let configured = |name: &str| PathBuf::from(std::env::var(name).expect(name));
        let audio = configured("NERO_ACCEPTANCE_AUDIO");
        let transcript = configured("NERO_ACCEPTANCE_TRANSCRIPT");
        let work = configured("NERO_ACCEPTANCE_WORK");
        fs::create_dir(&work).unwrap();
        let process = Processing::new(
            work.join("library"),
            configured("NERO_ACCEPTANCE_RESOURCES"),
        );
        let cancel = AtomicBool::new(false);
        let mut source: Transcript =
            serde_json::from_slice(&fs::read(transcript).unwrap()).unwrap();
        let isolated = std::env::var_os("NERO_ACCEPTANCE_ASR_RAW").map(|raw| {
            let core = CoreHandle::open(&process.root).unwrap();
            let meeting = core
                .import_audio("Recorded acceptance", &audio, &audio)
                .unwrap();
            source = core
                .commit_nemotron_transcript(&meeting.id, None, fs::read(raw).unwrap())
                .unwrap();
            (core, meeting.id)
        });
        // Same rate/depth as normalized audio, but a container the native reader rejects.
        let extensible = work.join("extensible.wav");
        let mut bytes = Vec::from(b"RIFF".as_slice());
        bytes.extend_from_slice(&(60_u32 + 3200).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        for value in [65534_u16, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [16000_u32, 32000] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [2_u16, 16, 22, 16] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&4_u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&3200_u32.to_le_bytes());
        bytes.resize(bytes.len() + 3200, 0);
        fs::write(&extensible, &bytes).unwrap();
        let ext_work = work.join("extensible-check");
        fs::create_dir(&ext_work).unwrap();
        let ext_input = AnalysisAudio::prepare(&process, &extensible, &ext_work, &cancel).unwrap();
        assert_ne!(ext_input.path(), extensible);
        assert_eq!(fs::read(&extensible).unwrap(), bytes);
        let started = std::time::Instant::now();
        let prepared = AnalysisAudio::prepare(&process, &audio, &work, &cancel).unwrap();
        let prepare_seconds = started.elapsed().as_secs_f64();
        let native_started = std::time::Instant::now();
        let draft = analyze_with_model(
            &process,
            source,
            prepared.path(),
            &work,
            &cancel,
            &configured("NERO_ACCEPTANCE_SPEAKER_MODEL_DIR"),
        )
        .unwrap();
        prepared.verify(&audio, &cancel).unwrap();
        let native: Value = serde_json::from_slice(&draft.raw).unwrap();
        assert!(!native["segments"].as_array().unwrap().is_empty());
        let speaker_seconds = native_started.elapsed().as_secs_f64();
        let source_version = draft.source.version.clone();
        let native_frames = draft.posterior.as_ref().unwrap().frames;
        let mut storage = json!(null);
        if let Some((core, meeting_id)) = isolated {
            let before = core.transcript(&meeting_id).unwrap().unwrap();
            let store_started = std::time::Instant::now();
            let after = core.apply_diarization(&meeting_id, draft).unwrap();
            assert_ne!(after.version.id, before.version.id);
            let words = |doc: &Transcript| {
                doc.segments
                    .iter()
                    .flat_map(|s| s.text.split_whitespace())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            assert_eq!(words(&before), words(&after));
            assert_eq!(
                core.transcript_history(&meeting_id).unwrap().entries.len(),
                2
            );
            let reader = nero_core::reader::LibraryReader::open(&process.root).unwrap();
            let mut read_segments = Vec::new();
            for offset in (0..after.segments.len()).step_by(200) {
                let page = reader
                    .query(
                        "transcript_get",
                        json!({"meetingId":meeting_id,"offset":offset,"limit":200}),
                    )
                    .unwrap();
                read_segments.extend(page["segments"].as_array().unwrap().iter().cloned());
            }
            assert_eq!(read_segments.len(), after.segments.len());
            for (actual, expected) in read_segments.iter().zip(&after.segments) {
                assert_eq!(actual["id"], expected.id);
                assert_eq!(actual["text"], expected.text);
                assert_eq!(actual["speakerId"], json!(expected.speaker_id));
            }
            record(
                &work.join("after.json"),
                &serde_json::to_value(&after).unwrap(),
            )
            .unwrap();
            storage = json!({"seconds":store_started.elapsed().as_secs_f64(),"historyGenerations":2,
                "beforeVersion":before.version,"afterVersion":after.version,
                "beforeSegments":before.segments.len(),"afterSegments":after.segments.len(),
                "speakers":after.speakers.len(),"readerAllPagesEqual":true,"textUnchanged":true,
                "analysis":after.speaker_analysis});
        }
        record(&work.join("report.json"), &json!({
            "input":prepared.evidence(), "prepareSeconds":prepare_seconds,
            "speakerSeconds":speaker_seconds,"nativeFrames":native_frames,"storage":storage,
            "activitySegments":native["segments"].as_array().unwrap().len(), "extensiblePcm16Normalized":true,
            "sourceVersion":source_version,
            "scope":"recorded input normalization plus native speaker execution; no user library writes or accuracy qualification"
        })).unwrap();
    }

    #[test]
    fn absent_speaker_model_preserves_the_saved_recognition_and_audio() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root);
        let (core, meeting_id, source, audio) = baseline(&process);
        let before = fs::read(&audio).unwrap();
        let cancel = AtomicBool::new(false);
        let error = analyze_recording(
            &process,
            &core,
            &meeting_id,
            source.clone(),
            &audio,
            None,
            temporary.path(),
            &cancel,
        )
        .unwrap_err();
        assert_eq!(error.kind, FailureKind::ModelNotReady);
        assert_eq!(
            core.transcript(&meeting_id).unwrap().unwrap().version,
            source.version
        );
        assert_eq!(fs::read(&audio).unwrap(), before);
        assert!(!temporary.path().join("diarization-0").exists());
    }

    #[test]
    fn changed_recognition_is_rejected_before_starting_speaker_analysis() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root);
        let (core, meeting_id, source, audio) = baseline(&process);
        let edited = core
            .edit_transcript(
                &meeting_id,
                source.version.clone(),
                vec!["수정한 문장".into()],
            )
            .unwrap();
        let error = analyze_recording(
            &process,
            &core,
            &meeting_id,
            source,
            &audio,
            None,
            temporary.path(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.kind, FailureKind::SourceChanged);
        assert_eq!(
            core.transcript(&meeting_id).unwrap().unwrap().version,
            edited.version
        );
        assert!(!temporary.path().join("diarization-0").exists());
    }

    #[test]
    fn cancellation_between_recognition_and_speaker_analysis_keeps_the_baseline() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root);
        let (core, meeting_id, source, audio) = baseline(&process);
        let error = analyze_recording(
            &process,
            &core,
            &meeting_id,
            source.clone(),
            &audio,
            None,
            temporary.path(),
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error.kind, FailureKind::Cancelled);
        assert_eq!(
            core.transcript(&meeting_id).unwrap().unwrap().version,
            source.version
        );
        assert!(!temporary.path().join("diarization-0").exists());
    }

    #[test]
    fn whole_recording_policy_is_shared_with_the_versioned_posterior_projector() {
        let config = configuration(true);
        assert_eq!(config["scope"], "recording");
        assert_eq!(config["postprocessingSource"], "nvidia-v3-probability-v1");
        assert_eq!(config["assignment"], "nemo-word-160ms-argmax-v1");
        assert_eq!(
            config["geometry"],
            json!({"spkcache":264,"fifo":40,"chunk":340,
            "updatePeriod":300,"leftContext":0,"rightContext":40})
        );
        assert_eq!(config["fullAttention"], false);
        assert_eq!(config["batching"], false);
        assert!(config.get("preset").is_none());
    }

    #[test]
    fn posterior_completion_and_hashes_must_match_the_actual_input_and_files() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root.clone());
        let (_, _, _, audio) = baseline(&process);
        let weights = root.join("pinned.gguf");
        let native = root.join("native-library");
        fs::write(&weights, b"test weight identity").unwrap();
        fs::write(&native, b"test library identity").unwrap();
        let data = vec![0u8; 101 * 8 * 4];
        let probabilities = root.join("probabilities.f32le");
        fs::write(&probabilities, &data).unwrap();
        let valid = json!({"nativeStreamFinished":true,"sourceFramesFed":16000,
            "nativeFrames":101,"expectedNativeFrames":101,"speakerSlots":8,"backend":"cpu",
            "secondsPerFrame":f64::from(0.01_f32),"probabilityBytes":data.len(),
            "runtimeVersionString":nero_core::speech_runtime::asr_version(),
            "runtimePin":nero_core::speech_runtime::pin(),
            "inputAndWeightsAndLibraryUnchanged":true,
            "postprocessingPolicy":"nemotron-v3-contiguous-threshold-0.5-v1",
            "threshold":0.5,"thresholdComparison":">",
            "geometry":{"timebaseSeconds":0.08,"spkcache":264,"fifo":40,"chunk":340,
                "updatePeriod":300,"leftContext":0,"rightContext":40},
            "sourceAudioSha256":crate::speech_audio::digest(&audio).unwrap(),
            "modelSha256":crate::speech_audio::digest(&weights).unwrap(),
            "librarySha256":crate::speech_audio::digest(&native).unwrap(),
            "probabilitySHA256":crate::speech_audio::digest(&probabilities).unwrap()});
        let receipt = root.join("receipt.json");
        fs::write(&receipt, serde_json::to_vec(&valid).unwrap()).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            posterior(
                &root,
                &audio,
                &weights,
                &native,
                &configuration(false),
                &cancel
            )
            .unwrap()
            .probabilities,
            data
        );
        for (key, value) in [
            ("nativeStreamFinished", json!(false)),
            ("nativeFrames", json!(100)),
            ("sourceFramesFed", json!(15999)),
            ("probabilitySHA256", json!("0".repeat(64))),
            ("secondsPerFrame", json!(0.01_f64)),
        ] {
            let mut changed = valid.clone();
            changed[key] = value;
            fs::write(&receipt, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert_eq!(
                posterior(
                    &root,
                    &audio,
                    &weights,
                    &native,
                    &configuration(false),
                    &cancel
                )
                .unwrap_err()
                .kind,
                FailureKind::Output
            );
        }
        fs::write(&receipt, serde_json::to_vec(&valid).unwrap()).unwrap();
        fs::write(&probabilities, &data[..data.len() - 4]).unwrap();
        assert_eq!(
            posterior(
                &root,
                &audio,
                &weights,
                &native,
                &configuration(false),
                &cancel
            )
            .unwrap_err()
            .kind,
            FailureKind::Output
        );
    }

    #[test]
    fn source_reads_and_the_speaker_commit_share_the_core_error_mapping() {
        use nero_core::CoreError;
        for (error, kind) in [
            (CoreError::MeetingNotFound, FailureKind::MeetingUnavailable),
            (
                CoreError::InvalidState("changed".into()),
                FailureKind::SourceChanged,
            ),
            (
                CoreError::InvalidInput("rejected".into()),
                FailureKind::Output,
            ),
            (CoreError::RecordingBusy, FailureKind::RecordingActive),
            (CoreError::Unavailable, FailureKind::EngineUnavailable),
            (CoreError::Storage("disk".into()), FailureKind::Storage),
            (CoreError::EditConflict, FailureKind::Storage),
        ] {
            assert_eq!(core_failure(&error), kind);
            assert_eq!(source_failure(error).kind, kind);
        }
    }

    #[test]
    fn native_storage_failures_do_not_echo_worker_paths() {
        assert!(!failure(FailureKind::Storage)
            .message
            .contains("private/path"));
    }
}
