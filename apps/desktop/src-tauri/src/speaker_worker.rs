use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RUNTIME_PIN: &str = "6a3ca369370782acee0dd155e34394ba60b920c4";
const MODEL_SHA256: &str = "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1";
const MODEL_BYTES: u64 = 107_012_128;
const SAMPLE_RATE: u32 = 16_000;
const SLOTS: usize = 8;
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_POSTERIOR_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SEGMENTS: usize = 100_000;
const POLICY: &str = "nemotron-v3-contiguous-threshold-0.5-v1";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkerFailureKind {
    BackendUnavailable,
    Allocation,
    Input,
    Protocol,
    Storage,
    ModelNotReady,
    EngineUnavailable,
    SourceChanged,
}

#[derive(Debug)]
pub struct WorkerError {
    pub kind: WorkerFailureKind,
    pub message: String,
}

impl WorkerError {
    fn new(kind: WorkerFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn error_json(&self) -> Value {
        json!({"error":{"message":self.message,"type":"nero_speaker_worker",
            "failureKind":self.kind,"command":"diarize","exit_code":1}})
    }
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WorkerError {}

type Result<T> = std::result::Result<T, WorkerError>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Backend {
    Cpu,
    Metal,
    Vulkan,
}

impl Backend {
    fn gpu(self) -> i32 {
        if matches!(self, Self::Cpu) {
            -1
        } else {
            0
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema_version: u32,
    audio: PathBuf,
    model: PathBuf,
    library: PathBuf,
    output: PathBuf,
    backend: Backend,
    model_sha256: String,
    runtime_pin: String,
}

#[repr(C)]
struct ModelConfig {
    size: usize,
    model_path: *const c_char,
    gpu: i32,
    preset: *const c_char,
    chunk_frames: i32,
    right_context_frames: i32,
    left_context_frames: i32,
    fifo_frames: i32,
    spkcache_frames: i32,
    update_period_frames: i32,
}

type Create = unsafe extern "C" fn(*const ModelConfig, *mut *mut c_void) -> i32;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Slots = unsafe extern "C" fn(*const c_void) -> i32;
type Stride = unsafe extern "C" fn(*const c_void) -> f64;
type Open = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> i32;
type Push = unsafe extern "C" fn(*mut c_void, *const f32, usize, i32) -> i32;
type Finish = unsafe extern "C" fn(*mut c_void) -> i32;
type Count = unsafe extern "C" fn(*const c_void) -> i64;
type Probs = unsafe extern "C" fn(*const c_void, *mut f32, usize) -> i32;
type Text = unsafe extern "C" fn() -> *const c_char;

struct Api {
    _library: Library,
    create: Create,
    destroy: Destroy,
    slots: Slots,
    stride: Stride,
    open: Open,
    push: Push,
    finish: Finish,
    close: Destroy,
    count: Count,
    base: Count,
    probs: Probs,
    error: Text,
    version: Text,
}

fn loader_error(error: libloading::Error, backend: Backend) -> WorkerError {
    let missing_dependency = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .and_then(std::io::Error::raw_os_error)
        == Some(126);
    let kind = if cfg!(windows) && missing_dependency && !matches!(backend, Backend::Cpu) {
        WorkerFailureKind::BackendUnavailable
    } else {
        WorkerFailureKind::EngineUnavailable
    };
    WorkerError::new(
        kind,
        format!("Cannot load packaged speech library: {error}"),
    )
}

impl Api {
    fn load(path: &Path, backend: Backend) -> Result<Self> {
        #[cfg(windows)]
        let library: Library = unsafe {
            // Resolve dependencies beside the verified DLL and in System32 only.
            libloading::os::windows::Library::load_with_flags(path, 0x0000_0100 | 0x0000_0800)
        }
        .map_err(|error| loader_error(error, backend))?
        .into();
        #[cfg(not(windows))]
        let library =
            unsafe { Library::new(path) }.map_err(|error| loader_error(error, backend))?;
        unsafe {
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|error| {
                            WorkerError::new(
                                WorkerFailureKind::Protocol,
                                format!("Missing pinned C ABI symbol {}: {error}", $name),
                            )
                        })?
                };
            }
            Ok(Self {
                create: symbol!("nemo_speech_diar_create", Create),
                destroy: symbol!("nemo_speech_diar_destroy", Destroy),
                slots: symbol!("nemo_speech_diar_num_speakers", Slots),
                stride: symbol!("nemo_speech_diar_seconds_per_frame", Stride),
                open: symbol!("nemo_speech_diar_stream_open", Open),
                push: symbol!("nemo_speech_diar_stream_push_f32", Push),
                finish: symbol!("nemo_speech_diar_stream_finish", Finish),
                close: symbol!("nemo_speech_diar_stream_close", Destroy),
                count: symbol!("nemo_speech_diar_frame_count", Count),
                base: symbol!("nemo_speech_diar_frame_probs_start", Count),
                probs: symbol!("nemo_speech_diar_frame_probs", Probs),
                error: symbol!("nemo_speech_asr_last_error", Text),
                version: symbol!("nemo_speech_asr_version", Text),
                _library: library,
            })
        }
    }

    fn text(&self, getter: Text) -> Result<String> {
        let pointer = unsafe { getter() };
        if pointer.is_null() {
            return Ok(String::new());
        }
        let value = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        if value.len() > 64 * 1024 {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Unbounded native error/version string",
            ));
        }
        Ok(value)
    }

    fn check(&self, status: i32, operation: &str, backend: Backend) -> Result<()> {
        if status == 0 {
            return Ok(());
        }
        let message = self.text(self.error)?;
        let gpu_missing = message
            == "use_gpu=true but no matching GPU device found (gpu_device_idx=0)"
            || message == "--backend requested Metal, but this build has no Metal backend"
            || message == "--backend requested Vulkan, but this build has no Vulkan backend";
        let kind = if status == 2 {
            WorkerFailureKind::Allocation
        } else if gpu_missing && !matches!(backend, Backend::Cpu) {
            WorkerFailureKind::BackendUnavailable
        } else if status == 1 {
            WorkerFailureKind::Protocol
        } else {
            WorkerFailureKind::EngineUnavailable
        };
        Err(WorkerError::new(
            kind,
            format!("{operation}: native status {status}: {message}"),
        ))
    }
}

struct Native<'a> {
    api: &'a Api,
    model: *mut c_void,
    stream: *mut c_void,
    backend: Backend,
}

impl<'a> Native<'a> {
    fn open(api: &'a Api, path: &Path, backend: Backend) -> Result<Self> {
        let mut native = Self {
            api,
            model: std::ptr::null_mut(),
            stream: std::ptr::null_mut(),
            backend,
        };
        let path = path.to_str().ok_or_else(|| {
            WorkerError::new(WorkerFailureKind::Input, "Model path is not valid UTF-8")
        })?;
        let model_path = CString::new(path)
            .map_err(|_| WorkerError::new(WorkerFailureKind::Input, "Model path contains NUL"))?;
        let preset = b"v3-offline\0";
        let config = ModelConfig {
            size: std::mem::size_of::<ModelConfig>(),
            model_path: model_path.as_ptr(),
            gpu: backend.gpu(),
            preset: preset.as_ptr().cast(),
            chunk_frames: 340,
            right_context_frames: 40,
            left_context_frames: 0,
            fifo_frames: 40,
            spkcache_frames: 264,
            update_period_frames: 300,
        };
        api.check(
            unsafe { (api.create)(&config, &mut native.model) },
            "create",
            backend,
        )?;
        if native.model.is_null() {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Native create returned a null model",
            ));
        }
        let slots = unsafe { (api.slots)(native.model) };
        let stride = unsafe { (api.stride)(native.model) };
        if slots != SLOTS as i32
            || !stride.is_finite()
            || stride.to_bits() != f64::from(0.01_f32).to_bits()
        {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                format!("Unexpected V3 cadence/capacity: {stride}/{slots}"),
            ));
        }
        api.check(
            unsafe { (api.open)(native.model, &mut native.stream) },
            "stream_open",
            backend,
        )?;
        if native.stream.is_null() {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Native open returned a null stream",
            ));
        }
        Ok(native)
    }

    fn push(&self, samples: &[f32]) -> Result<()> {
        self.api.check(
            unsafe {
                (self.api.push)(
                    self.stream,
                    samples.as_ptr(),
                    samples.len(),
                    SAMPLE_RATE as i32,
                )
            },
            "stream_push",
            self.backend,
        )
    }

    fn finish(&self) -> Result<()> {
        self.api.check(
            unsafe { (self.api.finish)(self.stream) },
            "stream_finish",
            self.backend,
        )
    }

    fn collect(&self, timeline: &mut Timeline, raw: &mut impl Write, maximum: u64) -> Result<()> {
        let count = u64::try_from(unsafe { (self.api.count)(self.stream) }).map_err(|_| {
            WorkerError::new(WorkerFailureKind::Protocol, "Negative native frame count")
        })?;
        let base = u64::try_from(unsafe { (self.api.base)(self.stream) }).map_err(|_| {
            WorkerError::new(WorkerFailureKind::Protocol, "Negative native frame base")
        })?;
        if base > count || count > maximum || count < timeline.exported || base > timeline.exported
        {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Native frame range is incomplete or out of bounds",
            ));
        }
        if count == timeline.exported {
            return Ok(());
        }
        let values = usize::try_from(count - base)
            .ok()
            .and_then(|frames| frames.checked_mul(SLOTS))
            .filter(|values| *values <= MAX_RETAINED_BYTES / 4)
            .ok_or_else(|| {
                WorkerError::new(
                    WorkerFailureKind::Allocation,
                    "Native retained probability range exceeds bounded buffer",
                )
            })?;
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(values).map_err(|_| {
            WorkerError::new(
                WorkerFailureKind::Allocation,
                "Cannot allocate retained probability buffer",
            )
        })?;
        buffer.resize(values, 0.0_f32);
        self.api.check(
            unsafe { (self.api.probs)(self.stream, buffer.as_mut_ptr(), buffer.len()) },
            "frame_probs",
            self.backend,
        )?;
        timeline.append(base, count, buffer, raw)
    }
}

impl Drop for Native<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                (self.api.close)(self.stream);
            }
            if !self.model.is_null() {
                (self.api.destroy)(self.model);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameActivity {
    start: u64,
    end: u64,
    speaker: usize,
}

struct Activity {
    starts: [Option<u64>; SLOTS],
    segments: Vec<FrameActivity>,
}

impl Activity {
    fn new() -> Self {
        Self {
            starts: [None; SLOTS],
            segments: Vec::new(),
        }
    }

    fn push(&mut self, frame: u64, values: &[f32]) -> Result<()> {
        for (slot, value) in values.iter().enumerate() {
            if *value > 0.5 {
                self.starts[slot].get_or_insert(frame);
            } else if let Some(start) = self.starts[slot].take() {
                self.close(start, frame, slot)?;
            }
        }
        Ok(())
    }

    fn close(&mut self, start: u64, end: u64, slot: usize) -> Result<()> {
        if end <= start || self.segments.len() >= MAX_SEGMENTS {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Invalid or excessive contiguous speaker segments",
            ));
        }
        self.segments.try_reserve(1).map_err(|_| {
            WorkerError::new(
                WorkerFailureKind::Allocation,
                "Cannot allocate activity segment",
            )
        })?;
        self.segments.push(FrameActivity {
            start,
            end,
            speaker: slot + 1,
        });
        Ok(())
    }

    fn finish(mut self, end: u64) -> Result<Vec<FrameActivity>> {
        for slot in 0..SLOTS {
            if let Some(start) = self.starts[slot].take() {
                self.close(start, end, slot)?;
            }
        }
        self.segments
            .sort_by_key(|segment| (segment.start, segment.speaker, segment.end));
        Ok(self.segments)
    }
}

struct Timeline {
    exported: u64,
    base: u64,
    previous: Vec<f32>,
    hash: Sha256,
    activity: Activity,
    batches: u64,
}

impl Timeline {
    fn new() -> Self {
        Self {
            exported: 0,
            base: 0,
            previous: Vec::new(),
            hash: Sha256::new(),
            activity: Activity::new(),
            batches: 0,
        }
    }

    fn append(
        &mut self,
        base: u64,
        count: u64,
        values: Vec<f32>,
        output: &mut impl Write,
    ) -> Result<()> {
        if base < self.base
            || base > self.exported
            || count < self.exported
            || count < base
            || usize::try_from(count - base)
                .ok()
                .and_then(|n| n.checked_mul(SLOTS))
                != Some(values.len())
            || values
                .iter()
                .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
        {
            return Err(WorkerError::new(
                WorkerFailureKind::Protocol,
                "Invalid or missing native probability frames",
            ));
        }
        let overlap_start = base.max(self.base);
        for frame in overlap_start..self.exported {
            let old = ((frame - self.base) as usize) * SLOTS;
            let new = ((frame - base) as usize) * SLOTS;
            if self.previous[old..old + SLOTS]
                .iter()
                .zip(&values[new..new + SLOTS])
                .any(|(old, new)| old.to_bits() != new.to_bits())
            {
                return Err(WorkerError::new(
                    WorkerFailureKind::Protocol,
                    "Native probabilities changed for an already exported frame",
                ));
            }
        }
        for frame in self.exported..count {
            let index = ((frame - base) as usize) * SLOTS;
            let row = &values[index..index + SLOTS];
            self.activity.push(frame, row)?;
            for value in row {
                let bytes = value.to_le_bytes();
                output.write_all(&bytes).map_err(storage)?;
                self.hash.update(bytes);
            }
        }
        self.exported = count;
        self.base = base;
        self.previous = values;
        self.batches += 1;
        Ok(())
    }
}

fn storage(error: std::io::Error) -> WorkerError {
    WorkerError::new(WorkerFailureKind::Storage, error.to_string())
}

fn plain_file(path: &Path, maximum: u64, kind: WorkerFailureKind) -> Result<File> {
    if !path.is_absolute() {
        return Err(WorkerError::new(kind, "An absolute file path is required"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| WorkerError::new(kind, error.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(WorkerError::new(
            kind,
            "Invalid, linked, or oversized input file",
        ));
    }
    File::open(path).map_err(|error| WorkerError::new(kind, error.to_string()))
}

fn hash(path: &Path, maximum: u64, kind: WorkerFailureKind) -> Result<String> {
    let mut file = plain_file(path, maximum, kind)?;
    let mut sha = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| WorkerError::new(kind, error.to_string()))?;
        if count == 0 {
            break;
        }
        sha.update(&buffer[..count]);
    }
    Ok(format!("{:x}", sha.finalize()))
}

fn new_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(storage)
}

fn save(path: &Path, document: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(document)
        .map_err(|error| WorkerError::new(WorkerFailureKind::Protocol, error.to_string()))?;
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(WorkerError::new(
            WorkerFailureKind::Protocol,
            "Worker result exceeds output limit",
        ));
    }
    let mut file = new_file(path)?;
    file.write_all(&bytes).map_err(storage)?;
    file.write_all(b"\n").map_err(storage)?;
    file.sync_all().map_err(storage)
}

fn validate_pcm_container(path: &Path) -> Result<()> {
    let mut file = plain_file(path, u64::from(u32::MAX) + 8, WorkerFailureKind::Input)?;
    let length = file.metadata().map_err(storage)?.len();
    let mut header = [0_u8; 12];
    file.read_exact(&mut header)
        .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
    if &header[..4] != b"RIFF"
        || &header[8..] != b"WAVE"
        || u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8 != length
    {
        return Err(WorkerError::new(
            WorkerFailureKind::Input,
            "Input must use a complete RIFF/WAVE container",
        ));
    }
    let mut position = 12_u64;
    while position.checked_add(8).is_some_and(|end| end <= length) {
        let mut chunk = [0_u8; 8];
        file.read_exact(&mut chunk)
            .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        let end = position
            .checked_add(8)
            .and_then(|n| n.checked_add(size))
            .and_then(|n| n.checked_add(size % 2))
            .filter(|end| *end <= length)
            .ok_or_else(|| WorkerError::new(WorkerFailureKind::Input, "Truncated WAV chunk"))?;
        if &chunk[..4] == b"fmt " {
            let mut tag = [0_u8; 2];
            if size < 16 {
                return Err(WorkerError::new(
                    WorkerFailureKind::Input,
                    "Incomplete PCM WAV format",
                ));
            }
            file.read_exact(&mut tag)
                .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
            if u16::from_le_bytes(tag) != 1 {
                return Err(WorkerError::new(
                    WorkerFailureKind::Input,
                    "WAV format must be canonical PCM tag 1",
                ));
            }
            return Ok(());
        }
        file.seek(SeekFrom::Start(end))
            .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
        position = end;
    }
    Err(WorkerError::new(
        WorkerFailureKind::Input,
        "WAV PCM format chunk is missing",
    ))
}

struct Prepared {
    source_sha: String,
    model_sha: String,
    library_sha: String,
    frames: u64,
}

fn prepare(request: &Request) -> Result<Prepared> {
    if request.schema_version != 1
        || request.runtime_pin != RUNTIME_PIN
        || request.model_sha256 != MODEL_SHA256
        || std::mem::size_of::<usize>() != 8
        || std::mem::size_of::<ModelConfig>() != 56
    {
        return Err(WorkerError::new(
            WorkerFailureKind::Protocol,
            "Request does not match the pinned worker contract",
        ));
    }
    if !request.output.is_absolute() {
        return Err(WorkerError::new(
            WorkerFailureKind::Input,
            "An absolute output directory is required",
        ));
    }
    let directory = fs::symlink_metadata(&request.output).map_err(storage)?;
    if !directory.is_dir() || directory.file_type().is_symlink() {
        return Err(WorkerError::new(
            WorkerFailureKind::Storage,
            "Output must be an existing plain directory",
        ));
    }
    if fs::metadata(&request.model)
        .map_err(|error| WorkerError::new(WorkerFailureKind::ModelNotReady, error.to_string()))?
        .len()
        != MODEL_BYTES
    {
        return Err(WorkerError::new(
            WorkerFailureKind::ModelNotReady,
            "Model byte size does not match the pin",
        ));
    }
    let model_sha = hash(
        &request.model,
        MODEL_BYTES,
        WorkerFailureKind::ModelNotReady,
    )?;
    if model_sha != MODEL_SHA256 {
        return Err(WorkerError::new(
            WorkerFailureKind::ModelNotReady,
            "Model SHA256 does not match the pin",
        ));
    }
    validate_pcm_container(&request.audio)?;
    let source_sha = hash(
        &request.audio,
        u64::from(u32::MAX) + 8,
        WorkerFailureKind::Input,
    )?;
    let library_sha = hash(
        &request.library,
        32 * 1024 * 1024,
        WorkerFailureKind::EngineUnavailable,
    )?;
    let reader = hound::WavReader::new(BufReader::new(plain_file(
        &request.audio,
        u64::from(u32::MAX) + 8,
        WorkerFailureKind::Input,
    )?))
    .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
    let spec = reader.spec();
    let frames = u64::from(reader.duration());
    if spec.channels != 1
        || spec.sample_rate != SAMPLE_RATE
        || spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
        || frames == 0
    {
        return Err(WorkerError::new(
            WorkerFailureKind::Input,
            "Input must be nonempty mono16k PCM16 WAV",
        ));
    }
    if (frames / 160 + 1)
        .checked_mul((SLOTS * 4) as u64)
        .is_none_or(|bytes| bytes > MAX_POSTERIOR_BYTES)
    {
        return Err(WorkerError::new(
            WorkerFailureKind::Input,
            "Recording exceeds the bounded posterior output size",
        ));
    }
    Ok(Prepared {
        source_sha,
        model_sha,
        library_sha,
        frames,
    })
}

fn process(request: &Request, input: &Prepared, started: Instant) -> Result<()> {
    let mut raw = BufWriter::new(new_file(&request.output.join("probabilities.f32le"))?);
    let mut progress = BufWriter::new(new_file(&request.output.join("progress.jsonl"))?);
    let api = Api::load(&request.library, request.backend)?;
    let version = api.text(api.version)?;
    if version != "nemo-speech-asr 0.2.0" {
        return Err(WorkerError::new(
            WorkerFailureKind::Protocol,
            format!("Unexpected runtime version: {version}"),
        ));
    }
    let native = Native::open(&api, &request.model, request.backend)?;
    let stride = unsafe { (api.stride)(native.model) };
    let expected = input.frames / 160 + 1;
    let mut timeline = Timeline::new();
    let mut reader = hound::WavReader::new(BufReader::new(plain_file(
        &request.audio,
        u64::from(u32::MAX) + 8,
        WorkerFailureKind::Input,
    )?))
    .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
    let mut samples = reader.samples::<i16>();
    let mut pushed = 0_u64;
    loop {
        let mut block = Vec::with_capacity(SAMPLE_RATE as usize);
        for _ in 0..SAMPLE_RATE {
            match samples.next() {
                Some(Ok(sample)) => block.push(f32::from(sample) / 32768.0),
                Some(Err(error)) => {
                    return Err(WorkerError::new(
                        WorkerFailureKind::Input,
                        error.to_string(),
                    ))
                }
                None => break,
            }
        }
        if block.is_empty() {
            break;
        }
        pushed = pushed
            .checked_add(block.len() as u64)
            .filter(|n| *n <= input.frames)
            .ok_or_else(|| {
                WorkerError::new(
                    WorkerFailureKind::Input,
                    "Audio has unexpected extra samples",
                )
            })?;
        native.push(&block)?;
        let previous = timeline.exported;
        native.collect(&mut timeline, &mut raw, expected)?;
        if timeline.exported != previous {
            serde_json::to_writer(&mut progress, &json!({"sourceFramesFed":pushed,
                "nativeFrames":timeline.exported,"retainedFrameBase":timeline.base,"elapsedSeconds":started.elapsed().as_secs_f64()}))
                .map_err(|error| WorkerError::new(WorkerFailureKind::Storage, error.to_string()))?;
            progress.write_all(b"\n").map_err(storage)?;
            progress.flush().map_err(storage)?;
        }
    }
    if pushed != input.frames {
        return Err(WorkerError::new(
            WorkerFailureKind::Input,
            "Audio ended before its declared frame count",
        ));
    }
    native.finish()?;
    native.collect(&mut timeline, &mut raw, expected)?;
    if timeline.exported != expected {
        return Err(WorkerError::new(
            WorkerFailureKind::Protocol,
            "Native output does not cover the complete centered-STFT timeline",
        ));
    }
    raw.flush().map_err(storage)?;
    raw.get_ref().sync_all().map_err(storage)?;
    progress.flush().map_err(storage)?;
    progress.get_ref().sync_all().map_err(storage)?;
    let raw_bytes = expected.checked_mul((SLOTS * 4) as u64).ok_or_else(|| {
        WorkerError::new(WorkerFailureKind::Protocol, "Probability size overflow")
    })?;
    if fs::metadata(request.output.join("probabilities.f32le"))
        .map_err(storage)?
        .len()
        != raw_bytes
    {
        return Err(WorkerError::new(
            WorkerFailureKind::Storage,
            "Probability file has an unexpected length",
        ));
    }
    let probability_sha = format!("{:x}", timeline.hash.finalize());
    let activities = timeline.activity.finish(expected)?;
    let duration = input.frames as f64 / f64::from(SAMPLE_RATE);
    let segments: Vec<_> = activities
        .iter()
        .filter_map(|segment| {
            let start = (segment.start as f64 * stride).min(duration);
            let end = (segment.end as f64 * stride).min(duration);
            (start < end).then(|| json!({"start":start,"end":end,"speaker":segment.speaker}))
        })
        .collect();
    for (path, expected_hash, maximum, kind) in [
        (
            &request.audio,
            &input.source_sha,
            u64::from(u32::MAX) + 8,
            WorkerFailureKind::SourceChanged,
        ),
        (
            &request.model,
            &input.model_sha,
            MODEL_BYTES,
            WorkerFailureKind::SourceChanged,
        ),
        (
            &request.library,
            &input.library_sha,
            32 * 1024 * 1024,
            WorkerFailureKind::SourceChanged,
        ),
    ] {
        if hash(path, maximum, kind)? != *expected_hash {
            return Err(WorkerError::new(
                WorkerFailureKind::SourceChanged,
                "Audio, model, or runtime changed while processing",
            ));
        }
    }
    let result = json!({"segments":segments});
    let result_bytes = serde_json::to_vec(&result)
        .map_err(|error| WorkerError::new(WorkerFailureKind::Protocol, error.to_string()))?;
    let result_sha = format!("{:x}", Sha256::digest(&result_bytes));
    save(&request.output.join("diarization.json"), &result)?;
    let policy = json!({
        "schemaVersion":1,"status":"completed","processingPolicy":"whole-recording-cabi-v1",
        "postprocessingPolicy":POLICY,"threshold":0.5,"thresholdComparison":">",
        "padOnset":0,"padOffset":0,"minDurationOn":0,"minDurationOff":0,
        "geometry":{"timebaseSeconds":0.08,"spkcache":264,"fifo":40,"chunk":340,
            "updatePeriod":300,"leftContext":0,"rightContext":40},
        "geometryEvidence":"pinned-c-abi-overrides-no-runtime-readback"
    });
    let identity = json!({
        "runtimePin":RUNTIME_PIN,"runtimeVersionString":version,"backend":request.backend,
        "gpuIndex":request.backend.gpu(),"sourceAudioSha256":input.source_sha,
        "modelSha256":input.model_sha,"librarySha256":input.library_sha,
        "inputAndWeightsAndLibraryUnchanged":true
    });
    let completion = json!({
        "sourceFramesFed":pushed,"sourceSampleRate":SAMPLE_RATE,"nativeFrames":expected,
        "expectedNativeFrames":expected,"speakerSlots":SLOTS,"secondsPerFrame":stride,
        "nativeStreamFinished":true,"probabilitySHA256":probability_sha,
        "probabilityBytes":raw_bytes,"probabilityFormat":"frame-major-f32le",
        "lastRetainedFrameBase":timeline.base,"exportBatches":timeline.batches,
        "overlapCheck":"all-retained-frame-bitwise-equality",
        "diarizationJsonSHA256WithoutNewline":result_sha,
        "activityEndClipping":"exact-source-duration-v1","segmentCount":segments.len(),
        "elapsedSeconds":started.elapsed().as_secs_f64()
    });
    let mut receipt = serde_json::Map::new();
    for section in [policy, identity, completion] {
        receipt.extend(section.as_object().expect("receipt section").clone());
    }
    save(
        &request.output.join("receipt.json"),
        &Value::Object(receipt),
    )?;
    Ok(())
}

pub fn entry(request_path: &Path) -> Result<()> {
    let file = plain_file(request_path, MAX_REQUEST_BYTES, WorkerFailureKind::Input)?;
    let request: Request = serde_json::from_reader(BufReader::new(file))
        .map_err(|error| WorkerError::new(WorkerFailureKind::Input, error.to_string()))?;
    let input = prepare(&request)?;
    let started = Instant::now();
    let result = process(&request, &input, started);
    let outcome = save(
        &request.output.join("worker-outcome.json"),
        &match &result {
            Ok(()) => json!({"status":"completed","nativeStreamFinished":true,
            "sourceFramesFed":input.frames,"elapsedSeconds":started.elapsed().as_secs_f64()}),
            Err(error) => {
                json!({"status":"failed","failureKind":error.kind,"message":error.message,
            "elapsedSeconds":started.elapsed().as_secs_f64()})
            }
        },
    );
    match (result, outcome) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(values: &[[f32; SLOTS]]) -> Vec<f32> {
        values.iter().flatten().copied().collect()
    }
    fn row(first: f32, second: f32) -> [f32; SLOTS] {
        let mut value = [0.0; SLOTS];
        value[0] = first;
        value[1] = second;
        value
    }

    #[test]
    fn canonical_pcm_validation_rejects_extensible_float_and_incomplete_containers() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("pcm.wav");
        let mut writer = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample(123_i16).unwrap();
        writer.finalize().unwrap();
        assert!(validate_pcm_container(&source).is_ok());
        let original = fs::read(&source).unwrap();
        for (name, tag) in [("extensible.wav", 0xfffe_u16), ("float.wav", 3_u16)] {
            let path = temporary.path().join(name);
            let mut bytes = original.clone();
            bytes[20..22].copy_from_slice(&tag.to_le_bytes());
            fs::write(&path, bytes).unwrap();
            assert!(validate_pcm_container(&path).is_err());
        }
        let truncated = temporary.path().join("truncated.wav");
        fs::write(&truncated, &original[..original.len() - 1]).unwrap();
        assert!(validate_pcm_container(&truncated).is_err());
    }

    #[test]
    fn structured_error_retains_parent_protocol_and_specific_failure_kind() {
        let error = WorkerError::new(WorkerFailureKind::BackendUnavailable, "GPU absent");
        assert_eq!(
            error.error_json(),
            json!({"error":{
                "message":"GPU absent","type":"nero_speaker_worker","command":"diarize",
                "exit_code":1,"failureKind":"backendUnavailable"
            }})
        );
    }

    #[test]
    fn official_boolean_threshold_preserves_short_overlapping_turns_across_exports() {
        let mut timeline = Timeline::new();
        let mut output = Vec::new();
        timeline
            .append(
                0,
                3,
                rows(&[row(0.5, 0.0), row(0.6, 0.7), row(0.8, 0.0)]),
                &mut output,
            )
            .unwrap();
        timeline
            .append(
                2,
                5,
                rows(&[row(0.8, 0.0), row(0.0, 0.7), row(0.0, 0.7)]),
                &mut output,
            )
            .unwrap();
        assert_eq!(output.len(), 5 * SLOTS * 4);
        assert_eq!(
            timeline.activity.finish(5).unwrap(),
            vec![
                FrameActivity {
                    start: 1,
                    end: 3,
                    speaker: 1
                },
                FrameActivity {
                    start: 1,
                    end: 2,
                    speaker: 2
                },
                FrameActivity {
                    start: 3,
                    end: 5,
                    speaker: 2
                },
            ]
        );
    }

    #[test]
    fn compaction_does_not_close_or_pad_an_active_speaker() {
        let mut timeline = Timeline::new();
        let mut output = Vec::new();
        timeline
            .append(0, 2, rows(&[row(0.8, 0.0), row(0.8, 0.0)]), &mut output)
            .unwrap();
        timeline
            .append(
                1,
                4,
                rows(&[row(0.8, 0.0), row(0.8, 0.0), row(0.0, 0.0)]),
                &mut output,
            )
            .unwrap();
        assert_eq!(
            timeline.activity.finish(4).unwrap(),
            vec![FrameActivity {
                start: 0,
                end: 3,
                speaker: 1
            }]
        );
    }

    #[test]
    fn a_gap_revision_or_invalid_probability_never_silently_completes() {
        for failure in [0, 1, 2] {
            let mut timeline = Timeline::new();
            let mut output = Vec::new();
            timeline
                .append(0, 2, rows(&[row(0.8, 0.0), row(0.8, 0.0)]), &mut output)
                .unwrap();
            let result = match failure {
                0 => timeline.append(3, 4, rows(&[row(0.8, 0.0)]), &mut output),
                1 => timeline.append(1, 3, rows(&[row(0.7, 0.0), row(0.8, 0.0)]), &mut output),
                _ => timeline.append(
                    1,
                    3,
                    rows(&[row(0.8, 0.0), row(f32::NAN, 0.0)]),
                    &mut output,
                ),
            };
            assert!(result.is_err());
            assert_eq!(output.len(), 2 * SLOTS * 4);
            assert_eq!(timeline.exported, 2);
        }
    }

    #[test]
    fn exported_bytes_and_digest_have_one_global_frame_order() {
        let mut timeline = Timeline::new();
        let mut output = Vec::new();
        timeline
            .append(0, 2, rows(&[row(0.8, 0.0), row(0.8, 0.0)]), &mut output)
            .unwrap();
        timeline
            .append(1, 3, rows(&[row(0.8, 0.0), row(0.0, 0.0)]), &mut output)
            .unwrap();
        let expected: Vec<u8> = rows(&[row(0.8, 0.0), row(0.8, 0.0), row(0.0, 0.0)])
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(output, expected);
        assert_eq!(
            format!("{:x}", timeline.hash.finalize()),
            format!("{:x}", Sha256::digest(&output))
        );
    }
}
