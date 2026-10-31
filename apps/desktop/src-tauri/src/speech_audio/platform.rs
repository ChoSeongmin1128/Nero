use super::{check_cancel, decode, failure, formats, wav};
use crate::processing::Processing;
use crate::summary_failure::Failure;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub(super) fn prepare(
    process: &Processing,
    source: &Path,
    destination: &Path,
    work: &Path,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    check_cancel(cancel)?;
    if formats::is_wave(source) {
        return wav::prepare(source, destination, cancel).map(|_| ());
    }
    #[cfg(target_os = "macos")]
    {
        if !formats::portable_on_macos(source) {
            let arguments = [
                source.as_os_str().to_os_string(),
                destination.as_os_str().to_os_string(),
            ];
            return crate::diarization::run(
                process,
                &process.worker("AudioPrepare"),
                &arguments,
                &work.join("prepare.log"),
                cancel,
                false,
                Some(source),
                "prepare-audio",
            )
            .map_err(|error| failure(error.kind));
        }
    }
    let _ = (process, work);
    decode::prepare(source, destination, cancel).map(|_| ())
}
