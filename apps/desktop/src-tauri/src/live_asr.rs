//! Pinned C ABI for one complete recognition stream. The isolated live worker
//! owns this handle; publication and source validation remain in the parent.
use libloading::Library;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
#[repr(C)]
struct Backend {
    size: usize,
    gpu: i32,
}
#[repr(C)]
struct Model {
    size: usize,
    path: *const c_char,
    name: *const c_char,
}
#[repr(C)]
struct Streaming {
    size: usize,
    chunk: f32,
    left: f32,
    right: f32,
    rnnt: i32,
}
#[repr(C)]
struct Endpoint {
    size: usize,
    enable: bool,
    vad: bool,
    history: i32,
}
#[repr(C)]
struct Batching {
    size: usize,
    enable: bool,
    max: i32,
    delay: i32,
    depth: i32,
    ingress: i32,
    arena: i32,
}
#[repr(C)]
struct Config {
    size: usize,
    backend: *const Backend,
    model: *const Model,
    streaming: *const Streaming,
    decoder: *const c_void,
    vad: *const c_void,
    endpoint: *const Endpoint,
    post: *const c_void,
    diar: *const c_void,
    batching: *const Batching,
}
#[repr(C)]
struct Options {
    size: usize,
    request: *const c_char,
    language: *const c_char,
    interim: bool,
    word_offsets: bool,
    punctuation: bool,
    verbatim: bool,
    profanity: bool,
    history: i32,
    contexts: *const c_void,
    context_count: usize,
    alternatives: i32,
    diar: bool,
    max_speakers: i32,
}
type Create = unsafe extern "C" fn(*const Config, *mut *mut c_void) -> i32;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Open = unsafe extern "C" fn(*mut c_void, *const Options, *mut *mut c_void) -> i32;
type Push = unsafe extern "C" fn(*mut c_void, *const f32, usize, i32) -> i32;
type Finish = unsafe extern "C" fn(*mut c_void) -> i32;
type Next = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> i32;
type Text = unsafe extern "C" fn() -> *const c_char;
type WordText = unsafe extern "C" fn(*const c_void, usize, usize) -> *const c_char;
type WordTime = unsafe extern "C" fn(*const c_void, usize, usize) -> i32;
type WordConfidence = unsafe extern "C" fn(*const c_void, usize, usize) -> f32;
type Count = unsafe extern "C" fn(*const c_void, usize) -> usize;
type Transcript = unsafe extern "C" fn(*const c_void, usize) -> *const c_char;
type Final = unsafe extern "C" fn(*const c_void) -> bool;
type Duration = unsafe extern "C" fn(*const c_void) -> f32;

pub(super) struct Asr {
    _library: Library,
    recognizer: *mut c_void,
    stream: *mut c_void,
    destroy: Destroy,
    close: Destroy,
    push: Push,
    finish: Finish,
    next: Next,
    free: Destroy,
    error: Text,
    word_text: WordText,
    word_start: WordTime,
    word_end: WordTime,
    confidence: WordConfidence,
    count: Count,
    final_result: Final,
    duration: Duration,
    transcript: Transcript,
    finished: bool,
}
fn text(pointer: *const c_char) -> Result<String> {
    if pointer.is_null() {
        return Err("Native returned null text".into());
    }
    // The pinned C ABI owns a NUL-terminated string for the accessor lifetime.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "Native text is not UTF-8".into())
}
impl Asr {
    pub(super) fn open(path: &Path, weights: &Path, gpu: i32) -> Result<Self> {
        #[cfg(windows)]
        let library: Library =
            unsafe { libloading::os::windows::Library::load_with_flags(path, 0x100 | 0x800) }
                .map_err(|e| e.to_string())?
                .into();
        #[cfg(not(windows))]
        let library = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
        unsafe {
            macro_rules! get {
                ($name:literal, $t:ty) => {
                    *library
                        .get::<$t>(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                };
            }
            let version = get!("nemo_speech_asr_version", Text);
            if text(version())? != "nemo-speech-asr 0.2.0" {
                return Err("Unexpected speech runtime".into());
            }
            let create = get!("nemo_speech_asr_create", Create);
            let open = get!("nemo_speech_asr_streaming_recognize", Open);
            let defaults = get!(
                "nemo_speech_asr_recognition_options_default",
                unsafe extern "C" fn() -> Options
            );
            let mut this = Self {
                recognizer: std::ptr::null_mut(),
                stream: std::ptr::null_mut(),
                destroy: get!("nemo_speech_asr_destroy", Destroy),
                close: get!("nemo_speech_asr_stream_close", Destroy),
                push: get!("nemo_speech_asr_stream_push_f32", Push),
                finish: get!("nemo_speech_asr_stream_finish", Finish),
                next: get!("nemo_speech_asr_stream_next", Next),
                free: get!("nemo_speech_asr_result_destroy", Destroy),
                error: get!("nemo_speech_asr_last_error", Text),
                word_text: get!("nemo_speech_asr_result_word_text", WordText),
                word_start: get!("nemo_speech_asr_result_word_start_time", WordTime),
                word_end: get!("nemo_speech_asr_result_word_end_time", WordTime),
                confidence: get!("nemo_speech_asr_result_word_confidence", WordConfidence),
                count: get!("nemo_speech_asr_result_word_count", Count),
                final_result: get!("nemo_speech_asr_result_is_final", Final),
                duration: get!("nemo_speech_asr_result_audio_processed", Duration),
                transcript: get!("nemo_speech_asr_result_transcript", Transcript),
                _library: library,
                finished: false,
            };
            let model_path = CString::new(weights.to_str().ok_or("Invalid model path")?)
                .map_err(|e| e.to_string())?;
            let backend = Backend {
                size: std::mem::size_of::<Backend>(),
                gpu,
            };
            let model = Model {
                size: std::mem::size_of::<Model>(),
                path: model_path.as_ptr(),
                name: std::ptr::null(),
            };
            let streaming = Streaming {
                size: std::mem::size_of::<Streaming>(),
                chunk: 0.16,
                left: 1.92,
                right: 1.92,
                rnnt: 13,
            };
            let endpoint = Endpoint {
                size: std::mem::size_of::<Endpoint>(),
                enable: false,
                vad: false,
                history: 0,
            };
            let batching = Batching {
                size: std::mem::size_of::<Batching>(),
                enable: false,
                max: 0,
                delay: 0,
                depth: 0,
                ingress: 0,
                arena: 0,
            };
            let config = Config {
                size: std::mem::size_of::<Config>(),
                backend: &backend,
                model: &model,
                streaming: &streaming,
                endpoint: &endpoint,
                batching: &batching,
                decoder: std::ptr::null(),
                vad: std::ptr::null(),
                post: std::ptr::null(),
                diar: std::ptr::null(),
            };
            let status = create(&config, &mut this.recognizer);
            this.check(status)?;
            if this.recognizer.is_null() {
                return Err("Null recognizer".into());
            }
            let mut options = defaults();
            options.language = c"ko-KR".as_ptr();
            options.interim = true;
            options.word_offsets = true;
            options.diar = false;
            let status = open(this.recognizer, &options, &mut this.stream);
            this.check(status)?;
            if this.stream.is_null() {
                return Err("Null recognition stream".into());
            }
            Ok(this)
        }
    }
    fn check(&self, status: i32) -> Result<()> {
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "ASR status {status}: {}",
                text(unsafe { (self.error)() })?
            ))
        }
    }
    pub(super) fn push(&self, samples: &[f32]) -> Result<()> {
        if self.finished {
            return Err("Push after stream finish".into());
        }
        self.check(unsafe { (self.push)(self.stream, samples.as_ptr(), samples.len(), 16000) })
    }
    pub(super) fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err("Repeated stream finish".into());
        }
        self.check(unsafe { (self.finish)(self.stream) })?;
        self.finished = true;
        Ok(())
    }
    pub(super) fn drain(&self) -> Result<Option<Value>> {
        let mut latest = None;
        loop {
            let mut result = std::ptr::null_mut();
            self.check(unsafe { (self.next)(self.stream, &mut result) })?;
            if result.is_null() {
                return Ok(latest);
            }
            let read: Result<Value> = (|| unsafe {
                let count = (self.count)(result, 0);
                if count > 100_000 {
                    return Err("Word result exceeds limit".into());
                }
                let mut words = Vec::with_capacity(count);
                for i in 0..count {
                    let word = text((self.word_text)(result, 0, i))?;
                    if word.len() > 64 * 1024 {
                        return Err("Word exceeds limit".into());
                    }
                    let start = (self.word_start)(result, 0, i);
                    let end = (self.word_end)(result, 0, i);
                    let confidence = (self.confidence)(result, 0, i);
                    if start < 0 || end < start || !confidence.is_finite() {
                        return Err("Invalid native word".into());
                    }
                    words.push(json!({"word":word,"start":start as f64/1000.0,"end":end as f64/1000.0,"confidence":confidence}));
                }
                let transcript = text((self.transcript)(result, 0))?;
                if transcript.len() > 8 * 1024 * 1024 {
                    return Err("Transcript exceeds limit".into());
                }
                let duration = (self.duration)(result);
                if !duration.is_finite() || duration < 0.0 {
                    return Err("Invalid native duration".into());
                }
                Ok(
                    json!({"words":words,"text":transcript,"final":(self.final_result)(result),"duration":duration}),
                )
            })();
            unsafe { (self.free)(result) };
            latest = Some(read?);
        }
    }
}
impl Drop for Asr {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                (self.close)(self.stream);
            }
            if !self.recognizer.is_null() {
                (self.destroy)(self.recognizer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_64_bit_c_abi_layout_is_preserved() {
        assert_eq!(std::mem::size_of::<usize>(), 8);
        assert_eq!(std::mem::size_of::<Backend>(), 16);
        assert_eq!(std::mem::size_of::<Model>(), 24);
        assert_eq!(std::mem::size_of::<Streaming>(), 24);
        assert_eq!(std::mem::size_of::<Endpoint>(), 16);
        assert_eq!(std::mem::size_of::<Batching>(), 32);
        assert_eq!(std::mem::size_of::<Config>(), 80);
        assert_eq!(std::mem::size_of::<Options>(), 72);
    }
}
