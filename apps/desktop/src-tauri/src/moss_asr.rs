use crate::failure::{Failure, FailureKind};
use crate::processing::{cancelled, plain_file};
use crate::speech_audio::AnalysisAudio;
use nero_core::{RecognitionChunkState, MOSS_CHUNK_FRAMES, MOSS_CHUNK_POLICY};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
            chunk["rawBytes"] = json!(read(&file, 2 * 1024 * 1024)?);
            chunk["notProcessed"] = json!(false);
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

pub(super) fn recognize(
    resources: &Path,
    audio: &AnalysisAudio,
    work: &Path,
    cancel: &AtomicBool,
    model: &Path,
    previous: Option<&Path>,
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
    let report = nero_core::inspect_moss_chunk_transcript(
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        frames,
        16_000,
    )
    .map_err(|e| e.to_string())?;
    let selected: Vec<u32> = report
        .chunks
        .iter()
        .filter(|chunk| chunk.state != RecognitionChunkState::Complete)
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
        let mut command = Command::new(&cli);
        let stdout = work.join("moss.stdout.log");
        let stderr = work.join("moss.stderr.log");
        command
            .arg("--product-chunks")
            .arg(work.join("moss-request.json"))
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
        let began = Instant::now();
        let limit = Duration::from_secs(600 + selected.len() as u64 * 180);
        let mut stopped = false;
        loop {
            if cancel.load(Ordering::Acquire)
                || began.elapsed() > limit
                || [&stdout, &stderr]
                    .iter()
                    .any(|p| fs::metadata(p).is_ok_and(|m| m.len() > 64 * 1024 * 1024))
            {
                child.kill().map_err(|_| {
                    failure("MOSS 실행기를 종료하지 못했습니다. 앱을 다시 열어 주세요.")
                })?;
                child.wait().map_err(|e| e.to_string())?;
                stopped = true;
                break;
            }
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(child);
        collect(&mut value, &output)?;
        save(&work.join("moss-checkpoint.json"), &value)?;
        cancelled(cancel)?;
        if Sha256::digest(read(&cli, 256 * 1024 * 1024)?) != cli_hash {
            return Err(failure("MOSS 실행기가 처리 중 변경됐습니다."));
        }
        // A stopped/crashed worker can leave independently completed chunks. Core
        // validates them and exposes all missing ranges instead of claiming success.
        let _ = stopped;
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
        let initial = recognize(&root, &audio, &first, &cancel, &model, None).unwrap();
        let initial_report =
            nero_core::inspect_moss_chunk_transcript(&initial, MOSS_CHUNK_FRAMES + 16000, 16000)
                .unwrap();
        assert!(!initial_report.text_complete());
        assert_eq!(
            initial_report.chunks[0].state,
            RecognitionChunkState::Complete
        );
        fs::remove_file(model.join("fail-second")).unwrap();
        let retried = recognize(&root, &audio, &second, &cancel, &model, Some(&first)).unwrap();
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
}
