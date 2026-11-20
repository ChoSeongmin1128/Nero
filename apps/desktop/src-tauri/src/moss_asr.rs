use crate::failure::{Failure, FailureKind};
use crate::process_tree::{supervise, Ended, Unconfirmed, Watch};
use crate::processing::{cancelled, plain_file};
use crate::speech_audio::AnalysisAudio;
use nero_core::{RecognitionChunkState, MOSS_CHUNK_FRAMES, MOSS_CHUNK_POLICY};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub(super) const MANIFEST: &str =
    include_str!("../../../../crates/nero-core/resources/models/moss.json");
const RUNTIME: &str = "mlx-audio-swift-v0.1.3+nero-moss-whole-v2";
const MAX_RAW: u64 = 16 * 1024 * 1024;

fn failure(message: &str) -> Failure {
    Failure::new(FailureKind::Output, message)
}
fn executable(resources: &Path) -> PathBuf {
    resources.join("native/moss/MossHarness")
}
pub(super) fn supported(resources: &Path) -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64")) && executable(resources).is_file()
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, Failure> {
    let mut raw = Vec::new();
    plain_file(path)?
        .take(limit + 1)
        .read_to_end(&mut raw)
        .map_err(|_| failure("MOSS 실행 결과를 읽지 못했습니다."))?;
    if raw.len() as u64 > limit {
        return Err(failure(
            "MOSS 실행 결과가 저장 한도를 넘었습니다. 원본 결과는 보관했습니다.",
        ));
    }
    Ok(raw)
}
fn save(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut output =
        tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut output, value).map_err(|e| e.to_string())?;
    output.as_file().sync_all().map_err(|e| e.to_string())?;
    output.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    fs::File::open(path.parent().unwrap())
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn empty(frames: u64, hash: &str) -> Value {
    let model: Value = serde_json::from_str(MANIFEST).expect("MOSS manifest");
    let chunks: Vec<Value> = (0..frames.div_ceil(MOSS_CHUNK_FRAMES)).map(|index| {
        let start = index * MOSS_CHUNK_FRAMES;
        json!({"index":index,"startFrame":start,"endFrame":(start+MOSS_CHUNK_FRAMES).min(frames),"rawBytes":[],"notProcessed":true})
    }).collect();
    json!({"schemaVersion":1,"model":model["repository"],"modelRevision":model["revision"],"runtime":RUNTIME,
        "processingPolicy":MOSS_CHUNK_POLICY,"inputAudioFrames":frames,"inputAudioSha256":hash,"chunkFrames":MOSS_CHUNK_FRAMES,"chunks":chunks})
}
fn collect(value: &mut Value, output: &Path) -> Result<(), Failure> {
    for chunk in value["chunks"]
        .as_array_mut()
        .ok_or_else(|| failure("MOSS 구간 목록을 확인하지 못했습니다."))?
    {
        let file = output.join(format!("chunk-{}.json", chunk["index"]));
        if file.exists() {
            let raw = json!(read(&file, 2 * 1024 * 1024)?);
            // Resuming reads back the same whole-chunk output the checkpoint holds.
            if chunk["rawBytes"] == raw && chunk["notProcessed"] == json!(false) {
                continue;
            }
            chunk["rawBytes"] = raw;
            chunk["notProcessed"] = json!(false);
            // A chunk recognized again reads the whole analysis file.
            if let Some(chunk) = chunk.as_object_mut() {
                chunk.remove("input");
                chunk.remove("pieces");
            }
        }
    }
    Ok(())
}
fn resume(work: &Path, frames: u64, hash: &str) -> Result<Value, Failure> {
    let checkpoint = work.join("moss-checkpoint.json");
    let mut value: Value = if checkpoint.exists() {
        serde_json::from_slice(&read(&checkpoint, MAX_RAW)?)
            .map_err(|_| failure("MOSS 재시도 기록이 손상됐습니다."))?
    } else {
        let request: Value =
            serde_json::from_slice(&read(&work.join("moss-request.json"), 128 * 1024)?)
                .map_err(|_| failure("MOSS 재시도 입력을 확인하지 못했습니다."))?;
        if request["inputAudioFrames"] != frames || request["inputAudioSha256"] != hash {
            return Err(failure(
                "녹음 원본이 변경되어 이전 MOSS 구간을 재사용하지 않았습니다.",
            ));
        }
        empty(frames, hash)
    };
    if value["inputAudioFrames"] != frames || value["inputAudioSha256"] != hash {
        return Err(failure(
            "녹음 원본이 변경되어 이전 MOSS 구간을 재사용하지 않았습니다.",
        ));
    }
    collect(&mut value, &work.join("moss-chunks"))?;
    nero_core::inspect_moss_chunk_transcript(
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        frames,
        16_000,
    )
    .map_err(|e| e.to_string())?;
    Ok(value)
}

pub(super) fn baseline(work: &Path) -> Result<Option<nero_core::TranscriptVersion>, String> {
    serde_json::from_slice(&read(&work.join("moss-baseline.json"), 4096).map_err(|e| e.message)?)
        .map_err(|_| "MOSS 재시도 기준 기록을 읽지 못했습니다.".into())
}
pub(super) fn save_baseline(
    work: &Path,
    version: &Option<nero_core::TranscriptVersion>,
) -> Result<(), String> {
    save(&work.join("moss-baseline.json"), &json!(version)).map_err(|e| e.message)
}

/// What produced a chunk outside the final job: the pinned model and the worker bytes.
pub(super) fn provenance(resources: &Path) -> Result<Value, Failure> {
    let model: Value = serde_json::from_str(MANIFEST).expect("MOSS manifest");
    let worker = Sha256::digest(read(&executable(resources), 256 * 1024 * 1024)?);
    Ok(json!({"model":model["repository"],"modelRevision":model["revision"],"runtime":RUNTIME,
        "workerSha256":format!("{worker:x}")}))
}

/// A whole chunk recognized from its own WAV while the meeting was recorded.
pub(super) struct Seed {
    pub index: u32,
    pub raw: Vec<u8>,
    /// `{sha256, frames, offsetFrames}` of the WAV the worker read.
    pub input: Value,
    pub provenance: Value,
}

// The PCM16 bytes of a 16 kHz mono WAV and its frame count.
fn pcm_region(path: &Path) -> Result<(u64, u64), Failure> {
    let unreadable = || failure("MOSS 입력 음성을 확인하지 못했습니다.");
    let mut file = plain_file(path)?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).map_err(|_| unreadable())?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(unreadable());
    }
    let mut at = 12_u64;
    loop {
        let mut chunk = [0_u8; 8];
        file.read_exact(&mut chunk).map_err(|_| unreadable())?;
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        at += 8;
        if &chunk[0..4] == b"data" {
            return Ok((at, size / 2));
        }
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Current((size + (size & 1)) as i64))
            .map_err(|_| unreadable())?;
        at += size + (size & 1);
    }
}

// Whether `seed` was read from exactly this audio's samples at its place.
fn seed_matches(audio: &Path, seed: &Seed) -> Result<bool, Failure> {
    let (Some(frames), Some(offset), Some(expected)) = (
        seed.input["frames"].as_u64(),
        seed.input["offsetFrames"].as_u64(),
        seed.input["sha256"].as_str(),
    ) else {
        return Ok(false);
    };
    let (data, total) = pcm_region(audio)?;
    if offset.checked_add(frames).is_none_or(|end| end > total) {
        return Ok(false);
    }
    let Ok(header) = nero_core::moss_chunk_wav_header(frames) else {
        return Ok(false);
    };
    let mut file = plain_file(audio)?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(data + offset * 2))
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    digest.update(header);
    let mut remaining = frames * 2;
    let mut buffer = vec![0_u8; 1 << 16];
    while remaining > 0 {
        let take = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..take])
            .map_err(|_| failure("MOSS 입력 음성을 확인하지 못했습니다."))?;
        digest.update(&buffer[..take]);
        remaining -= take as u64;
    }
    Ok(format!("{:x}", digest.finalize()) == expected)
}

// Places each seed that verifies against `audio` into a chunk that is not complete
// yet. A seed that does not verify, or would not decode as complete, is dropped.
fn apply_seeds(
    value: &mut Value,
    seeds: &[Seed],
    audio: &Path,
    frames: u64,
    current: &Value,
) -> Result<(), Failure> {
    for seed in seeds {
        let report = nero_core::inspect_moss_chunk_transcript(
            &serde_json::to_vec(&*value).map_err(|e| e.to_string())?,
            frames,
            16_000,
        )
        .map_err(|e| e.to_string())?;
        let open = report
            .chunks
            .get(seed.index as usize)
            .is_some_and(|chunk| chunk.state != RecognitionChunkState::Complete);
        if !open || seed.provenance != *current || !seed_matches(audio, seed)? {
            continue;
        }
        let mut trial = value.clone();
        trial["schemaVersion"] = json!(2);
        let chunk = &mut trial["chunks"][seed.index as usize];
        chunk["rawBytes"] = json!(seed.raw);
        chunk["notProcessed"] = json!(false);
        chunk["input"] = seed.input.clone();
        let complete = nero_core::inspect_moss_chunk_transcript(
            &serde_json::to_vec(&trial).map_err(|e| e.to_string())?,
            frames,
            16_000,
        )
        .is_ok_and(|report| {
            report.chunks[seed.index as usize].state == RecognitionChunkState::Complete
        });
        if complete {
            *value = trial;
        }
    }
    Ok(())
}

// Runs the worker on `request` in `work` until it exits, is cancelled, overruns or
// `abort` asks it to stop.
fn run_worker(
    resources: &Path,
    request: &Path,
    work: &Path,
    chunks: usize,
    cancel: &AtomicBool,
    abort: &dyn Fn() -> bool,
) -> Result<(), Failure> {
    let cli = executable(resources);
    let mut command = Command::new(&cli);
    let stdout = work.join("moss.stdout.log");
    let stderr = work.join("moss.stderr.log");
    command
        .arg("--product-chunks")
        .arg(request)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stdout)
                .map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stderr)
                .map_err(|e| e.to_string())?,
        ));
    let mut child = crate::process_tree::spawn(&mut command)?;
    let limit = Duration::from_secs(600 + chunks as u64 * 180);
    let stop = || {
        abort()
            || [&stdout, &stderr]
                .iter()
                .any(|p| fs::metadata(p).is_ok_and(|m| m.len() > 64 * 1024 * 1024))
    };
    // The chunks it wrote before it ended are read either way.
    match supervise(&mut child, cancel, Watch::budget(limit).stop_when(&stop)) {
        Ok(Ended::Unreadable(error)) => Err(error.to_string().into()),
        Ok(_) => Ok(()),
        Err(Unconfirmed) => Err(failure(
            "MOSS 실행기를 종료하지 못했습니다. 앱을 다시 열어 주세요.",
        )),
    }
}

/// Recognizes every whole chunk of a WAV written as `canonical_wav_header` + PCM16,
/// returning each chunk's exact worker output by index. Used while recording.
#[allow(clippy::too_many_arguments)]
pub(super) fn recognize_wav(
    resources: &Path,
    wav: &Path,
    sha256: &str,
    frames: u64,
    work: &Path,
    cancel: &AtomicBool,
    model: &Path,
    abort: &dyn Fn() -> bool,
) -> Result<Vec<(u32, Vec<u8>)>, Failure> {
    cancelled(cancel)?;
    // Whole chunks while recording; a single shorter piece when a chunk is split.
    let chunks = frames.div_ceil(MOSS_CHUNK_FRAMES);
    if frames == 0 || (chunks > 1 && frames % MOSS_CHUNK_FRAMES != 0) {
        return Err(failure("MOSS 구간 길이가 올바르지 않습니다."));
    }
    let output = work.join("moss-chunks");
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let selected: Vec<u64> = (0..chunks).collect();
    let request = json!({"schemaVersion":1,"processingPolicy":MOSS_CHUNK_POLICY,"inputPath":wav,"inputAudioSha256":sha256,
        "inputAudioFrames":frames,"modelDirectory":model,"outputDirectory":output,"selectedChunks":selected});
    save(&work.join("moss-request.json"), &request)?;
    run_worker(
        resources,
        &work.join("moss-request.json"),
        work,
        chunks as usize,
        cancel,
        abort,
    )?;
    cancelled(cancel)?;
    let mut results = Vec::new();
    for index in 0..chunks as u32 {
        let file = output.join(format!("chunk-{index}.json"));
        if file.exists() {
            results.push((index, read(&file, 2 * 1024 * 1024)?));
        }
    }
    Ok(results)
}

// Shortest chunk worth splitting: each piece keeps at least a second of audio.
// Core treats a failed chunk shorter than this as final, so the two agree.
const SPLIT_MINIMUM: u64 = nero_core::MOSS_SPLIT_MINIMUM_FRAMES;

// The canonical WAV of `frames` samples of `audio` from `offset`, and its digest.
fn piece_wav(audio: &Path, offset: u64, frames: u64, path: &Path) -> Result<String, Failure> {
    let (data, total) = pcm_region(audio)?;
    if offset.checked_add(frames).is_none_or(|end| end > total) {
        return Err(failure("MOSS 입력 음성을 확인하지 못했습니다."));
    }
    let header = nero_core::moss_chunk_wav_header(frames).map_err(|e| e.to_string())?;
    let mut bytes = vec![0_u8; (frames * 2) as usize];
    let mut file = plain_file(audio)?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(data + offset * 2))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut bytes)
        .map_err(|_| failure("MOSS 입력 음성을 확인하지 못했습니다."))?;
    let mut digest = Sha256::new();
    digest.update(header);
    digest.update(&bytes);
    let mut output = fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut output, &header)
        .and_then(|()| std::io::Write::write_all(&mut output, &bytes))
        .and_then(|()| output.sync_all())
        .map_err(crate::storage_space::write_error)?;
    Ok(format!("{:x}", digest.finalize()))
}

// Every chunk that failed as a whole is recognized again as two halves, each from its
// own WAV: the model is deterministic, so only a different input can change its
// result. The pieces are recorded whatever their outcome, so a retry never repeats it.
fn split_failed(
    resources: &Path,
    audio: &Path,
    frames: u64,
    work: &Path,
    cancel: &AtomicBool,
    model: &Path,
    value: &mut Value,
) -> Result<(), Failure> {
    let report = nero_core::inspect_moss_chunk_transcript(
        &serde_json::to_vec(&*value).map_err(|e| e.to_string())?,
        frames,
        16_000,
    )
    .map_err(|e| e.to_string())?;
    for chunk in report.chunks {
        let index = chunk.index as usize;
        let length = chunk.end_frame - chunk.start_frame;
        if chunk.state != RecognitionChunkState::Failed
            || value["chunks"][index].get("pieces").is_some()
            || value["chunks"][index].get("input").is_some()
            || length < SPLIT_MINIMUM
        {
            continue;
        }
        cancelled(cancel)?;
        let half = length / 2;
        let attempt = || -> Result<Option<Vec<Value>>, Failure> {
            let mut pieces = Vec::new();
            for (part, (offset, piece)) in [
                (chunk.start_frame, half),
                (chunk.start_frame + half, length - half),
            ]
            .into_iter()
            .enumerate()
            {
                let directory = work.join(format!("split-{index}-{part}"));
                fs::create_dir(&directory).map_err(|e| e.to_string())?;
                let wav = directory.join("piece.wav");
                let sha256 = piece_wav(audio, offset, piece, &wav)?;
                let outputs = recognize_wav(
                    resources, &wav, &sha256, piece, &directory, cancel, model, &|| false,
                );
                let _ = fs::remove_file(&wav);
                // No output means the worker did not finish (time limit, crash), which
                // says nothing about this input; the chunk stays open for a later retry.
                let Some(raw) = outputs?
                    .into_iter()
                    .find(|(index, _)| *index == 0)
                    .map(|(_, raw)| raw)
                else {
                    return Ok(None);
                };
                pieces.push(json!({"rawBytes":raw,"input":{"sha256":sha256,"frames":piece,"offsetFrames":offset}}));
            }
            Ok(Some(pieces))
        };
        // A piece that could not be written or run leaves this chunk open for a later
        // retry; the chunks already recognized are still saved. A stop ends the pass.
        let pieces = match attempt() {
            Ok(Some(pieces)) => pieces,
            Ok(None) => continue,
            Err(error) => {
                cancelled(cancel)?;
                eprintln!("Nero: MOSS 구간 {index}을 나눠 다시 인식하지 못했습니다: {}", error.message);
                continue;
            }
        };
        value["schemaVersion"] = json!(3);
        value["chunks"][index]["pieces"] = json!(pieces);
        // Each chunk's pieces are kept as soon as both ran, so a stop later in this
        // pass does not repeat them.
        save(&work.join("moss-checkpoint.json"), value)?;
    }
    Ok(())
}

pub(super) fn recognize(
    resources: &Path,
    audio: &AnalysisAudio,
    work: &Path,
    cancel: &AtomicBool,
    model: &Path,
    previous: Option<&Path>,
    seeds: &[Seed],
) -> Result<Vec<u8>, Failure> {
    cancelled(cancel)?;
    let evidence = audio.evidence();
    let frames = evidence["inputAudioFrames"]
        .as_u64()
        .ok_or_else(|| failure("MOSS 입력 길이를 확인하지 못했습니다."))?;
    let hash = evidence["analysisAudioSha256"]
        .as_str()
        .ok_or_else(|| failure("MOSS 입력 해시를 확인하지 못했습니다."))?;
    if frames == 0 || frames.div_ceil(MOSS_CHUNK_FRAMES) > 4096 {
        return Err(failure(
            "MOSS 처리 가능 길이를 넘었습니다. 원음은 유지했습니다.",
        ));
    }
    let mut value = match previous {
        Some(path) => resume(path, frames, hash)?,
        None => empty(frames, hash),
    };
    if !seeds.is_empty() {
        apply_seeds(&mut value, seeds, audio.path(), frames, &provenance(resources)?)?;
    }
    let report = nero_core::inspect_moss_chunk_transcript(
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        frames,
        16_000,
    )
    .map_err(|e| e.to_string())?;
    // A chunk already tried in pieces would give the same result again.
    let selected: Vec<u32> = report
        .chunks
        .iter()
        .filter(|chunk| {
            chunk.state != RecognitionChunkState::Complete
                && value["chunks"][chunk.index as usize].get("pieces").is_none()
        })
        .map(|chunk| chunk.index)
        .collect();
    let output = work.join("moss-chunks");
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let request = json!({"schemaVersion":1,"processingPolicy":MOSS_CHUNK_POLICY,"inputPath":audio.path(),"inputAudioSha256":hash,
        "inputAudioFrames":frames,"modelDirectory":model,"outputDirectory":output,"selectedChunks":selected});
    save(&work.join("moss-request.json"), &request)?;
    save(&work.join("moss-checkpoint.json"), &value)?;
    if !selected.is_empty() {
        let cli = executable(resources);
        let cli_hash = Sha256::digest(read(&cli, 256 * 1024 * 1024)?);
        // A stopped/crashed worker can leave independently completed chunks. Core
        // validates them and exposes all missing ranges instead of claiming success.
        run_worker(
            resources,
            &work.join("moss-request.json"),
            work,
            selected.len(),
            cancel,
            &|| false,
        )?;
        collect(&mut value, &output)?;
        save(&work.join("moss-checkpoint.json"), &value)?;
        cancelled(cancel)?;
        split_failed(resources, audio.path(), frames, work, cancel, model, &mut value)?;
        save(&work.join("moss-checkpoint.json"), &value)?;
        cancelled(cancel)?;
        if Sha256::digest(read(&cli, 256 * 1024 * 1024)?) != cli_hash {
            return Err(failure("MOSS 실행기가 처리 중 변경됐습니다."));
        }
    }
    let raw = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    if raw.len() as u64 > MAX_RAW {
        return Err(failure(
            "MOSS 결과 저장 한도를 넘었습니다. 구간 원본은 보관했습니다.",
        ));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_is_bound_to_audio_and_preserves_missing_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let frames = MOSS_CHUNK_FRAMES + 16000;
        let raw = empty(frames, &"a".repeat(64));
        save(&dir.path().join("moss-checkpoint.json"), &raw).unwrap();
        assert_eq!(resume(dir.path(), frames, &"a".repeat(64)).unwrap(), raw);
        assert!(resume(dir.path(), frames, &"b".repeat(64)).is_err());
        assert!(resume(dir.path(), frames + 1, &"a".repeat(64)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn a_retry_executes_only_failed_chunks_and_keeps_exact_completed_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let cli = root.join("native/moss/MossHarness");
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&cli,r#"#!/usr/bin/python3
import json, sys
from pathlib import Path
request=json.loads(Path(sys.argv[2]).read_text())
flags=Path(request['modelDirectory'])
(flags/'executed.json').write_text(json.dumps(request['selectedChunks']))
for index in request['selectedChunks']:
 start=index*960000; end=min(start+960000,request['inputAudioFrames']); frames=end-start
 lengths=[(min(480000,frames-i)+1279)//1280 for i in range(0,frames,480000)]
 failed=index==1 and (flags/'fail-second').exists()
 value={'index':index,'startFrame':start,'endFrame':end,'inputAudioSha256':request['inputAudioSha256'],
  'rawText':'[0]합성 문장[0.5]','segments':[{'start':0,'end':0.5,'text':'합성 문장'}],'parsedCompletely':not failed,
  'generationMetadata':[{'metadataSchemaVersion':2,'eosPolicy':'model-generation-config+tokenizer-v1','samplingPolicy':'greedy-argmax-v1',
   'terminationReason':'eos','terminalTokenId':151645,'configuredEosTokenIds':[151645],'tokenizerEosTokenId':151645,
   'inputAudioSamples':frames,'featureChunkCount':len(lengths),'featureTokenLengths':lengths,'audioTokenCount':sum(lengths),'audioEmbeddingCount':sum(lengths),
   'promptTokens':1000,'contextSize':32768,'maxTokens':8192,'effectiveMaxTokens':8192,'generationTokens':12}]}
 (Path(request['outputDirectory'])/('chunk-%d.json'%index)).write_text(json.dumps(value))
"#).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        let model = root.join("fixture-model");
        fs::create_dir(&model).unwrap();
        fs::write(model.join("fail-second"), b"fixture").unwrap();
        let input = root.join("input.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..(MOSS_CHUNK_FRAMES + 16000) {
            wav.write_sample(0i16).unwrap();
        }
        wav.finalize().unwrap();
        let cancel = AtomicBool::new(false);
        let audio = AnalysisAudio::from_live(&input, &input, &cancel).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let initial = recognize(&root, &audio, &first, &cancel, &model, None, &[]).unwrap();
        let initial_report =
            nero_core::inspect_moss_chunk_transcript(&initial, MOSS_CHUNK_FRAMES + 16000, 16000)
                .unwrap();
        assert!(!initial_report.text_complete());
        assert_eq!(
            initial_report.chunks[0].state,
            RecognitionChunkState::Complete
        );
        fs::remove_file(model.join("fail-second")).unwrap();
        let retried =
            recognize(&root, &audio, &second, &cancel, &model, Some(&first), &[]).unwrap();
        assert!(nero_core::inspect_moss_chunk_transcript(
            &retried,
            MOSS_CHUNK_FRAMES + 16000,
            16000
        )
        .unwrap()
        .text_complete());
        assert_eq!(
            fs::read_to_string(model.join("executed.json")).unwrap(),
            "[1]"
        );
        let initial: Value = serde_json::from_slice(&initial).unwrap();
        let retried: Value = serde_json::from_slice(&retried).unwrap();
        assert_eq!(
            initial["chunks"][0]["rawBytes"],
            retried["chunks"][0]["rawBytes"]
        );
        assert!(!second.join("moss-chunks/chunk-0.json").exists());
    }

    #[cfg(unix)]
    fn fake_worker(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let cli = root.join("native/moss/MossHarness");
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&cli,r#"#!/usr/bin/python3
import json, sys
from pathlib import Path
request=json.loads(Path(sys.argv[2]).read_text())
flags=Path(request['modelDirectory'])
with open(flags/'executed.log','a') as log: log.write(json.dumps([request['inputAudioFrames']]+request['selectedChunks'])+'\n')
piece=request['inputAudioFrames']<960000
for index in request['selectedChunks']:
 start=index*960000; end=min(start+960000,request['inputAudioFrames']); frames=end-start
 lengths=[(min(480000,frames-i)+1279)//1280 for i in range(0,frames,480000)]
 failed=(flags/('fail-pieces' if piece else 'fail-whole-%d'%index)).exists()
 value={'index':index,'startFrame':start,'endFrame':end,'inputAudioSha256':request['inputAudioSha256'],
  'rawText':'[0]합성 문장[0.5]','segments':[{'start':0,'end':0.5,'text':'합성 문장'}],'parsedCompletely':not failed,
  'generationMetadata':[{'metadataSchemaVersion':2,'eosPolicy':'model-generation-config+tokenizer-v1','samplingPolicy':'greedy-argmax-v1',
   'terminationReason':'eos','terminalTokenId':151645,'configuredEosTokenIds':[151645],'tokenizerEosTokenId':151645,
   'inputAudioSamples':frames,'featureChunkCount':len(lengths),'featureTokenLengths':lengths,'audioTokenCount':sum(lengths),'audioEmbeddingCount':sum(lengths),
   'promptTokens':1000,'contextSize':32768,'maxTokens':8192,'effectiveMaxTokens':8192,'generationTokens':12}]}
 (Path(request['outputDirectory'])/('chunk-%d.json'%index)).write_text(json.dumps(value))
"#).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        let model = root.join("fixture-model");
        fs::create_dir(&model).unwrap();
        model
    }

    #[cfg(unix)]
    #[test]
    fn chunks_recognized_while_recording_fill_their_places_and_only_the_rest_runs_after() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let model = fake_worker(&root);
        let frames = 2 * MOSS_CHUNK_FRAMES + 16000;
        let samples: Vec<i16> = (0..frames).map(|i| ((i * 7) % 2001) as i16 - 1000).collect();
        let input = root.join("audio.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in &samples {
            wav.write_sample(*sample).unwrap();
        }
        wav.finalize().unwrap();
        let cancel = AtomicBool::new(false);
        let current = provenance(&root).unwrap();
        // What the recording-time processor produces for chunk `index`.
        let live = |index: u64, provenance: Value| {
            let directory = tempfile::tempdir_in(&root).unwrap().keep();
            let mut writer = crate::live_moss::ChunkWriter::new(directory.clone());
            let start = (index * MOSS_CHUNK_FRAMES) as usize;
            let mut files = Vec::new();
            // The writer numbers its own chunks; feed it from the recording start.
            for chunk in samples[..start + MOSS_CHUNK_FRAMES as usize].chunks(4096) {
                files.extend(writer.push(chunk, || true).unwrap());
            }
            let file = files.pop().unwrap();
            assert_eq!(file.index, index);
            let work = directory.join("run");
            fs::create_dir(&work).unwrap();
            let outputs = recognize_wav(
                &root, &file.path, &file.sha256, MOSS_CHUNK_FRAMES, &work, &cancel, &model, &|| false,
            )
            .unwrap();
            Seed {
                index: index as u32,
                raw: outputs[0].1.clone(),
                input: json!({"sha256":file.sha256,"frames":MOSS_CHUNK_FRAMES,"offsetFrames":index * MOSS_CHUNK_FRAMES}),
                provenance,
            }
        };
        let seeds = vec![live(0, current.clone()), live(1, current.clone())];
        fs::remove_file(model.join("executed.log")).unwrap();
        let audio = AnalysisAudio::from_live(&input, &input, &cancel).unwrap();
        let work = root.join("final");
        fs::create_dir(&work).unwrap();
        let raw = recognize(&root, &audio, &work, &cancel, &model, None, &seeds).unwrap();
        assert_eq!(
            fs::read_to_string(model.join("executed.log")).unwrap(),
            format!("[{frames}, 2]\n")
        );
        let report = nero_core::inspect_moss_chunk_transcript(&raw, frames, 16000).unwrap();
        assert!(report.text_complete());
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(value["chunks"][1]["input"]["offsetFrames"], MOSS_CHUNK_FRAMES);
        assert!(value["chunks"][2].get("input").is_none());
        assert_eq!(value["chunks"][0]["rawBytes"], json!(seeds[0].raw));

        // A retry from this checkpoint keeps the recording-time chunks as they are.
        let retry = root.join("retry");
        fs::create_dir(&retry).unwrap();
        fs::remove_file(model.join("executed.log")).unwrap();
        let again = recognize(&root, &audio, &retry, &cancel, &model, Some(&work), &[]).unwrap();
        assert!(!model.join("executed.log").exists());
        assert_eq!(serde_json::from_slice::<Value>(&again).unwrap()["chunks"], value["chunks"]);

        // A chunk from another worker, or whose samples differ, is recognized again.
        let mut other = current.clone();
        other["workerSha256"] = json!("0".repeat(64));
        let mut tampered = live(1, current.clone());
        tampered.input["sha256"] = json!("1".repeat(64));
        let rejected = vec![live(0, other), tampered];
        fs::remove_file(model.join("executed.log")).unwrap();
        let work = root.join("rejected");
        fs::create_dir(&work).unwrap();
        let raw = recognize(&root, &audio, &work, &cancel, &model, None, &rejected).unwrap();
        assert_eq!(
            fs::read_to_string(model.join("executed.log")).unwrap(),
            format!("[{frames}, 0, 1, 2]\n")
        );
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert!(value["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|chunk| chunk.get("input").is_none()));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_chunk_is_recognized_again_in_two_halves_and_never_repeated_after() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let model = fake_worker(&root);
        fs::write(model.join("fail-whole-1"), b"fixture").unwrap();
        let frames = 2 * MOSS_CHUNK_FRAMES + 16000;
        let input = root.join("audio.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..frames {
            wav.write_sample(((i * 13) % 997) as i16).unwrap();
        }
        wav.finalize().unwrap();
        let cancel = AtomicBool::new(false);
        let audio = AnalysisAudio::from_live(&input, &input, &cancel).unwrap();
        let run = |name: &str, previous: Option<&Path>| {
            let work = root.join(name);
            fs::create_dir(&work).unwrap();
            let _ = fs::remove_file(model.join("executed.log"));
            let raw = recognize(&root, &audio, &work, &cancel, &model, previous, &[]).unwrap();
            let log = fs::read_to_string(model.join("executed.log")).unwrap_or_default();
            (work, raw, log)
        };
        let half = MOSS_CHUNK_FRAMES / 2;
        let (_, raw, log) = run("split", None);
        assert_eq!(log, format!("[{frames}, 0, 1, 2]\n[{half}, 0]\n[{half}, 0]\n"));
        let report = nero_core::inspect_moss_chunk_transcript(&raw, frames, 16000).unwrap();
        assert!(report.text_complete());
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["schemaVersion"], 3);
        assert_eq!(value["chunks"][1]["pieces"][1]["input"]["offsetFrames"], MOSS_CHUNK_FRAMES + half);
        assert!(!value["chunks"][1]["rawBytes"].as_array().unwrap().is_empty());

        // A piece that cannot be written leaves its chunk open for a retry, and the
        // chunks recognized around it are still returned.
        let blocked = root.join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::create_dir(blocked.join("split-1-0")).unwrap();
        let raw = recognize(&root, &audio, &blocked, &cancel, &model, None, &[]).unwrap();
        let report = nero_core::inspect_moss_chunk_transcript(&raw, frames, 16000).unwrap();
        assert_eq!(report.chunks[0].state, RecognitionChunkState::Complete);
        assert_eq!(report.chunks[1].state, RecognitionChunkState::Failed);
        assert_eq!(report.chunks[2].state, RecognitionChunkState::Complete);
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert!(value["chunks"][1].get("pieces").is_none());

        fs::write(model.join("fail-pieces"), b"fixture").unwrap();
        let (failed, raw, _) = run("unrecognized", None);
        let report = nero_core::inspect_moss_chunk_transcript(&raw, frames, 16000).unwrap();
        assert_eq!(report.chunks[1].state, RecognitionChunkState::Failed);
        assert_eq!(report.chunks[1].issue.as_deref(), Some("splitFailed"));
        // The same input gives the same result: a retry runs nothing again.
        let (_, again, log) = run("retry", Some(&failed));
        assert_eq!(log, "");
        assert_eq!(again, raw);
    }

    #[test]
    fn a_chunk_recognized_again_no_longer_claims_its_separate_wav() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = empty(MOSS_CHUNK_FRAMES, &"a".repeat(64));
        value["chunks"][0]["input"] = json!({"sha256":"b".repeat(64)});
        fs::write(dir.path().join("chunk-0.json"), b"{}").unwrap();
        collect(&mut value, dir.path()).unwrap();
        assert!(value["chunks"][0].get("input").is_none());
        assert_eq!(value["chunks"][0]["notProcessed"], false);
    }
}
