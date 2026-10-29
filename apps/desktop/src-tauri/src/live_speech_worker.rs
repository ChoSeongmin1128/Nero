//! Isolated ASR and independent speaker streams. PCM is read only after the
//! recorder has saved it; EOF without the explicit final frame count is failure.
use crate::{live_asr::Asr, speaker_worker as speaker};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::time::Instant;

type Result<T> = std::result::Result<T, String>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    asr_model: PathBuf,
    asr_library: PathBuf,
    speaker: speaker::Request,
    meeting_id: String,
    session_id: String,
}
fn digest(path: &Path) -> Result<String> {
    crate::speech_audio::digest(path).map_err(|e| e.message)
}
fn save(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, value).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn emit(value: &Value) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|e| e.to_string())?;
    out.write_all(b"\n")
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}
pub(super) fn entry(path: &Path) -> Result<ExitPolicy> {
    let request: Request =
        serde_json::from_reader(File::open(path).map_err(|e| e.to_string())?.take(65537))
            .map_err(|e| e.to_string())?;
    let manifest: Value = serde_json::from_str(crate::asr::MANIFEST).map_err(|e| e.to_string())?;
    let model_sha = digest(&request.asr_model)?;
    if Some(model_sha.as_str()) != manifest["files"][0]["sha256"].as_str()
        || std::fs::metadata(&request.asr_model)
            .map_err(|e| e.to_string())?
            .len()
            != manifest["files"][0]["bytes"]
                .as_u64()
                .ok_or("Missing model pin")?
    {
        return Err("ASR model does not match its pin".into());
    }
    let speaker_sha = digest(&request.speaker.model)?;
    if speaker_sha != "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1" {
        return Err("Speaker model does not match its pin".into());
    }
    let library_sha = digest(&request.asr_library)?;
    let speaker_library_sha = digest(&request.speaker.library)?;
    // Unfinished multi-model destruction faults on the pinned Windows runtime.
    // Preserve those handles on error; completed streams close in reverse order.
    let mut asr = ManuallyDrop::new(Asr::open(
        &request.asr_library,
        &request.asr_model,
        request.speaker.backend.gpu(),
    )?);
    let mut speaker_api = ManuallyDrop::new(
        speaker::Api::load(&request.speaker.library, request.speaker.backend)
            .map_err(|e| e.to_string()),
    );
    let mut speaker_error = speaker_api.as_ref().err().cloned();
    let mut diar = match &*speaker_api {
        Ok(api) => {
            match speaker::Native::open(api, &request.speaker.model, request.speaker.backend) {
                Ok(native) => Some(ManuallyDrop::new(native)),
                Err(e) => {
                    speaker_error = Some(e.to_string());
                    None
                }
            }
        }
        Err(_) => None,
    };
    let mut timeline = speaker::Timeline::new();
    let probability_path = request.speaker.output.join("probabilities.f32le");
    let mut posterior = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probability_path)
            .map_err(|e| e.to_string())?,
    );
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut count = 0_u64;
    let mut hash = Sha256::new();
    let mut latest = None;
    let mut last_preview = 0_u64;
    let mut quiet_frames = 0_u64;
    let started = Instant::now();
    loop {
        let mut size = [0; 4];
        input
            .read_exact(&mut size)
            .map_err(|_| "Live input ended without completion")?;
        let size = u32::from_le_bytes(size) as usize;
        if size == 0 {
            let mut expected = [0; 8];
            input.read_exact(&mut expected).map_err(|e| e.to_string())?;
            if u64::from_le_bytes(expected) != count || count == 0 {
                return Err("Incomplete live input frame count".into());
            }
            break;
        }
        if size > 16000 || count + size as u64 > 16000 * 3600 * 3 {
            return Err("Live input exceeds bounded block/duration".into());
        }
        let mut bytes = vec![0; size * 2];
        input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        hash.update(&bytes);
        let samples: Vec<f32> = bytes
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect();
        // Silence is only a publication hint; every sample still reaches both models.
        for sample in &samples {
            if sample.abs() < 0.01 {
                quiet_frames += 1;
            } else {
                quiet_frames = 0;
            }
        }
        count += size as u64;
        asr.push(&samples)?;
        if let Some(result) = asr.drain()? {
            latest = Some(result);
        }
        if let Some(native) = &diar {
            if let Err(error) = native
                .push(&samples)
                .and_then(|_| native.collect(&mut timeline, &mut posterior, count / 160 + 1))
            {
                speaker_error = Some(error.to_string());
                diar = None;
            }
        }
        let boundary = quiet_frames >= 12000
            || latest
                .as_ref()
                .and_then(|r| r["text"].as_str())
                .is_some_and(|text| text.trim_end().ends_with(['.', '!', '?']));
        if count - last_preview >= 60 * 16000 && (boundary || count - last_preview >= 180 * 16000) {
            if let Some(result) = &latest {
                posterior.flush().map_err(|e| e.to_string())?;
                let words = preview_speakers(&result["words"], &probability_path)?;
                emit(
                    &json!({"kind":"preview","meetingId":request.meeting_id,"sessionId":request.session_id,
                    "generation":count,"words":words,"text":result["text"],"provisional":true}),
                )?;
            }
            last_preview = count;
        }
    }
    // The parent closes the prepared WAV before writing the final frame count.
    // Bind the samples actually fed to the models to that finalized input.
    let mut wav = hound::WavReader::open(&request.speaker.audio).map_err(|e| e.to_string())?;
    let spec = wav.spec();
    if spec.sample_rate != 16000
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || u64::from(wav.duration()) != count
    {
        return Err("Finalized live WAV identity differs from fed PCM".into());
    }
    let fed_sha = format!("{:x}", hash.finalize());
    let mut finalized = Sha256::new();
    for sample in wav.samples::<i16>() {
        finalized.update(sample.map_err(|e| e.to_string())?.to_le_bytes());
    }
    if fed_sha != format!("{:x}", finalized.finalize())
        || digest(&request.asr_model)? != model_sha
        || digest(&request.asr_library)? != library_sha
    {
        return Err("Live audio/model/runtime changed".into());
    }
    asr.finish()?;
    let result = asr.drain()?.ok_or("No final ASR result")?;
    if result["final"] != true {
        return Err("ASR did not complete its native stream".into());
    }
    let raw = json!({"schemaVersion":1,"model":manifest["repository"],"modelRevision":manifest["revision"],
        "runtime":nero_core::speech_runtime::current(),
        "processingPolicy":"whole-recording-stateful-live-v1","completionEvidence":"native-stream-finish-cabi-v1",
        "completed":true,"inputAudioFrames":count,"audioSampleRate":16000,
        "audioDurationSeconds":count as f64/16000.0,"nativeDurationSeconds":result["duration"],
        "streamFramesFed":count,"nativeStreamFinished":true,"inputPcmSha256":fed_sha,
        "words":result["words"]});
    save(
        &request
            .speaker
            .output
            .parent()
            .ok_or("Missing live work root")?
            .join("recognition.json"),
        &raw,
    )?;
    emit(
        &json!({"kind":"asr-finished","meetingId":request.meeting_id,"sessionId":request.session_id}),
    )?;
    if let Some(mut native) = diar {
        if let Err(error) = speaker::finish_live(
            &request.speaker,
            count,
            &speaker_library_sha,
            started,
            &native,
            timeline,
            posterior,
        ) {
            speaker_error = Some(error.to_string());
        } else {
            unsafe {
                ManuallyDrop::drop(&mut native);
            }
        }
    }
    if speaker_error.is_none() {
        unsafe {
            ManuallyDrop::drop(&mut asr);
            ManuallyDrop::drop(&mut speaker_api);
        }
    }
    save(
        &request
            .speaker
            .output
            .parent()
            .unwrap()
            .join("live-outcome.json"),
        &json!({"frames":count,
        "asrCompleted":true,"speakerCompleted":speaker_error.is_none(),"speakerError":speaker_error,
        "elapsedSeconds":started.elapsed().as_secs_f64()}),
    )?;
    Ok(if speaker_error.is_none() {
        ExitPolicy::Normal
    } else {
        ExitPolicy::Retained
    })
}
fn preview_speakers(words: &Value, path: &Path) -> Result<Value> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let available = file.metadata().map_err(|e| e.to_string())?.len() / 32;
    let mut words = words.as_array().ok_or("Invalid preview words")?.clone();
    for word in &mut words {
        let start = (word["start"].as_f64().ok_or("Invalid preview time")? / f64::from(0.01_f32))
            .floor() as u64;
        let end = (start + 16).min(available);
        if start >= end {
            continue;
        }
        file.seek(SeekFrom::Start(start * 32))
            .map_err(|e| e.to_string())?;
        let mut scores = [0_f64; 8];
        let mut row = [0_u8; 32];
        for _ in start..end {
            file.read_exact(&mut row).map_err(|e| e.to_string())?;
            for (i, score) in scores.iter_mut().enumerate() {
                *score += f64::from(f32::from_le_bytes(
                    row[i * 4..i * 4 + 4].try_into().unwrap(),
                ));
            }
        }
        let slot = scores
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0
            + 1;
        word["speaker"] = json!(slot);
    }
    Ok(Value::Array(words))
}

pub(super) enum ExitPolicy {
    Normal,
    Retained,
}
impl ExitPolicy {
    pub(super) fn exit(self, code: i32) -> ! {
        if matches!(self, Self::Normal) {
            std::process::exit(code);
        }
        // Files have already been closed/synchronized by entry. Native handles left
        // after an error cannot run Metal's static finalizer before OS reclamation.
        #[cfg(unix)]
        unsafe {
            extern "C" {
                fn _exit(status: i32) -> !;
            }
            _exit(code)
        }
        #[cfg(not(unix))]
        std::process::exit(code)
    }
}
