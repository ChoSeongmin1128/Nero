use crate::speech_choice::{SpeechChoice, SpeechEngine};
use std::sync::OnceLock;

const GIB: u64 = 1024 * 1024 * 1024;
// Product headroom policy: OpenAI's unquantized large model lists about 10 GB
// VRAM. Recommend the optional full model only at 16 GiB unified memory; this
// is a conservative recommendation, not a measured Q5 minimum or quality claim.
const LARGE_RECOMMENDED_MEMORY: u64 = 16 * GIB;
const COMPACT_MEMORY: u64 = 8 * GIB;

fn physical_memory() -> Option<u64> {
    static MEMORY: OnceLock<Option<u64>> = OnceLock::new();
    *MEMORY.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("/usr/sbin/sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .and_then(|text| text.trim().parse::<u64>().ok())
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::SystemInformation::{
                GlobalMemoryStatusEx, MEMORYSTATUSEX,
            };
            let mut status = MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                ..unsafe { std::mem::zeroed() }
            };
            (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            None
        }
    })
}

pub(super) fn apply(choices: &mut [SpeechChoice]) {
    recommend(
        choices,
        cfg!(all(target_os = "macos", target_arch = "aarch64")),
        physical_memory(),
    );
}

fn recommend(choices: &mut [SpeechChoice], apple_silicon: bool, memory: Option<u64>) {
    let supported = |engine| {
        choices
            .iter()
            .any(|choice| choice.id == engine && choice.supported)
    };
    let preferred = if apple_silicon
        && memory.is_some_and(|bytes| bytes >= LARGE_RECOMMENDED_MEMORY)
        && supported(SpeechEngine::WhisperLargeV3)
    {
        Some((
            SpeechEngine::WhisperLargeV3,
            "이 Mac의 메모리에서는 정확도 우선 모델을 선택할 수 있습니다.",
        ))
    } else if apple_silicon
        && memory.is_some_and(|bytes| bytes <= COMPACT_MEMORY)
        && supported(SpeechEngine::AppleSpeech)
    {
        Some((
            SpeechEngine::AppleSpeech,
            "별도 전사 모델 없이 macOS의 음성 인식을 사용할 수 있습니다.",
        ))
    } else if supported(SpeechEngine::Nemotron) {
        Some((
            SpeechEngine::Nemotron,
            "녹음 중 음성 기록을 준비하는 기본 모델입니다.",
        ))
    } else if supported(SpeechEngine::WhisperTurbo) {
        Some((
            SpeechEngine::WhisperTurbo,
            "이 기기에서 사용할 수 있는 다국어 전사 모델입니다.",
        ))
    } else {
        None
    };
    for choice in choices {
        choice.recommended = preferred.is_some_and(|(engine, _)| engine == choice.id);
        choice.recommendation_reason = preferred
            .filter(|(engine, _)| *engine == choice.id)
            .map(|(_, reason)| reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choices() -> Vec<SpeechChoice> {
        SpeechEngine::ALL
            .into_iter()
            .map(|id| SpeechChoice {
                id,
                label: id.title(),
                description: id.description(),
                model_bytes: 0,
                ready: false,
                supported: true,
                recommended: false,
                recommendation_reason: None,
            })
            .collect()
    }
    #[test]
    fn recommendation_uses_memory_and_support_without_changing_selection() {
        let mut rows = choices();
        recommend(&mut rows, true, Some(32 * GIB));
        assert_eq!(
            rows.iter().find(|row| row.recommended).unwrap().id,
            SpeechEngine::WhisperLargeV3
        );
        rows.iter_mut()
            .find(|row| row.id == SpeechEngine::WhisperLargeV3)
            .unwrap()
            .supported = false;
        recommend(&mut rows, true, Some(32 * GIB));
        assert_eq!(
            rows.iter().find(|row| row.recommended).unwrap().id,
            SpeechEngine::Nemotron
        );
        recommend(&mut rows, true, Some(8 * GIB));
        assert_eq!(
            rows.iter().find(|row| row.recommended).unwrap().id,
            SpeechEngine::AppleSpeech
        );
        recommend(&mut rows, true, None);
        assert_eq!(
            rows.iter().find(|row| row.recommended).unwrap().id,
            SpeechEngine::Nemotron
        );
        recommend(&mut rows, false, Some(64 * GIB));
        assert_eq!(
            rows.iter().find(|row| row.recommended).unwrap().id,
            SpeechEngine::Nemotron
        );
        for row in &mut rows {
            row.supported = false;
        }
        recommend(&mut rows, true, Some(64 * GIB));
        assert!(!rows.iter().any(|row| row.recommended));
    }
}
