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
