use crate::speech_choice::{SpeechChoice, SpeechEngine};
use std::sync::OnceLock;

const GIB: u64 = 1024 * 1024 * 1024;
// Product headroom policy: OpenAI's unquantized large model lists about 10 GB
// VRAM. Recommend the optional full model only at 16 GiB unified memory; this
// is a conservative recommendation, not a measured Q5 minimum or quality claim.
// Measured peak memory footprint of one CrispASR Qwen + aligner process (M4 Pro, 2026-11-04).
const QWEN_PEAK: u64 = 4_475_539_632;
const QWEN_RECOMMENDED_MEMORY: u64 = 16 * GIB;

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
    let preferred = if apple_silicon && supported(SpeechEngine::Moss) {
        Some((
            SpeechEngine::Moss,
            "녹음 종료 후 구간별로 전사하는 기본 모델입니다.",
        ))
    } else if !apple_silicon && supported(SpeechEngine::Nemotron) {
        Some((
            SpeechEngine::Nemotron,
            "이 기기에서 사용할 수 있는 기본 전사 모델입니다.",
        ))
    } else {
        None
    };
    for choice in choices {
        choice.recommended = preferred.is_some_and(|(engine, _)| engine == choice.id);
        choice.recommendation_reason = preferred
            .filter(|(engine, _)| *engine == choice.id)
            .map(|(_, reason)| reason);
        choice.memory_warning = (choice.id == SpeechEngine::QwenKo)
            .then(|| qwen_memory_warning(memory))
            .flatten();
    }
}

fn qwen_memory_warning(memory: Option<u64>) -> Option<String> {
    let peak = QWEN_PEAK as f64 / 1e9;
    match memory {
        Some(bytes) if bytes >= QWEN_RECOMMENDED_MEMORY => None,
        Some(bytes) => {
            let share = if QWEN_PEAK * 2 >= bytes {
                "절반 이상을".to_owned()
            } else {
                format!("약 {}%를", QWEN_PEAK * 100 / bytes)
            };
            Some(format!(
                "이 Mac 메모리 {}GB 중 약 {peak:.1}GB, {share} 쓸 수 있어 처리 중 느려지거나 실패할 수 있습니다.",
                bytes / GIB
            ))
        }
        None => Some(format!(
            "이 Mac의 메모리를 확인하지 못했습니다. 처리 중 약 {peak:.1}GB를 쓸 수 있어 느려지거나 실패할 수 있습니다."
        )),
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
                memory_warning: None,
            })
            .collect()
    }
    #[test]
    fn mac_always_recommends_moss_and_never_qwen() {
        let mut rows = choices();
        for memory in [None, Some(8 * GIB), Some(16 * GIB), Some(64 * GIB)] {
            recommend(&mut rows, true, memory);
            assert_eq!(
                rows.iter().find(|r| r.recommended).unwrap().id,
                SpeechEngine::Moss
            );
            assert!(
                !rows
                    .iter()
                    .find(|r| r.id == SpeechEngine::QwenKo)
                    .unwrap()
                    .recommended
            );
        }
        rows.iter_mut()
            .find(|r| r.id == SpeechEngine::Moss)
            .unwrap()
            .supported = false;
        recommend(&mut rows, true, Some(64 * GIB));
        assert!(!rows.iter().any(|r| r.recommended));
        recommend(&mut rows, false, Some(64 * GIB));
        assert_eq!(
            rows.iter().find(|r| r.recommended).unwrap().id,
            SpeechEngine::Nemotron
        );
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn qwen_warning_states_the_measured_share_below_sixteen_gib_and_unknown_memory() {
        assert_eq!(qwen_memory_warning(Some(16 * GIB)), None);
        let small = qwen_memory_warning(Some(8 * GIB)).unwrap();
        assert!(
            small.contains("8GB") && small.contains("4.5GB") && small.contains("절반 이상을"),
            "{small}"
        );
        assert!(qwen_memory_warning(Some(12 * GIB))
            .unwrap()
            .contains("약 34%를"));
        assert!(qwen_memory_warning(None)
            .unwrap()
            .contains("확인하지 못했습니다"));
    }
}
