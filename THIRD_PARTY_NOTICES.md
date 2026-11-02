# 서드파티 고지

현재 Nero 데스크톱 설치 파일에 포함되는 구성 요소와 별도로 다운로드하는 모델의 고지입니다.
아래 경로는 앱의 resource directory 기준이며, macOS에서는 `Nero.app/Contents/Resources/` 아래에 있습니다.
Windows에서는 설치된 앱의 resource directory에 같은 상대 경로를 사용합니다.
모델 가중치는 설치 파일에 포함하지 않으며, 사용자가 모델 준비를 선택했을 때 앱 데이터 디렉터리에 저장합니다.

## Rust 실행 코드

`native/licenses/distribution/manifest.json`은 해당 플랫폼의 `nero-desktop`과 `nero-cli`에 필요한
Cargo 실행 의존성, 버전, 라이선스 선언과 동봉한 고지 파일의 SHA256을 기록합니다.
원문은 `native/licenses/distribution/rust/<crate>-<version>/`에 포함합니다.
빌드 도구, 개발 전용 의존성과 컴파일 시 실행되는 procedural macro는 이 실행 코드 목록에서 제외합니다.
실행 라이브러리에 포함되는 native code의 별도 고지는 해당 crate의 고지와 함께 보존합니다.

일부 upstream crate archive는 라이선스 선언만 포함하고 전문을 생략합니다.
이 경우 `apps/desktop/licenses/rust-supplements/manifest.json`에 crate archive의 고정 소스 커밋,
원문 출처와 해시를 기록하고 보충 전문을 동봉합니다.
objc2 계열의 upstream 선언과 표준 MIT 전문은 함께 보존하며, 패키지 작성자 정보도 실행 의존성 목록에 기록합니다.
Symphonia의 수정된 AAC 소스는 `vendor/symphonia-codec-aac/`에 보존합니다.

## 화면 구성과 글꼴

- Pretendard Variable 1.3.9: Copyright (c) 2021, Kil Hyung-jin. SIL Open Font License 1.1.
  원본 글꼴과 전문은 내장 화면 자산 `shared/vendor/pretendard/`에 포함합니다.
  출처: https://github.com/orioncactus/pretendard/tree/v1.3.9
- Lucide 아이콘과 React, Base UI, React DayPicker 등 npm 실행 의존성의 버전과 전문은
  내장 화면 자산 `shared/licenses/manifest.json` 및 인접 고지 파일에 포함합니다.
  직접 보관한 Lucide SVG의 전문은 `shared/vendor/lucide/LICENSE`에 포함합니다.
- shadcn/ui의 전문은 `shared/licenses/shadcn-ui.txt`에 포함합니다.

## 음성 처리와 요약 실행기

- Nemotron 음성 인식과 화자 구분은 NVIDIA NeMo-Speech.cpp 0.2.0 기반 실행기를 공유합니다.
  플랫폼별 수정 소스, runtime identity와 archive pin은 `scripts/native-diarization/runtime.json`이 정본입니다.
  라이선스와 NOTICE는 `native/diarization/<backend>/share/licenses/`에 포함합니다.
  출처: https://github.com/NVIDIA/NeMo-Speech.cpp
- 회의 요약 실행기는 llama.cpp b11734입니다. 고정 소스와 archive pin은
  `scripts/native-summary/runtime.json`, MIT 전문은 `native/summary/LICENSE-llama.cpp.txt`에 있습니다.
  출처: https://github.com/ggml-org/llama.cpp/tree/f3fb2fcdb
- Whisper 실행기는 whisper.cpp b5454, commit `d1be6fde11ac6e0407606b4e42fe72d34add8037`입니다.
  MIT 전문은 `native/whisper/share/licenses/whisper.cpp/LICENSE`에 포함합니다.
  Windows Vulkan 빌드의 Vulkan-Headers와 SPIRV-Headers 전문도 같은 `share/licenses/` 아래에 포함합니다.
  Vulkan SDK, CMake와 Ninja는 빌드 도구이며 사용자에게 설치하지 않습니다.
  출처: https://github.com/ggml-org/whisper.cpp/tree/d1be6fde11ac6e0407606b4e42fe72d34add8037
- WAV 준비에 사용하는 rubato와 numeric 의존성의 전문은 `native/licenses/portable-wav/`에 포함합니다.
  파일 가져오기에 사용하는 Symphonia 0.6.1(MPL 2.0)과 추가 의존성의 전문 및 고정 소스 archive 링크는
  `native/licenses/audio-decode/manifest.json`과 인접 파일에 포함합니다.
  수정된 AAC decoder는 지원하지 않는 in-band SBR 입력을 엄격히 거부합니다.
  수정 소스와 변경 설명은 `vendor/symphonia-codec-aac/`에 있습니다.

Windows 실행기에 함께 포함되는 Microsoft C++ runtime은 해당 도구 공급자의 재배포 조건을 따릅니다.
macOS와 Windows의 시스템 프레임워크 및 GPU driver는 설치 파일에 포함하지 않습니다.

## 별도로 준비하는 모델

모델 라이선스 검토 정본은 `apps/desktop/licenses/models/manifest.json`입니다.
고정 repository/revision 및 파일 pin을 실제 모델 설정과 대조하고, 공급자의 해당 revision 모델 카드와
라이선스 전문 출처 및 SHA256을 함께 기록합니다. 런타임 모델 설정과 readiness marker는 이 고지와 별개입니다.
설치 파일에는 같은 자료를 `native/licenses/distribution/models/` 아래에 포함합니다.

| 모델 | 이용 조건 | 모델 pin |
| --- | --- | --- |
| NVIDIA Nemotron 3.5 ASR Streaming 0.6B Q8 | OpenMDW 1.1 | `config/models/nemotron-asr.json` |
| NVIDIA Nemotron 3 Diarization Q8 | OpenMDW 1.1 | `apps/desktop/models/diarization.json` |
| Google Gemma 4 E4B IT QAT Q4_0 GGUF | Apache License 2.0 | `apps/desktop/models/summary.json` |
| Whisper large-v3-turbo Q5_0 | MIT | `config/models/whisper-turbo.json` |

OpenMDW 1.1 원문: https://openmdw.ai/license/1-1/

Gemma 4 공급자 라이선스 안내: https://ai.google.dev/gemma/docs/gemma_4_license

Whisper 모델 카드: https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md
