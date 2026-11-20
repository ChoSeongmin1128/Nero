//! Recognizes whole MOSS chunks of the current recording while it is captured, so the
//! transcription after recording only has the rest left. Each chunk is read from saved
//! capture PCM through the same normalizer as the final playback WAV, written as its
//! own WAV and kept as a receipt. The final job checks every receipt against the
//! recording before using it; this processor never commits a transcript.
use crate::job_control::JobControl;
use crate::live_speech::LiveJob;
use crate::moss_asr::{self, Seed};
use crate::processing::{directory, plain_file, ModelSpec, Processing};
use crate::speech_audio::normalize::Normalizer;
use crate::speech_choice::SpeechEngine;
use nero_core::{CaptureReader, CoreHandle, MOSS_CHUNK_FRAMES};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

const MARKER: &str = "live-moss.json";
const PREFERENCE: &str = "preferences/recording-transcription.json";
const BLOCK: usize = 2048;
// Chunk WAVs waiting for the worker. Beyond this the final job recognizes them.
const PENDING_LIMIT: usize = 5;

/// Whether recordings that use MOSS recognize finished chunks while recording.
pub(crate) fn enabled(root: &Path) -> bool {
    plain_file(&root.join(PREFERENCE))
        .ok()
        .and_then(|file| {
            let mut text = String::new();
            file.take(4096).read_to_string(&mut text).ok()?;
            serde_json::from_str::<Value>(&text).ok()
        })
        .and_then(|value| value["enabled"].as_bool())
        .unwrap_or(false)
}

pub(crate) fn set_enabled(root: &Path, enabled: bool) -> Result<(), String> {
    let path = root.join(PREFERENCE);
    let parent = path.parent().expect("preference directory");
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, &json!({"schemaVersion":1,"enabled":enabled}))
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(&path).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, value).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Collects normalized 16 kHz samples and closes each whole chunk as a WAV of
/// `moss_chunk_wav_header` followed by its PCM16, with that file's digest.
pub(crate) struct ChunkWriter {
    directory: PathBuf,
    samples: Vec<i16>,
    next: u64,
}

pub(crate) struct ChunkFile {
    pub index: u64,
    pub path: PathBuf,
    pub sha256: String,
}

impl ChunkWriter {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            samples: Vec::with_capacity(MOSS_CHUNK_FRAMES as usize),
            next: 0,
        }
    }

    /// Adds samples; returns every chunk they completed. `keep` false drops a chunk
    /// instead of writing it, while the chunk count still advances.
    pub(crate) fn push(
        &mut self,
        mut samples: &[i16],
        keep: impl Fn() -> bool,
    ) -> Result<Vec<ChunkFile>, String> {
        let mut done = Vec::new();
        while !samples.is_empty() {
            let room = MOSS_CHUNK_FRAMES as usize - self.samples.len();
            let take = room.min(samples.len());
            self.samples.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.samples.len() == MOSS_CHUNK_FRAMES as usize {
                let index = self.next;
                self.next += 1;
                if keep() {
                    done.push(self.write(index)?);
                }
                self.samples.clear();
            }
        }
        Ok(done)
    }

    fn write(&self, index: u64) -> Result<ChunkFile, String> {
        let header =
            nero_core::moss_chunk_wav_header(MOSS_CHUNK_FRAMES).map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(44 + self.samples.len() * 2);
        bytes.extend_from_slice(&header);
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let path = self.directory.join(format!("chunk-{index}.wav"));
        let mut file = File::create(&path).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(crate::storage_space::write_error)?;
        Ok(ChunkFile {
            index,
            path,
            sha256,
        })
    }
}

impl Processing {
    /// Starts the recording-time processor for a MOSS session. Model hashing and every
    /// worker run happen on its own thread, never while a recording is starting.
    pub(crate) fn start_live_moss(
        self: &Arc<Self>,
        core: &CoreHandle,
        meeting_id: &str,
        session_id: &str,
    ) -> Result<(), String> {
        let reader = core.capture_reader(session_id).map_err(|e| e.to_string())?;
        let path = directory(&self.root, &format!("processing/{}", uuid::Uuid::new_v4()))?;
        Processing::bind_job(&path, meeting_id)?;
        write_json(
            &path.join(MARKER),
            &json!({"schemaVersion":1,"meetingId":meeting_id,"sessionId":session_id}),
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        // Never paused with SIGSTOP: a stopped worker would keep the model in memory.
        let control = JobControl::new(cancel.clone(), false);
        control.set_throttle(true);
        let job = Arc::new(LiveJob::new(
            meeting_id,
            session_id,
            SpeechEngine::Moss,
            cancel.clone(),
            running.clone(),
            Some(control.clone()),
        ));
        let process = self.clone();
        let handle = thread::Builder::new()
            .name("nero-live-moss".into())
            .spawn(move || {
                let _running = crate::live_speech::RunningGuard::new(running);
                if let Err(error) = run(&process, reader, &path, &cancel, &control) {
                    let _ = write_json(&path.join("live-failure.json"), &json!({"error":error}));
                }
                if control.termination_unconfirmed() {
                    process.cleanup_failed();
                }
            })
            .map_err(|e| e.to_string())?;
        job.set_handle(handle);
        *self.live.lock().unwrap() = Some(job);
        Ok(())
    }
}

fn run(
    process: &Processing,
    mut reader: CaptureReader,
    path: &Path,
    cancel: &AtomicBool,
    control: &JobControl,
) -> Result<(), String> {
    crate::job_control::throttle_this_thread(true);
    let spec = ModelSpec::speech(SpeechEngine::Moss);
    let model = process
        .verified_model_dir(spec, cancel)
        .map_err(|error| error.message)?;
    let provenance = moss_asr::provenance(&process.resources).map_err(|error| error.message)?;
    let chunks = path.join("chunks");
    fs::create_dir(&chunks).map_err(|e| e.to_string())?;
    let mut writer = ChunkWriter::new(chunks);
    let mut normalizer = Normalizer::new(reader.sample_rate())?;
    let pending: Mutex<VecDeque<ChunkFile>> = Mutex::new(VecDeque::new());
    let mut attempt = 0_u32;
    let result = (|| -> Result<(), String> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            let finished = reader.finished();
            let available = reader
                .available_frames()
                .saturating_sub(reader.consumed_frames());
            if available == 0 && finished {
                // The chunk in flight has finished; the final job recognizes the rest.
                return Ok(());
            }
            if available >= BLOCK as u64 || (finished && available > 0) {
                let samples = reader.read_mono(BLOCK).map_err(|e| e.to_string())?;
                let normalized = normalizer.push(&samples, false)?;
                let room = || pending.lock().unwrap().len() < PENDING_LIMIT;
                let done = writer.push(&normalized, room)?;
                pending.lock().unwrap().extend(done);
                continue;
            }
            let next = (control.auto_reason().is_none())
                .then(|| pending.lock().unwrap().pop_front())
                .flatten();
            let Some(chunk) = next else {
                thread::sleep(Duration::from_millis(100));
                continue;
            };
            if !process.model_ready(spec) {
                return Err("MOSS 모델이 처리 중 변경됐습니다.".into());
            }
            attempt += 1;
            let work = path.join(format!("run-{}-{attempt}", chunk.index));
            fs::create_dir(&work).map_err(|e| e.to_string())?;
            let outputs = moss_asr::recognize_wav(
                &process.resources,
                &chunk.path,
                &chunk.sha256,
                MOSS_CHUNK_FRAMES,
                &work,
                cancel,
                &model,
                &|| control.auto_reason().is_some(),
            );
            match outputs {
                Ok(outputs) if !outputs.is_empty() => {
                    let (_, raw) = &outputs[0];
                    receipt(path, &chunk, raw, &provenance)?;
                    let _ = fs::remove_file(&chunk.path);
                    let _ = fs::remove_dir_all(&work);
                }
                // Stopped for the recording's sake: try this chunk again later.
                _ if control.auto_reason().is_some() && !cancel.load(Ordering::Acquire) => {
                    pending.lock().unwrap().push_front(chunk);
                    let _ = fs::remove_dir_all(&work);
                }
                // Any other failure leaves the chunk to the final job.
                _ => {
                    let _ = fs::remove_file(&chunk.path);
                }
            }
        }
    })();
    for chunk in pending.lock().unwrap().drain(..) {
        let _ = fs::remove_file(chunk.path);
    }
    result
}

fn receipt(path: &Path, chunk: &ChunkFile, raw: &[u8], provenance: &Value) -> Result<(), String> {
    let output = path.join(format!("chunk-{}.json", chunk.index));
    let mut file = File::create(&output).map_err(|e| e.to_string())?;
    file.write_all(raw)
        .and_then(|()| file.sync_all())
        .map_err(crate::storage_space::write_error)?;
    write_json(
        &path.join(format!("receipt-{}.json", chunk.index)),
        &json!({"schemaVersion":1,"index":chunk.index,
            "input":{"sha256":chunk.sha256,"frames":MOSS_CHUNK_FRAMES,"offsetFrames":chunk.index * MOSS_CHUNK_FRAMES},
            "outputSha256":format!("{:x}", Sha256::digest(raw)),"provenance":provenance}),
    )
}

/// Every chunk recognized while `session_id` recorded `meeting_id`, with the folders
/// that hold them. The final job still checks each against the recording.
pub(crate) fn seeds(root: &Path, meeting_id: &str, session_id: &str) -> (Vec<Seed>, Vec<PathBuf>) {
    let mut seeds = Vec::new();
    let mut folders = Vec::new();
    let Ok(entries) = fs::read_dir(root.join("processing")) else {
        return (seeds, folders);
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        let marker = fs::read(folder.join(MARKER))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if marker.is_none_or(|marker| {
            marker["meetingId"] != meeting_id || marker["sessionId"] != session_id
        }) {
            continue;
        }
        folders.push(folder.clone());
        let Ok(files) = fs::read_dir(&folder) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.starts_with("receipt-") {
                continue;
            }
            let Some(receipt) = fs::read(file.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            else {
                continue;
            };
            let Some(index) = receipt["index"].as_u64().and_then(|i| u32::try_from(i).ok())
            else {
                continue;
            };
            let Ok(raw) = fs::read(folder.join(format!("chunk-{index}.json"))) else {
                continue;
            };
            if receipt["outputSha256"] != format!("{:x}", Sha256::digest(&raw)) {
                continue;
            }
            seeds.push(Seed {
                index,
                raw,
                input: receipt["input"].clone(),
                provenance: receipt["provenance"].clone(),
            });
        }
    }
    seeds.sort_by_key(|seed| seed.index);
    (seeds, folders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preference_is_off_until_turned_on() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(!enabled(temporary.path()));
        set_enabled(temporary.path(), true).unwrap();
        assert!(enabled(temporary.path()));
        set_enabled(temporary.path(), false).unwrap();
        assert!(!enabled(temporary.path()));
    }

    // Streamed in uneven reads like the capture reader, each chunk file is the
    // canonical header plus exactly that range of the finished recording's samples.
    #[test]
    fn streamed_chunk_files_equal_the_same_ranges_of_the_finished_recording() {
        for rate in [48_000_u32, 44_100] {
            let frames = rate as usize * 125;
            let input: Vec<f64> = (0..frames)
                .map(|i| ((i as f64) * 0.013).sin() * 0.4 + ((i % 97) as f64 - 48.0) / 400.0)
                .collect();
            let mut batch = Normalizer::new(rate).unwrap();
            let mut finished = Vec::new();
            for block in input.chunks(BLOCK) {
                finished.extend(batch.push(block, false).unwrap());
            }
            finished.extend(batch.push(&[], true).unwrap());
            let temporary = tempfile::tempdir().unwrap();
            let mut stream = Normalizer::new(rate).unwrap();
            let mut writer = ChunkWriter::new(temporary.path().to_path_buf());
            let mut files = Vec::new();
            for read in input.chunks(137).collect::<Vec<_>>().chunks(15) {
                let samples: Vec<f64> = read.concat();
                for block in samples.chunks(BLOCK) {
                    let normalized = stream.push(block, false).unwrap();
                    files.extend(writer.push(&normalized, || true).unwrap());
                }
            }
            assert_eq!(files.len(), 2, "{rate}");
            for file in files {
                let start = (file.index * MOSS_CHUNK_FRAMES) as usize;
                let mut expected =
                    nero_core::moss_chunk_wav_header(MOSS_CHUNK_FRAMES).unwrap().to_vec();
                for sample in &finished[start..start + MOSS_CHUNK_FRAMES as usize] {
                    expected.extend_from_slice(&sample.to_le_bytes());
                }
                assert_eq!(fs::read(&file.path).unwrap(), expected, "{rate} {}", file.index);
            }
        }
    }

    #[test]
    fn receipts_are_found_on_disk_for_their_own_recording_only_with_intact_output() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let folder = root.join("processing/one");
        fs::create_dir_all(&folder).unwrap();
        write_json(
            &folder.join(MARKER),
            &json!({"schemaVersion":1,"meetingId":"meeting","sessionId":"session"}),
        )
        .unwrap();
        let chunk = |index: u64| ChunkFile {
            index,
            path: folder.join(format!("chunk-{index}.wav")),
            sha256: "a".repeat(64),
        };
        receipt(&folder, &chunk(0), b"zero", &json!({"p":1})).unwrap();
        receipt(&folder, &chunk(1), b"one", &json!({"p":1})).unwrap();
        fs::write(folder.join("chunk-1.json"), b"changed").unwrap();
        let (seeds, folders) = seeds(root, "meeting", "session");
        assert_eq!(folders, [folder.clone()]);
        assert_eq!(seeds.len(), 1);
        assert_eq!((seeds[0].index, seeds[0].raw.as_slice()), (0, &b"zero"[..]));
        assert_eq!(seeds[0].input["offsetFrames"], 0);
        assert_eq!(seeds[0].provenance, json!({"p":1}));
        assert!(super::seeds(root, "meeting", "other").0.is_empty());
        assert!(super::seeds(root, "other", "session").1.is_empty());
    }

    #[test]
    fn chunk_files_hold_whole_chunks_and_a_full_queue_drops_but_still_counts_them() {
        let temporary = tempfile::tempdir().unwrap();
        let mut writer = ChunkWriter::new(temporary.path().to_path_buf());
        let samples: Vec<i16> = (0..MOSS_CHUNK_FRAMES + 10).map(|i| (i % 7) as i16).collect();
        let first = writer.push(&samples[..1000], || true).unwrap();
        assert!(first.is_empty());
        let done = writer.push(&samples[1000..], || true).unwrap();
        assert_eq!(done.len(), 1);
        let bytes = fs::read(&done[0].path).unwrap();
        assert_eq!(bytes.len() as u64, 44 + MOSS_CHUNK_FRAMES * 2);
        assert_eq!(done[0].sha256, format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(bytes[44..46], samples[0].to_le_bytes());
        let dropped = writer
            .push(&vec![0; MOSS_CHUNK_FRAMES as usize], || false)
            .unwrap();
        assert!(dropped.is_empty());
        let next = writer
            .push(&vec![0; MOSS_CHUNK_FRAMES as usize], || true)
            .unwrap();
        assert_eq!(next[0].index, 2);
    }
}
