//! Recording follows its original writer. This consumer may block or fall
//! behind on disk without backpressuring the device callback.
use crate::process_tree::TreeChild as Child;
use crate::processing::{directory, Processing};
use crate::speech_choice::SpeechEngine;
use nero_core::{CaptureReader, CoreHandle, SessionState, Snapshot};
use rubato::{FftFixedIn, Resampler};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const BLOCK: usize = 2048;
pub(super) struct Completed {
    pub audio: PathBuf,
    pub speaker: PathBuf,
    pub backend: Value,
    pub raw: Vec<u8>,
    pub session_id: String,
    pub meeting_id: String,
}
pub(super) struct LiveJob {
    pub(super) meeting_id: String,
    pub(super) session_id: String,
    pub(super) engine: SpeechEngine,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    pub running: Arc<AtomicBool>,
}
impl LiveJob {
    pub(super) fn stop(&self) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                child.kill().map_err(|e| e.to_string())?;
            }
            child.wait().map_err(|e| e.to_string())?;
        }
        self.join()
    }
    fn join(&self) -> Result<(), String> {
        let handle = self.handle.lock().unwrap().take();
        if let Some(handle) = handle {
            handle
                .join()
                .map_err(|_| "녹음 중 처리 종료를 확인하지 못했습니다.")?;
        } else {
            for _ in 0..100 {
                if !self.running.load(Ordering::Acquire) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(100));
            }
            if self.running.load(Ordering::Acquire) {
                return Err("녹음 중 처리 종료를 확인하지 못했습니다.".into());
            }
        }
        Ok(())
    }
}
impl Drop for LiveJob {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
struct RunningGuard(Arc<AtomicBool>);
impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub(super) struct FinalizationGuard<'a> {
    process: &'a Processing,
    job: Option<Arc<LiveJob>>,
}
impl Drop for FinalizationGuard<'_> {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            if job.running.load(Ordering::Acquire) && job.stop().is_err() {
                self.process.cleanup_failed();
            }
            if !job.running.load(Ordering::Acquire) {
                let mut slot = self.process.live.lock().unwrap();
                let old = if slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, job))
                {
                    slot.take()
                } else {
                    None
                };
                drop(slot);
                drop(old);
            }
        }
    }
}

struct ChildGuard {
    child: Arc<Mutex<Option<Child>>>,
    process: Arc<Processing>,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            if child.try_wait().ok().flatten().is_none()
                && (child.kill().is_err() || child.wait().is_err())
            {
                self.process.cleanup_failed();
            }
        }
    }
}
impl Processing {
    pub fn start_live(
        self: &Arc<Self>,
        core: &CoreHandle,
        snapshot: &Snapshot,
    ) -> Result<(), String> {
        let Some(session) = snapshot
            .session
            .as_ref()
            .filter(|s| s.state == SessionState::Recording)
        else {
            return Ok(());
        };
        self.cancel_live()?;
        self.update(|p| {
            p.live_transcript = Some(
                json!({"meetingId":session.meeting_id,"sessionId":session.id,
            "generation":0,"words":[],"text":"","provisional":true}),
            )
        });
        let reader = core
            .capture_reader(&session.id)
            .map_err(|e| e.to_string())?;
        let path = directory(&self.root, &format!("processing/{}", uuid::Uuid::new_v4()))?;
        fs::write(
            path.join("owner.json"),
            serde_json::to_vec(&json!({"meetingId":session.meeting_id})).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let child = Arc::new(Mutex::new(None));
        let engine = self.speech_engine();
        let job = Arc::new(LiveJob {
            engine,
            meeting_id: session.meeting_id.clone(),
            session_id: session.id.clone(),
            cancel: cancel.clone(),
            child: child.clone(),
            handle: Mutex::new(None),
            running: running.clone(),
        });
        let process = self.clone();
        let core = core.clone();
        let meeting_id = session.meeting_id.clone();
        let session_id = session.id.clone();
        *job.handle.lock().unwrap() = Some(
            thread::Builder::new()
                .name("nero-live-speech".into())
                .spawn(move || {
                    let _running = RunningGuard(running);
                    let result = run(
                        process.clone(),
                        core,
                        reader,
                        &path,
                        &meeting_id,
                        &session_id,
                        cancel,
                        child,
                        engine,
                    );
                    if let Err(error) = result {
                        let _ = fs::write(
                            path.join("live-failure.json"),
                            serde_json::to_vec(&json!({"error":error})).unwrap(),
                        );
                        process.update(|p| {
                            if let Some(preview) = p
                                .live_transcript
                                .as_mut()
                                .filter(|p| p["sessionId"] == session_id)
                            {
                                preview["deferred"] = json!(true);
                                preview["message"] =
                                    json!("녹음을 마친 뒤 음성 처리를 이어갑니다.");
                            }
                        });
                    }
                })
                .map_err(|e| e.to_string())?,
        );
        *self.live.lock().unwrap() = Some(job);
        Ok(())
    }
    pub(super) fn cancel_live(&self) -> Result<(), String> {
        let job = self.live.lock().unwrap().take();
        if let Some(job) = job {
            job.stop().inspect_err(|_| self.cleanup_failed())?;
        }
        Ok(())
    }
    pub(super) fn live_allows(&self, kind: &str, meeting_id: Option<&str>) -> bool {
        self.live.lock().unwrap().as_ref().is_none_or(|job| {
            !job.running.load(Ordering::Acquire)
                || (!self.recording.load(Ordering::Acquire)
                    && kind == "transcribe"
                    && meeting_id == Some(job.meeting_id.as_str()))
        })
    }
    pub(super) fn cancel_live_meeting(&self, meeting_id: Option<&str>) -> Result<(), String> {
        let mut slot = self.live.lock().unwrap();
        let job = if slot
            .as_ref()
            .is_some_and(|job| Some(job.meeting_id.as_str()) == meeting_id)
        {
            slot.take()
        } else {
            None
        };
        drop(slot);
        if let Some(job) = job {
            job.stop().inspect_err(|_| self.cleanup_failed())?;
        }
        Ok(())
    }
    pub(super) fn wait_live_asr(
        &self,
        meeting: &str,
        session: &str,
        cancel: &AtomicBool,
    ) -> Result<FinalizationGuard<'_>, String> {
        let job = self
            .live
            .lock()
            .unwrap()
            .clone()
            .filter(|j| j.meeting_id == meeting && j.session_id == session);
        let guard = FinalizationGuard { process: self, job };
        if let Some(job) = &guard.job {
            while job.running.load(Ordering::Acquire)
                && !self.live_completed.lock().unwrap().contains_key(session)
            {
                crate::processing::cancelled(cancel)?;
                thread::sleep(Duration::from_millis(50));
            }
        }
        crate::processing::cancelled(cancel)?;
        Ok(guard)
    }
    pub(super) fn wait_live(
        &self,
        meeting_id: &str,
        session_id: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let job = self.live.lock().unwrap().clone();
        if let Some(job) = job.filter(|j| j.meeting_id == meeting_id && j.session_id == session_id)
        {
            while job.running.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) {
                    job.stop().inspect_err(|_| self.cleanup_failed())?;
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            job.join()?;
        }
        crate::processing::cancelled(cancel)
    }
    pub(super) fn take_live(&self, meeting_id: &str, session_id: &str) -> Option<Completed> {
        let mut completed = self.live_completed.lock().unwrap();
        if completed
            .get(session_id)
            .is_none_or(|result| result.meeting_id != meeting_id)
        {
            return None;
        }
        let result = completed.remove(session_id);
        drop(completed);
        let mut slot = self.live.lock().unwrap();
        if slot.as_ref().is_some_and(|j| {
            j.meeting_id == meeting_id
                && j.session_id == session_id
                && !j.running.load(Ordering::Acquire)
        }) {
            let old = slot.take();
            drop(slot);
            drop(old);
        }
        result
    }
}
fn run(
    process: Arc<Processing>,
    core: CoreHandle,
    mut reader: CaptureReader,
    path: &Path,
    meeting_id: &str,
    session_id: &str,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    engine: SpeechEngine,
) -> Result<(), String> {
    let _guard = ChildGuard {
        child: child.clone(),
        process: process.clone(),
    };
    let asr = Processing::model_for(engine);
    let model_path = process.model_dir_for(engine)?.join(&asr.files[0].name);
    let speaker_model = crate::diarization::model();
    let speaker_weights =
        crate::diarization::model_dir(&process)?.join(&speaker_model.files[0].name);
    let gpu = crate::diarization::library(&process, true).is_file();
    let library = crate::diarization::library(&process, gpu);
    let backend = json!(if gpu {
        if cfg!(target_os = "macos") {
            "metal"
        } else {
            "vulkan"
        }
    } else {
        "cpu"
    });
    let speaker_path = path.join("speaker");
    fs::create_dir(&speaker_path).map_err(|e| e.to_string())?;
    let audio = path.join("analysis.wav");
    let request = json!({"asrEngine":engine,"whisperExecutable":crate::whisper_asr::executable(&process.resources),
        "asrModel":model_path,"asrLibrary":library,"meetingId":meeting_id,"sessionId":session_id,
        "speaker":{"schemaVersion":1,"audio":audio,"model":speaker_weights,"library":library,
            "output":speaker_path,"backend":backend,"modelSha256":speaker_model.files[0].sha256,
            "runtimePin":nero_core::speech_runtime::pin()}});
    let request_path = path.join("live-request.json");
    fs::write(&request_path, serde_json::to_vec_pretty(&request).unwrap())
        .map_err(|e| e.to_string())?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("worker.log"))
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("--nero-live-speech-worker")
        .arg(&request_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000);
    }
    let mut spawned = crate::process_tree::spawn(&mut command)?;
    let mut stdin = BufWriter::new(spawned.stdin.take().ok_or("Missing live input pipe")?);
    let stdout = spawned.stdout.take().ok_or("Missing live result pipe")?;
    *child.lock().unwrap() = Some(spawned);
    let asr_ready = Arc::new(AtomicBool::new(false));
    let ready = asr_ready.clone();
    let preview_process = process.clone();
    let preview_meeting = meeting_id.to_owned();
    let preview_session = session_id.to_owned();
    let preview = thread::spawn(move || {
        let mut lines = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let mut limited = std::io::Read::take(&mut lines, 16 * 1024 * 1024 + 1);
            match limited.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if line.len() > 16 * 1024 * 1024 {
                break;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                if value["kind"] == "asr-finished"
                    && value["meetingId"] == preview_meeting
                    && value["sessionId"] == preview_session
                {
                    ready.store(true, Ordering::Release);
                }
                if value["kind"] == "preview"
                    && value["meetingId"] == preview_meeting
                    && value["sessionId"] == preview_session
                {
                    preview_process.update(|p| p.live_transcript = Some(value));
                }
            }
        }
    });
    let raw_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&audio)
        .map_err(|e| e.to_string())?;
    let sync = raw_file.try_clone().map_err(|e| e.to_string())?;
    let mut writer = hound::WavWriter::new(
        BufWriter::new(raw_file),
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut normalizer = Normalizer::new(reader.sample_rate())?;
    let mut sent = 0_u64;
    let mut input_hash = Sha256::new();
    let stream_result: Result<(), String> = (|| {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Live processing cancelled".into());
            }
            let finished = reader.finished();
            let available = reader
                .available_frames()
                .saturating_sub(reader.consumed_frames());
            if available < BLOCK as u64 && !finished {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            if available == 0 && finished {
                break;
            }
            let samples = reader.read_mono(BLOCK).map_err(|e| e.to_string())?;
            let normalized = normalizer.push(&samples, false)?;
            send(
                &normalized,
                &mut stdin,
                &mut writer,
                &mut sent,
                &mut input_hash,
            )?;
        }
        if !reader.successful() {
            return Err("Recording did not finish successfully".into());
        }
        let tail = normalizer.push(&[], true)?;
        send(&tail, &mut stdin, &mut writer, &mut sent, &mut input_hash)?;
        writer.finalize().map_err(|e| e.to_string())?;
        sync.sync_all().map_err(|e| e.to_string())?;
        stdin
            .write_all(&0_u32.to_le_bytes())
            .and_then(|_| stdin.write_all(&sent.to_le_bytes()))
            .and_then(|_| stdin.flush())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    drop(stdin);
    if stream_result.is_err() {
        if let Some(child) = child.lock().unwrap().as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let input_sha = format!("{:x}", input_hash.finalize());
    let mut published = false;
    let status = loop {
        if stream_result.is_ok() && asr_ready.load(Ordering::Acquire) && !published {
            publish_asr(
                &process, &core, path, meeting_id, session_id, &backend, sent, &input_sha, engine,
            )?;
            published = true;
        }
        let status = child
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("Missing live worker")?
            .try_wait()
            .map_err(|e| e.to_string())?;
        if let Some(status) = status {
            break status;
        }
        if cancel.load(Ordering::Acquire) {
            if let Some(child) = child.lock().unwrap().as_mut() {
                child.kill().map_err(|e| e.to_string())?;
            }
        }
        thread::sleep(Duration::from_millis(50));
    };
    preview
        .join()
        .map_err(|_| "Live result reader stopped unexpectedly")?;
    stream_result?;
    if !published {
        publish_asr(
            &process, &core, path, meeting_id, session_id, &backend, sent, &input_sha, engine,
        )?;
    }
    fs::write(
        path.join("parent-completion.json"),
        serde_json::to_vec(&json!({"workerExitSuccess":status.success(),"streamFramesFed":sent}))
            .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn publish_asr(
    process: &Processing,
    core: &CoreHandle,
    path: &Path,
    meeting: &str,
    session: &str,
    backend: &Value,
    frames: u64,
    pcm_sha: &str,
    engine: SpeechEngine,
) -> Result<(), String> {
    let raw = read_limited(&path.join("recognition.json"), 16 * 1024 * 1024)?;
    let result: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let model: Value = serde_json::from_str(engine.manifest()).map_err(|e| e.to_string())?;
    if result["inputAudioFrames"] != frames
        || result["inputPcmSha256"] != pcm_sha
        || result["model"] != model["repository"]
        || result["modelRevision"] != model["revision"]
        || match engine {
            SpeechEngine::Nemotron => {
                result["streamFramesFed"] != frames || result["nativeStreamFinished"] != true
            }
            SpeechEngine::WhisperTurbo => {
                result["completed"] != true
                    || result["completionEvidence"] != "whisper-cli-windows-exit0-v1"
            }
        }
    {
        return Err("Live ASR input identity differs from capture".into());
    }
    let snapshot = core.snapshot().map_err(|e| e.to_string())?;
    if !snapshot
        .meetings
        .iter()
        .any(|m| m.id == meeting && m.audio.as_ref().is_some_and(|a| a.session_id == session))
    {
        return Err("Live capture source changed".into());
    }
    let audio = path.join("analysis.wav");
    let source = core.audio_path(meeting).map_err(|e| e.to_string())?;
    let receipt = json!({"schemaVersion":1,"speechEngine":engine,"meetingId":meeting,"sessionId":session,"backend":backend,
        "audioSha256":crate::speech_audio::digest(&audio).map_err(|e|e.message)?,
        "sourceSha256":crate::speech_audio::digest(&source).map_err(|e|e.message)?,
        "recognitionSha256":format!("{:x}",Sha256::digest(&raw))});
    let mut file = tempfile::NamedTempFile::new_in(path).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, &receipt).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist_noclobber(path.join("parent-asr-completion.json"))
        .map_err(|e| e.to_string())?;
    process.live_completed.lock().unwrap().insert(
        session.into(),
        Completed {
            audio,
            speaker: path.join("speaker"),
            backend: backend.clone(),
            raw,
            session_id: session.into(),
            meeting_id: meeting.into(),
        },
    );
    Ok(())
}
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = crate::processing::plain_file(path)?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err("Live artifact exceeds limit".into());
    }
    let mut raw = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() as u64 > limit {
        return Err("Live artifact exceeds limit".into());
    }
    Ok(raw)
}
impl Processing {
    pub(super) fn recover_live(
        &self,
        meeting: &str,
        session: &str,
        source: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<Completed>, String> {
        let root = self.root.join("processing");
        if !root.exists() {
            return Ok(None);
        }
        let root = directory(&self.root, "processing")?;
        let source_sha = crate::speech_audio::digest(source).map_err(|e| e.message)?;
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            crate::processing::cancelled(cancel)?;
            let entry = entry.map_err(|e| e.to_string())?;
            if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
                || !entry.file_type().map_err(|e| e.to_string())?.is_dir()
            {
                continue;
            }
            let path = entry.path();
            let candidate = (|| -> Result<Completed, String> {
                let receipt: Value = serde_json::from_slice(&read_limited(
                    &path.join("parent-asr-completion.json"),
                    65536,
                )?)
                .map_err(|e| e.to_string())?;
                if receipt["schemaVersion"] != 1
                    || receipt["meetingId"] != meeting
                    || receipt["sessionId"] != session
                    || receipt["sourceSha256"] != source_sha
                    || !matches!(
                        receipt["backend"].as_str(),
                        Some("metal" | "vulkan" | "cpu")
                    )
                {
                    return Err("Unrelated live artifact".into());
                }
                let audio = path.join("analysis.wav");
                if crate::speech_audio::digest(&audio).map_err(|e| e.message)?
                    != receipt["audioSha256"]
                {
                    return Err("Live prepared audio changed".into());
                }
                let raw = read_limited(&path.join("recognition.json"), 16 * 1024 * 1024)?;
                if format!("{:x}", Sha256::digest(&raw)) != receipt["recognitionSha256"] {
                    return Err("Live recognition changed".into());
                }
                Ok(Completed {
                    audio,
                    speaker: path.join("speaker"),
                    backend: receipt["backend"].clone(),
                    raw,
                    meeting_id: meeting.into(),
                    session_id: session.into(),
                })
            })();
            if let Ok(candidate) = candidate {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}

fn send(
    samples: &[i16],
    stdin: &mut impl Write,
    writer: &mut hound::WavWriter<BufWriter<File>>,
    sent: &mut u64,
    hash: &mut Sha256,
) -> Result<(), String> {
    for block in samples.chunks(16000) {
        if block.is_empty() {
            continue;
        }
        stdin
            .write_all(&(block.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        for sample in block {
            writer.write_sample(*sample).map_err(|e| e.to_string())?;
            hash.update(sample.to_le_bytes());
            stdin
                .write_all(&sample.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        stdin.flush().map_err(|e| e.to_string())?;
        *sent += block.len() as u64;
    }
    Ok(())
}
pub(super) struct Normalizer {
    rate: u32,
    resampler: Option<FftFixedIn<f64>>,
    delay: usize,
    consumed: u64,
    written: u64,
    pending: Vec<f64>,
    finished: bool,
}
impl Normalizer {
    pub(super) fn new(rate: u32) -> Result<Self, String> {
        if rate == 0 || rate > 768000 {
            return Err("Invalid capture rate".into());
        }
        let resampler = if rate == 16000 {
            None
        } else {
            Some(FftFixedIn::new(rate as usize, 16000, BLOCK, 2, 1).map_err(|e| e.to_string())?)
        };
        let delay = resampler.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            rate,
            resampler,
            delay,
            consumed: 0,
            written: 0,
            pending: Vec::new(),
            finished: false,
        })
    }
    pub(super) fn push(&mut self, samples: &[f64], finish: bool) -> Result<Vec<i16>, String> {
        if self.finished || samples.len() > BLOCK || samples.iter().any(|s| !s.is_finite()) {
            return Err("Invalid normalized input lifecycle".into());
        }
        self.consumed += samples.len() as u64;
        self.pending.extend_from_slice(samples);
        let expected =
            (u128::from(self.consumed) * 16000 + u128::from(self.rate / 2)) / u128::from(self.rate);
        let mut result = Vec::new();
        let mut flushes = 0;
        while self.pending.len() >= BLOCK || (finish && self.written < expected as u64) {
            let take = BLOCK.min(self.pending.len());
            let mut input = vec![vec![0.0; BLOCK]];
            input[0][..take].copy_from_slice(&self.pending[..take]);
            self.pending.drain(..take);
            let output = if let Some(resampler) = &mut self.resampler {
                resampler
                    .process(&input, None)
                    .map_err(|e| e.to_string())?
                    .remove(0)
            } else {
                input.remove(0)[..take].to_vec()
            };
            let skip = self.delay.min(output.len());
            self.delay -= skip;
            for sample in &output[skip..] {
                if self.written >= expected as u64 {
                    break;
                }
                if !sample.is_finite() {
                    return Err("Invalid resampler output".into());
                }
                result.push(
                    (sample.clamp(-1.0, 1.0) * 32768.0)
                        .round()
                        .clamp(-32768.0, 32767.0) as i16,
                );
                self.written += 1;
            }
            flushes += 1;
            if flushes > 64 {
                return Err("Resampler exceeded bounded flush".into());
            }
        }
        if finish {
            self.finished = true;
            if !self.pending.is_empty() || self.written != expected as u64 {
                return Err("Incomplete resampling tail".into());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_live_results_remain_bound_to_their_meeting_and_capture() {
        let temp = tempfile::tempdir().unwrap();
        let process = Processing::new(
            temp.path().join("isolated-library"),
            temp.path().join("resources"),
        );
        process.live_completed.lock().unwrap().insert(
            "capture-a".into(),
            Completed {
                audio: temp.path().join("analysis.wav"),
                speaker: temp.path().join("speaker"),
                backend: json!("cpu"),
                raw: vec![1, 2, 3],
                session_id: "capture-a".into(),
                meeting_id: "meeting-a".into(),
            },
        );
        assert!(process.take_live("meeting-b", "capture-a").is_none());
        assert!(process.take_live("meeting-a", "capture-b").is_none());
        process.cancel_live().unwrap();
        assert_eq!(
            process.take_live("meeting-a", "capture-a").unwrap().raw,
            vec![1, 2, 3]
        );
        assert!(process.take_live("meeting-a", "capture-a").is_none());
    }
}

#[cfg(test)]
pub(super) fn waiting_fixture(process: &Processing, meeting: &str, session: &str) -> Arc<LiveJob> {
    let cancel = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let flag = cancel.clone();
    let active = running.clone();
    let handle = thread::spawn(move || {
        let _running = RunningGuard(active);
        while !flag.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
    });
    let job = Arc::new(LiveJob {
        engine: SpeechEngine::Nemotron,
        meeting_id: meeting.into(),
        session_id: session.into(),
        cancel,
        child: Arc::new(Mutex::new(None)),
        handle: Mutex::new(Some(handle)),
        running,
    });
    *process.live.lock().unwrap() = Some(job.clone());
    job
}

#[cfg(all(test, unix))]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn cancelling_finalization_reaps_a_child_blocked_after_eof() {
        use std::io::Read;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root);
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let guard = active.clone();
        let handle = thread::spawn(move || {
            let _guard = RunningGuard(guard);
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).unwrap();
        });
        let child = Arc::new(Mutex::new(Some(child)));
        *process.live.lock().unwrap() = Some(Arc::new(LiveJob {
            engine: SpeechEngine::Nemotron,
            meeting_id: "meeting".into(),
            session_id: "session".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            child: child.clone(),
            handle: Mutex::new(Some(handle)),
            running: active.clone(),
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let waiting = process.clone();
        let handle = thread::spawn(move || waiting.wait_live("meeting", "session", &flag));
        cancel.store(true, Ordering::Release);
        assert!(handle.join().unwrap().is_err());
        assert!(!active.load(Ordering::Acquire));
        assert!(child
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_some());
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn synced_asr_is_available_before_speakers_and_survives_processor_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root.clone());
        let input = root.join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..16000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        let core = CoreHandle::open(&root).unwrap();
        let meeting = core.import_audio("Synthetic", &input, &input).unwrap();
        let session = &meeting.audio.as_ref().unwrap().session_id;
        let source = core.audio_path(&meeting.id).unwrap();
        let path = directory(&root, &format!("processing/{}", uuid::Uuid::new_v4())).unwrap();
        fs::copy(&input, path.join("analysis.wav")).unwrap();
        let pcm = format!("{:x}", Sha256::digest(vec![0u8; 32000]));
        let raw = serde_json::to_vec(&json!({"streamFramesFed":16000,"inputAudioFrames":16000,
            "nativeStreamFinished":true,"inputPcmSha256":pcm,
            "model":"nvidia/nemotron-3.5-asr-streaming-0.6b","modelRevision":"1c8deaecc64b91f034d73e08dd8b64625eb3395d"}))
        .unwrap();
        fs::write(path.join("recognition.json"), &raw).unwrap();
        let live = waiting_fixture(&process, &meeting.id, session);
        publish_asr(
            &process,
            &core,
            &path,
            &meeting.id,
            session,
            &json!("cpu"),
            16000,
            &pcm,
            SpeechEngine::Nemotron,
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let guard = process
            .wait_live_asr(&meeting.id, session, &cancel)
            .unwrap();
        assert!(live.running.load(Ordering::Acquire));
        assert_eq!(process.take_live(&meeting.id, session).unwrap().raw, raw);
        drop(guard);
        assert!(!live.running.load(Ordering::Acquire));
        let restarted = Processing::new(root.clone(), root);
        assert_eq!(
            restarted
                .recover_live(&meeting.id, session, &source, &cancel)
                .unwrap()
                .unwrap()
                .raw,
            raw
        );
        assert!(restarted
            .recover_live(&meeting.id, "other-session", &source, &cancel)
            .unwrap()
            .is_none());
        fs::write(path.join("recognition.json"), b"changed").unwrap();
        assert!(restarted
            .recover_live(&meeting.id, session, &source, &cancel)
            .unwrap()
            .is_none());
        fs::write(path.join("recognition.json"), &raw).unwrap();
        fs::write(&source, b"changed original").unwrap();
        assert!(restarted
            .recover_live(&meeting.id, session, &source, &cancel)
            .unwrap()
            .is_none());
    }
}
