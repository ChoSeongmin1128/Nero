# 음성 파일 가져오기

macOS와 Windows는 같은 음성 파일 선택 목록을 사용합니다. 확장자만 바꿔서는
지원하지 않는 인코딩을 가져올 수 없습니다. 원본 파일은 그대로 보관하고,
재생과 음성 분석에는 별도의 mono/16kHz/PCM16 WAV를 사용합니다.

| 파일 | Windows 인코딩 지원 | macOS 처리 |
|---|---|---|
| WAV | 정수 PCM 8/16/24/32bit, float32 | 같은 공통 변환기 |
| M4A | AAC-LC, ALAC | 기존 AVFoundation 변환기 |
| MP3 | MPEG Layer III | 기존 AVFoundation 변환기 |
| AAC | ADTS의 AAC-LC | 기존 AVFoundation 변환기 |
| AIFF/AIF, CAF | PCM | 기존 AVFoundation 변환기 |
| FLAC | FLAC | 공통 디코더 |
| Ogg | Vorbis | 공통 디코더 |

Windows에서는 HE-AAC와 Opus를 지원하지 않습니다. 영상 파일, 여러 오디오
트랙을 포함한 파일, 처리 중 채널/샘플레이트가 바뀌는 파일도 가져오기 대상이
아닙니다. 지원하지 않는 파일은 PCM WAV로 변환해 가져올 수 있습니다.

변환 중 취소, 읽기 오류, 디코더 오류, 저장 오류가 발생하면 기존 회의는
변경하지 않습니다. 완성된 PCM만 새 경로에 게시하며 기존 파일을 덮어쓰지
않습니다. 원본/PCM의 SHA-256과 정확한 PCM 프레임 수를 분석 결과에 연결하고
저장 직전에 다시 확인합니다.

길이를 명시한 컨테이너는 디코딩한 프레임 수를 그 길이와 대조합니다. ADTS는
비트레이트 기반 길이 추정치를 쓰지 않고 패킷 경계를 확인해 직접 셉니다.
길이나 종료 표식이 없는 원시 스트림은 완전한 패킷 사이에서 잘렸는지 판별할
수 없습니다. 길이 태그 없는 MP3는 마지막 불완전한 패킷도 디코더가 종료로
취급할 수 있습니다. 이런 스트림의 원본이 완전하다는 보증을 만들지 않습니다.

## 구현 책임

`apps/desktop/src-tauri/src/speech_audio/`는 오디오 입력과 PCM 생성만 담당합니다.

| 파일 | 책임 |
|---|---|
| `formats.rs` | 파일 선택 목록과 가져오기 확장자 정책 |
| `import.rs` | 원본을 작업 폴더로 안전하게 복사, 원본 제목 추출 |
| `platform.rs` | WAV/공통 디코더/macOS 변환기 선택 |
| `decode.rs` | 컨테이너/단일 오디오 트랙 선택과 스트리밍 디코딩 |
| `framing.rs` | 원시 ADTS의 정확한 패킷/프레임 확인 |
| `wav.rs` | WAV 스펙 검증과 샘플 읽기 |
| `normalize.rs` | 실시간 녹음/파일 가져오기가 공유하는 리샘플링 |
| `output.rs` | 채널 평균, PCM 기록, 완료/동기화 후 원자적 게시 |
| `mod.rs` | 분석 입력 계약과 원본/PCM 무결성 확인 |

작업 진행, 취소 상태, 회의 연결과 저장 경합 확인은 `processing.rs`에 남습니다.
Core는 회의 데이터와 원본/분석 파일 보관을 담당하며 컨테이너를 디코딩하지
않습니다. 디코더 선택이 음성 인식 모델 선택이나 GPU 선택에 영향을 주지 않습니다.

## 검증

일반 네이티브 테스트는 AIFF/CAF 합성 입력, WAV 샘플 형식과 샘플레이트,
다운믹스, 안티앨리어싱, 취소/저장 실패와 원본 보존을 검사합니다.

실제 인코더로 만든 합성 파일도 별도로 검사할 수 있습니다. 작업용 Python 환경에
`soundfile==0.13.1`, `numpy==2.3.4`를 준비하고 macOS에서 다음 생성기를 실행합니다.
이 도구와 인코더는 앱 실행에 필요하지 않습니다. 생성된 음성 파일은 Git에 넣지 않습니다.

```sh
python scripts/audio-prepare/make-fixtures.py <new-temporary-directory>
NERO_AUDIO_FIXTURES=<fixture-directory> cargo test -p nero-desktop --locked \
  encoded_codec_matrix -- --ignored
```

동일한 생성 파일로 Windows 네이티브 테스트를 실행할 수 있습니다. macOS 어댑터
검증은 패키지의 `AudioPrepare`를 사용하며 실제 사용자 라이브러리를 열지 않습니다.

```sh
NERO_AUDIO_FIXTURES=<fixture-directory> NERO_AUDIO_RESOURCES=<app-resource-directory> \
  cargo test -p nero-desktop --locked macos_import_adapter -- --ignored
```
