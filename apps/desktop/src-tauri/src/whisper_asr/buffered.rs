//! Exact PCM ownership, context windows and EOF receipts.
use super::backend::Session;
use super::native::native_words_for_engine;
use super::*;
use std::collections::VecDeque;

pub(crate) struct Buffered {
    runtime: Session,
    model: PathBuf,
    model_sha: String,
    engine: crate::speech_choice::SpeechEngine,
    work: PathBuf,
    samples: VecDeque<i16>,
    base: u64,
    received: u64,
    owned: u64,
    hash: Sha256,
    words: Vec<Value>,
    windows: Vec<Value>,
    pub(crate) generation: u64,
    inherited_tree: bool,
    prompt: String,
}

impl Buffered {
    pub(super) fn pcm_digest(&self) -> String {
        format!("{:x}", self.hash.clone().finalize())
    }

    pub(crate) fn new(
        cli: PathBuf,
        model: PathBuf,
        work: PathBuf,
        inherited_tree: bool,
        engine: crate::speech_choice::SpeechEngine,
        prompt: String,
    ) -> Result<Self, String> {
        if !matches!(
            engine,
            crate::speech_choice::SpeechEngine::WhisperTurbo
                | crate::speech_choice::SpeechEngine::WhisperLargeV3
        ) {
            return Err(failure());
        }
        let manifest: Value = serde_json::from_str(engine.manifest()).map_err(|_| failure())?;
        let model_sha = file_hash(&model)?;
        if Some(model_sha.as_str()) != manifest["files"][0]["sha256"].as_str()
            || plain_file(&model)?
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                != manifest["files"][0]["bytes"].as_u64().ok_or_else(failure)?
        {
            return Err("Whisper 모델 파일을 다시 준비해 주세요.".into());
        }
        let runtime = Session::new(cli)?;
        Ok(Self {
            runtime,
            model,
            model_sha,
            engine,
            work,
            samples: VecDeque::new(),
            base: 0,
            received: 0,
            owned: 0,
            hash: Sha256::new(),
            words: Vec::new(),
            windows: Vec::new(),
            generation: 0,
            inherited_tree,
            prompt,
        })
    }

    pub(crate) fn push(&mut self, samples: &[i16], cancel: &AtomicBool) -> Result<(), String> {
        cancelled(cancel)?;
        for sample in samples {
            self.samples.push_back(*sample);
            self.hash.update(sample.to_le_bytes());
        }
        self.received = self
            .received
            .checked_add(samples.len() as u64)
            .ok_or_else(failure)?;
        while self.received >= self.owned + OWNED + CONTEXT {
            let target = self.owned + OWNED;
            let mut cut = target;
            let mut quiet = 0_u64;
            for position in self.owned + MIN_OWNED..target {
                let sample = self.samples[(position - self.base) as usize];
                if i32::from(sample).abs() < 328 {
                    quiet += 1;
                } else {
                    quiet = 0;
                }
                if quiet >= RATE / 3 {
                    cut = position + 1;
                }
            }
            self.window(cut, cut + CONTEXT, cancel)?;
        }
        Ok(())
    }

    fn window(
        &mut self,
        owned_end: u64,
        input_end: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        cancelled(cancel)?;
        let input_start = self.owned.saturating_sub(CONTEXT);
        if input_start < self.base
            || input_end > self.received
            || input_end <= input_start
            || input_end - input_start > 180 * RATE
        {
            return Err(failure());
        }
        let mut input = tempfile::Builder::new()
            .prefix("whisper-window-")
            .suffix(".wav")
            .tempfile_in(&self.work)
            .map_err(|e| e.to_string())?;
        let mut writer = hound::WavWriter::new(
            input.as_file_mut(),
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut pcm = Sha256::new();
        for sample in self
            .samples
            .iter()
            .skip((input_start - self.base) as usize)
            .take((input_end - input_start) as usize)
        {
            writer.write_sample(*sample).map_err(|e| e.to_string())?;
            pcm.update(sample.to_le_bytes());
        }
        writer.finalize().map_err(|e| e.to_string())?;
        input.as_file().sync_all().map_err(|e| e.to_string())?;
        // Close the writable handle before native CRT/miniaudio opens it on
        // Windows. Keep TempPath ownership so every exit still removes the WAV.
        let input = input.into_temp_path();
        let index = self.windows.len();
        let prefix = self.work.join(format!("whisper-window-{index}"));
        let native_path = prefix.with_extension("json");
        if native_path.exists() {
            return Err("Whisper 원본 결과 경로가 이미 있습니다.".into());
        }
        let run = self.runtime.run(
            &self.model,
            &input,
            &prefix,
            &self.work,
            cancel,
            self.inherited_tree,
            &self.prompt,
        )?;
        let raw = read(&run.native, 16 * 1024 * 1024)?;
        let projected =
            native_words_for_engine(&raw, input_end - input_start, input_start, self.engine)?;
        let begin = self.words.len();
        let mut terminal_contexts = Vec::new();
        for word in projected {
            let start = word["start"].as_f64().ok_or_else(failure)?;
            let end = word["end"].as_f64().ok_or_else(failure)?;
            let center = ((start * RATE as f64).round() + (end * RATE as f64).round()) * 0.5;
            let previous = match self.words.last() {
                Some(last) => Some((
                    last["start"].as_f64().ok_or_else(failure)?,
                    last["end"].as_f64().ok_or_else(failure)?,
                    self.words.len() == begin,
                )),
                None => None,
            };
            let retained = center >= self.owned as f64
                && (center < owned_end as f64
                    || (owned_end == self.received && center == owned_end as f64))
                && !repeats_previous(previous, start, end);
            if let Some(native_end) = word.get("nativeSegmentEndFrame") {
                terminal_contexts.push(json!({"nativeSegmentEndFrame":native_end,
                    "sourceContextStartFrame":(word["start"].as_f64().ok_or_else(failure)?*RATE as f64).round() as u64,
                    "sourceContextEndFrame":input_end,"retainedInOwnedOutput":retained}));
            }
            if retained {
                self.words.push(word);
            }
        }
        let mut receipt = json!({"index":index,"inputStartFrame":input_start,"inputEndFrame":input_end,
            "ownedStartFrame":self.owned,"ownedEndFrame":owned_end,"inputPcmSha256":format!("{:x}",pcm.finalize()),
            "nativeResultSha256":format!("{:x}",Sha256::digest(&raw)),"nativeExitCode":0,
            "actualBackend":run.backend,"backendEvidence":run.evidence,"wordStart":begin,"wordCount":self.words.len()-begin});
        if let Some(execution) = run.execution {
            receipt["execution"] = execution;
        }
        let mut completion = receipt.clone();
        completion["tokenTimestampPolicy"] = "native-token-span-terminal-context-v2".into();
        completion["terminalContexts"] = json!(terminal_contexts);
        save(
            &self
                .work
                .join(format!("whisper-window-{index}-completion.json")),
            &completion,
        )?;
        self.windows.push(receipt);
        self.owned = owned_end;
        self.generation += 1;
        let keep = self.owned.saturating_sub(CONTEXT);
        self.samples.drain(..(keep - self.base) as usize);
        self.base = keep;
        self.runtime.verify()?;
        if file_hash(&self.model)? != self.model_sha {
            return Err("Whisper 실행기나 모델이 처리 중 변경됐습니다.".into());
        }
        Ok(())
    }

    pub(crate) fn preview(&self) -> Value {
        json!({"text":self.words.iter().filter_map(|w|w["word"].as_str()).collect::<Vec<_>>().join(" "),"words":self.words})
    }

    pub(crate) fn finish(
        &mut self,
        frames: u64,
        pcm_sha: &str,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        if frames != self.received
            || frames == 0
            || format!("{:x}", self.hash.clone().finalize()) != pcm_sha
        {
            return Err(failure());
        }
        if self.owned < self.received {
            self.window(self.received, self.received, cancel)?;
        }
        let model: Value = serde_json::from_str(self.engine.manifest()).map_err(|_| failure())?;
        let runtime: Value = serde_json::from_str(RUNTIME).map_err(|_| failure())?;
        Ok(
            json!({"schemaVersion":if cfg!(windows){2}else{1},"model":model["repository"],"modelRevision":model["revision"],"modelSha256":self.model_sha,
            "runtime":runtime["identity"],"target":if cfg!(windows){"windows-x64"}else{"macos-arm64"},
            "processingPolicy":nero_core::WHISPER_PROCESSING_POLICY,"completionEvidence":"whisper-cli-windows-exit0-v1",
            "tokenTimestampPolicy":"native-token-span-terminal-context-v2","completed":true,"inputAudioFrames":frames,
            "audioSampleRate":16000,"audioDurationSeconds":frames as f64/RATE as f64,"inputPcmSha256":pcm_sha,
            "windows":self.windows,"words":self.words}),
        )
    }
}

// A word that runs backward from the word kept before it, or that overlaps the last
// word an earlier window kept, is the same speech recognized twice at the boundary.
// `previous` is (start, end, kept by an earlier window).
fn repeats_previous(previous: Option<(f64, f64, bool)>, start: f64, end: f64) -> bool {
    previous.is_some_and(|(previous_start, previous_end, earlier_window)| {
        start < previous_start || end < previous_end || (earlier_window && start < previous_end)
    })
}

#[cfg(test)]
mod boundary_tests {
    use super::repeats_previous;

    #[test]
    fn only_boundary_repeats_and_backward_words_are_dropped() {
        assert!(!repeats_previous(None, 1.0, 1.2));
        // Next word in the same window, even touching or slightly overlapping.
        assert!(!repeats_previous(Some((1.0, 1.4, false)), 1.3, 1.6));
        // The first word of a new window overlapping the last kept word.
        assert!(repeats_previous(Some((10.0, 10.4, true)), 10.3, 10.9));
        assert!(!repeats_previous(Some((10.0, 10.4, true)), 10.4, 10.9));
        // Running backward never reaches the transcript.
        assert!(repeats_previous(Some((10.0, 10.4, true)), 9.9, 11.0));
        assert!(repeats_previous(Some((10.0, 10.4, false)), 10.1, 10.3));
    }
}
