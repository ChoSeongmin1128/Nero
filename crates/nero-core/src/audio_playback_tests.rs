use super::*;
use crate::audio_normalize::{Normalizer, SAMPLE_RATE};
use crate::engine::Engine;
use sha2::{Digest, Sha256};
use std::io::Cursor;

fn fixture(
    rate: u32,
    channels: u16,
) -> (tempfile::TempDir, Store, StoredMeeting, Manifest, Vec<u8>) {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::open(temporary.path().canonicalize().unwrap().join("library")).unwrap();
    let mut stored = store.create("회의".into(), "보존할 메모".into()).unwrap();
    let mut manifest = Manifest::new(
        &stored.meeting.id,
        "synthetic",
        "시험 입력",
        rate,
        channels,
        Encoding::F32,
    );
    manifest.phase = CapturePhase::Completed;
    let mut raw = store.prepare_capture(&manifest).unwrap();
    let mut original = Vec::new();
    for frame in 0..4099 {
        for channel in 0..channels {
            let sample =
                ((f64::from(frame) * 0.021 + f64::from(channel) * 0.31).sin() * 0.5) as f32;
            original.extend_from_slice(&sample.to_le_bytes());
        }
    }
    raw.write_all(&original).unwrap();
    raw.sync_all().unwrap();
    drop(raw);
    stored.capture_id = Some(manifest.session_id.clone());
    stored.meeting.status = MeetingStatus::Completed;
    store.save(&stored).unwrap();
    (temporary, store, stored, manifest, original)
}

fn finalize_speech(
    store: &Store,
    stored: &mut StoredMeeting,
    manifest: &mut Manifest,
) -> AudioArtifact {
    let playback =
        Playback::with_native_limit(4099, manifest.sample_rate, manifest.channels, 0).unwrap();
    let artifact = finalize_audio_with_playback(&store.root, manifest, Some(playback))
        .unwrap()
        .unwrap();
    stored.meeting.audio = Some(artifact.clone());
    store.save(stored).unwrap();
    artifact
}

#[test]
fn ordinary_capture_keeps_native_float_playback_bytes_and_legacy_geometry() {
    let (_temporary, store, mut stored, mut manifest, original) = fixture(96_000, 2);
    let artifact = finalize_audio(&store.root, &mut manifest).unwrap().unwrap();
    assert!(artifact.playback.is_none());
    assert_eq!(artifact.frames, 4099);
    assert_eq!(artifact.sample_rate, 96_000);
    assert_eq!(artifact.channels, 2);
    let mut expected = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut expected,
            Playback::from_artifact(&artifact).unwrap().spec,
        )
        .unwrap();
        for sample in original.chunks_exact(4) {
            writer
                .write_sample(f32::from_le_bytes(sample.try_into().unwrap()))
                .unwrap();
        }
        writer.finalize().unwrap();
    }
    let path = store.root.join(&artifact.relative_path);
    assert_eq!(fs::read(&path).unwrap(), expected.into_inner());
    assert_eq!(
        fs::read(store.root.join(manifest.relative_dir()).join("audio.pcm")).unwrap(),
        original
    );
    stored.meeting.audio = Some(artifact);
    store.save(&stored).unwrap();
    assert_eq!(store.audio_path(&stored.meeting).unwrap(), path);
}

#[test]
fn bounded_playback_matches_live_pcm_hash_frames_and_native_coordinates_after_reopen() {
    for (rate, channels) in [(96_000, 2), (192_000, 4), (44_100, 3)] {
        let (_temporary, store, mut stored, mut manifest, original) = fixture(rate, channels);
        let artifact = finalize_speech(&store, &mut stored, &mut manifest);
        let playback = Playback::from_artifact(&artifact).unwrap();
        let expected_frames = crate::audio_normalize::output_frames(4099, rate).unwrap();
        assert_eq!(artifact.frames, 4099);
        assert_eq!(artifact.sample_rate, rate);
        assert_eq!(artifact.channels, channels);
        assert_eq!(playback.frames, expected_frames);
        assert_eq!(playback.spec.sample_rate, SAMPLE_RATE);
        let path = store.audio_path(&stored.meeting).unwrap();
        let actual = fs::read(&path).unwrap();
        assert_eq!(&actual[..4], b"RIFF");
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(u64::from(reader.duration()), expected_frames);
        assert_eq!(reader.spec(), playback.spec);
        let mut expected = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut expected, playback.spec).unwrap();
            let mut input = Cursor::new(&original);
            let mut normalizer = Normalizer::new(rate).unwrap();
            let mut remaining = 4099;
            while remaining != 0 {
                let count = remaining.min(137);
                let mono = crate::audio_playback::read_native_mono(
                    &mut input,
                    Encoding::F32,
                    channels,
                    count,
                )
                .unwrap();
                for sample in normalizer.push(&mono, false).unwrap() {
                    writer.write_sample(sample).unwrap();
                }
                remaining -= count;
            }
            for sample in normalizer.push(&[], true).unwrap() {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        let expected = expected.into_inner();
        assert_eq!(Sha256::digest(&actual), Sha256::digest(&expected));
        assert_eq!(actual, expected);
        assert_eq!(manifest.saved_frames, 4099);
        assert_eq!(
            fs::read(store.root.join(manifest.relative_dir()).join("audio.pcm")).unwrap(),
            original
        );
        let root = store.root.clone();
        drop(store);
        let engine = Engine::open(root).unwrap();
        assert_eq!(
            engine.meetings[&stored.meeting.id].meeting.audio.as_ref(),
            Some(&artifact)
        );
        assert_eq!(engine.audio_path(&stored.meeting.id).unwrap(), path);
        assert_eq!(fs::read(path).unwrap(), actual);
    }
}

#[test]
fn interrupted_playback_repair_reuses_saved_policy_without_changing_raw_pcm() {
    let (_temporary, store, mut stored, mut manifest, original) = fixture(96_000, 4);
    let artifact = finalize_speech(&store, &mut stored, &mut manifest);
    let path = store.audio_path(&stored.meeting).unwrap();
    let expected = fs::read(&path).unwrap();
    fs::write(&path, b"broken").unwrap();
    manifest.phase = CapturePhase::Interrupted;
    write_manifest(&store.root, &manifest).unwrap();
    stored.meeting.status = MeetingStatus::Interrupted;
    store.save(&stored).unwrap();
    let root = store.root.clone();
    drop(store);
    let engine = Engine::open(root.clone()).unwrap();
    assert_eq!(
        engine.meetings[&stored.meeting.id].meeting.audio.as_ref(),
        Some(&artifact)
    );
    assert_eq!(
        fs::read(engine.audio_path(&stored.meeting.id).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        fs::read(root.join(manifest.relative_dir()).join("audio.pcm")).unwrap(),
        original
    );
    let repaired: Manifest =
        read_json(&root.join(manifest.relative_dir()).join("manifest.json")).unwrap();
    assert_eq!(repaired.saved_frames, 4099);
    assert_eq!(repaired.phase, CapturePhase::Interrupted);
    assert_eq!(repaired.sample_rate, 96_000);
    assert_eq!(repaired.channels, 4);
    assert_eq!(
        engine.meetings[&stored.meeting.id].meeting.notes,
        "보존할 메모"
    );
}

#[test]
fn synthetic_playback_asr_speaker_generation_and_reader_preserve_coordinates_and_edits() {
    let (_temporary, store, mut stored, mut manifest, original) = fixture(96_000, 2);
    let artifact = finalize_speech(&store, &mut stored, &mut manifest);
    let playback = store.audio_path(&stored.meeting).unwrap();
    let playback_bytes = fs::read(&playback).unwrap();
    let playback_sha = format!("{:x}", Sha256::digest(&playback_bytes));
    let original_sha = format!("{:x}", Sha256::digest(&original));
    let mut pcm_sha = Sha256::new();
    let mut input = hound::WavReader::open(&playback).unwrap();
    assert_eq!(input.duration(), 683);
    for sample in input.samples::<i16>() {
        pcm_sha.update(sample.unwrap().to_le_bytes());
    }
    let pcm_sha = format!("{:x}", pcm_sha.finalize());
    let prepared_duration = 683.0 / f64::from(SAMPLE_RATE);
    let mut recognition: serde_json::Value =
        serde_json::from_slice(&crate::nemotron::tests::raw()).unwrap();
    recognition["processingPolicy"] = serde_json::json!("whole-recording-stateful-live-v1");
    recognition["completionEvidence"] = serde_json::json!("native-stream-finish-cabi-v1");
    recognition["inputAudioFrames"] = serde_json::json!(683);
    recognition["audioDurationSeconds"] = serde_json::json!(prepared_duration);
    recognition["nativeDurationSeconds"] = serde_json::json!(prepared_duration);
    recognition["streamFramesFed"] = serde_json::json!(683);
    recognition["nativeStreamFinished"] = serde_json::json!(true);
    recognition["inputPcmSha256"] = serde_json::json!(pcm_sha);
    recognition["words"] = serde_json::json!([
        {"word":"합성","start":0.0,"end":0.01},
        {"word":"문장.","start":0.01,"end":0.02},
        {"word":"끝.","start":0.02,"end":0.04}
    ]);
    let raw = serde_json::to_vec(&recognition).unwrap();
    let asr_sha = format!("{:x}", Sha256::digest(&raw));
    let root = store.root.clone();
    let id = stored.meeting.id.clone();
    drop(store);
    let mut engine = Engine::open(root.clone()).unwrap();
    let baseline = engine
        .commit_nemotron_transcript(&id, None, raw.clone())
        .unwrap();
    assert_eq!(baseline.segments.len(), 2);
    assert_eq!(baseline.source_session_id, manifest.session_id);
    assert_eq!(baseline.source.as_ref().unwrap().raw_sha256, asr_sha);
    assert_eq!(baseline.version.revision, 0);
    let baseline_id = baseline.version.id.clone();
    let edited = engine
        .edit_transcript(
            &id,
            baseline.version,
            vec!["사용자 수정.".into(), "끝.".into()],
        )
        .unwrap();
    assert_eq!(edited.version.revision, 1);
    let parent_version = edited.version.clone();
    let parent_sha = crate::summary::transcript_digest(&edited).unwrap();
    let mut draft = crate::nemotron::tests::posterior_draft(edited.clone());
    let mut probabilities = Vec::new();
    for _ in 0..5 {
        for slot in 0..8 {
            probabilities
                .extend_from_slice(&(if slot == 0 { 0.9_f32 } else { 0.0_f32 }).to_le_bytes());
        }
    }
    draft.posterior.as_mut().unwrap().frames = 5;
    draft.posterior.as_mut().unwrap().probabilities = probabilities.clone();
    draft.raw = serde_json::to_vec(&serde_json::json!({"file":"synthetic.wav","segments":[
        {"start":0.0,"end":prepared_duration,"speaker":1}
    ]}))
    .unwrap();
    let activity_sha = format!("{:x}", Sha256::digest(&draft.raw));
    let posterior_sha = format!("{:x}", Sha256::digest(&probabilities));
    let assigned = engine.apply_diarization(&id, draft).unwrap();
    assert_ne!(assigned.version.id, baseline_id);
    assert_eq!(assigned.version.revision, 1);
    assert_eq!(assigned.segments[0].text, "사용자 수정.");
    assert_eq!(assigned.segments[0].original_text, "합성 문장.");
    assert!(assigned
        .segments
        .iter()
        .any(|segment| segment.speaker_id.is_some()));
    let derived_version = assigned.version.clone();
    let derived_source = assigned.source.clone();
    let generation = root
        .join("meetings")
        .join(&id)
        .join("transcripts")
        .join(&assigned.version.id);
    let derived_raw: serde_json::Value = read_json(&generation.join("raw.json")).unwrap();
    let raw_sha = format!(
        "{:x}",
        Sha256::digest(fs::read(generation.join("raw.json")).unwrap())
    );
    assert_eq!(assigned.source.as_ref().unwrap().raw_sha256, raw_sha);
    assert_ne!(raw_sha, asr_sha);
    assert_eq!(derived_raw["canonicalAsrSha256"], asr_sha);
    assert_eq!(
        derived_raw["canonicalAsrRaw"],
        String::from_utf8(raw.clone()).unwrap()
    );
    assert_eq!(derived_raw["posterior"]["inputFrames"], 4099);
    assert_eq!(derived_raw["posterior"]["inputSampleRate"], 96_000);
    assert_eq!(derived_raw["posterior"]["preparedFrames"], 683);
    assert_eq!(derived_raw["posterior"]["sha256"], posterior_sha);
    let analysis = assigned.speaker_analysis.as_ref().unwrap();
    assert_eq!(analysis.raw_sha256, activity_sha);
    let evidence = generation.join("speaker-analysis").join(&analysis.run_id);
    assert_eq!(
        fs::read(evidence.join("posterior.f32le")).unwrap(),
        probabilities
    );
    let receipt: serde_json::Value = read_json(&evidence.join("run.json")).unwrap();
    assert_eq!(
        receipt["beforeVersion"],
        serde_json::to_value(&parent_version).unwrap()
    );
    assert_eq!(
        receipt["afterVersion"],
        serde_json::to_value(&derived_version).unwrap()
    );
    assert_eq!(receipt["provenance"]["sourceSha256"], parent_sha);
    assert_eq!(
        receipt["provenance"]["activityNormalization"]["inputFrames"],
        4099
    );
    assert_eq!(
        receipt["provenance"]["activityNormalization"]["inputSampleRate"],
        96_000
    );
    assert_eq!(
        receipt["provenance"]["activityNormalization"]["preparedFrames"],
        683
    );
    let mut names = assigned.speakers.clone();
    names[0].name = "합성 참석자".into();
    let assignments = assigned
        .segments
        .iter()
        .map(|segment| segment.speaker_id.clone())
        .collect();
    let named = engine
        .edit_speakers(&id, assigned.version, names, assignments)
        .unwrap();
    assert_eq!(named.version.revision, 2);
    assert_eq!(named.source, derived_source);
    assert_eq!(named.source_session_id, manifest.session_id);
    assert_eq!(named.segments[0].start, 0.0);
    assert_eq!(named.segments[0].end, 0.02);
    assert_eq!(named.segments.last().unwrap().end, 0.04);
    drop(engine);
    let reopened = Engine::open(root.clone()).unwrap();
    assert_eq!(
        reopened.meetings[&id].meeting.audio.as_ref(),
        Some(&artifact)
    );
    let current = reopened.transcript(&id).unwrap().unwrap();
    assert_eq!(current.version, named.version);
    assert_eq!(current.source, derived_source);
    assert_eq!(current.speakers[0].name, "합성 참석자");
    assert_eq!(current.segments[0].text, "사용자 수정.");
    let history = reopened.transcript_history(&id).unwrap();
    assert_eq!(history.unavailable, 0);
    assert_eq!(history.entries.len(), 2);
    assert!(history
        .entries
        .iter()
        .any(|entry| entry.version == parent_version));
    assert!(history
        .entries
        .iter()
        .any(|entry| entry.version == named.version && entry.current));
    let reader = crate::reader::LibraryReader::open(&root).unwrap();
    let query = reader
        .query("transcript_get", serde_json::json!({"meetingId":id}))
        .unwrap();
    assert_eq!(
        query["version"],
        serde_json::to_value(&named.version).unwrap()
    );
    assert_eq!(query["segments"][0]["text"], "사용자 수정.");
    assert_eq!(query["segments"][0]["start"], 0.0);
    assert_eq!(query["segments"][0]["end"], 0.02);
    assert_eq!(query["speakers"][0]["name"], "합성 참석자");
    let meeting = reader
        .query("meeting_get", serde_json::json!({"meetingId":id}))
        .unwrap();
    assert_eq!(meeting["meeting"]["transcriptVersion"], query["version"]);
    assert_eq!(
        fs::read(
            root.join("meetings")
                .join(&id)
                .join("transcripts")
                .join(baseline_id)
                .join("raw.json")
        )
        .unwrap(),
        raw
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&playback).unwrap())),
        playback_sha
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(root.join(manifest.relative_dir()).join("audio.pcm")).unwrap())
        ),
        original_sha
    );
    let persisted: Manifest =
        read_json(&root.join(manifest.relative_dir()).join("manifest.json")).unwrap();
    assert_eq!(persisted.saved_frames, 4099);
    assert_eq!(persisted.sample_rate, 96_000);
    assert_eq!(persisted.channels, 2);
}

#[cfg(unix)]
#[test]
fn sparse_three_hour_capture_accepts_complete_standard_playback_without_reading_pcm_body() {
    use std::io::SeekFrom;
    use std::os::unix::fs::MetadataExt;

    let temporary = tempfile::tempdir().unwrap();
    let store = Store::open(temporary.path().canonicalize().unwrap().join("library")).unwrap();
    let mut stored = store.create("회의".into(), "보존할 메모".into()).unwrap();
    let mut manifest = Manifest::new(
        &stored.meeting.id,
        "synthetic",
        "시험 입력",
        96_000,
        2,
        Encoding::F32,
    );
    manifest.phase = CapturePhase::Completed;
    let frames = 96_000_u64 * 3 * 60 * 60;
    let raw_bytes = frames * manifest.frame_bytes() as u64;
    let sentinel = [0, 0, 128, 62, 0, 0, 0, 191];
    let mut raw = store.prepare_capture(&manifest).unwrap();
    raw.write_all(&sentinel).unwrap();
    raw.set_len(raw_bytes).unwrap();
    raw.seek(SeekFrom::Start(raw_bytes - 8)).unwrap();
    raw.write_all(&sentinel).unwrap();
    raw.sync_all().unwrap();
    drop(raw);
    let directory = store.root.join(manifest.relative_dir());
    let raw_path = directory.join("audio.pcm");
    let before = fs::metadata(&raw_path).unwrap();
    assert!(before.blocks() * 512 < 1024 * 1024);

    let expected = Playback::for_capture(frames, 96_000, 2).unwrap();
    let data_bytes = u32::try_from(expected.frames * 2).unwrap();
    let mut wav = create_new(&directory.join("audio.wav")).unwrap();
    for bytes in [
        &b"RIFF"[..],
        &(data_bytes + 36).to_le_bytes(),
        &b"WAVEfmt "[..],
        &16_u32.to_le_bytes(),
        &1_u16.to_le_bytes(),
        &1_u16.to_le_bytes(),
        &SAMPLE_RATE.to_le_bytes(),
        &(SAMPLE_RATE * 2).to_le_bytes(),
        &2_u16.to_le_bytes(),
        &16_u16.to_le_bytes(),
        &b"data"[..],
        &data_bytes.to_le_bytes(),
    ] {
        wav.write_all(bytes).unwrap();
    }
    wav.set_len(u64::from(data_bytes) + 44).unwrap();
    wav.sync_all().unwrap();
    drop(wav);
    assert!(fs::metadata(directory.join("audio.wav")).unwrap().blocks() * 512 < 1024 * 1024);

    let artifact = finalize_audio(&store.root, &mut manifest).unwrap().unwrap();
    assert_eq!(artifact.frames, frames);
    assert_eq!(artifact.sample_rate, 96_000);
    assert_eq!(artifact.channels, 2);
    assert_eq!(artifact.playback, expected.metadata());
    assert_eq!(artifact.playback.as_ref().unwrap().frames, 172_800_000);
    stored.capture_id = Some(manifest.session_id.clone());
    stored.meeting.status = MeetingStatus::Completed;
    stored.meeting.audio = Some(artifact.clone());
    store.save(&stored).unwrap();
    assert_eq!(
        store.audio_path(&stored.meeting).unwrap(),
        directory.join("audio.wav")
    );
    let saved: Manifest = read_json(&directory.join("manifest.json")).unwrap();
    assert_eq!(saved.saved_frames, frames);
    assert_eq!(saved.phase, CapturePhase::Completed);
    let after = fs::metadata(&raw_path).unwrap();
    assert_eq!(after.len(), raw_bytes);
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.blocks(), before.blocks());
    let mut raw = open_read(&raw_path).unwrap();
    let mut actual = [0; 8];
    raw.read_exact(&mut actual).unwrap();
    assert_eq!(actual, sentinel);
    raw.seek(SeekFrom::Start(raw_bytes - 8)).unwrap();
    raw.read_exact(&mut actual).unwrap();
    assert_eq!(actual, sentinel);
}
