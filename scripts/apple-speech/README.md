# macOS 기본 전사

macOS 26 이상 Apple silicon 앱에서 `SpeechAnalyzer`를 사용한다. Nero 전용 ASR 가중치는 받지 않는다. 한국어 시스템 언어 자산이 없으면 명시적인 준비 작업에서 Apple이 관리하는 자산을 다운로드할 수 있다. 이 자산의 용량과 갱신은 macOS가 관리하므로 Nero 모델 크기 0이 다운로드가 없다는 뜻은 아니다.

지원 여부는 실행 환경의 API 결과로 확인한다. `SpeechTranscriber.isAvailable`과 `supportedLocale(equivalentTo:)`가 한국어를 지원하면 `SpeechTranscriber`를 선택한다. 그렇지 않으면 한국어를 지원하는 `DictationTranscriber`를 선택할 수 있다. 선택은 준비 전에 결정해 preference, capture session, job에 저장한다. 준비와 전사에는 이 선택을 필수 인자로 전달하며, 실행 실패 때 다른 전사기로 바꾸지 않는다. 시스템 서비스 접근이 제한된 프로세스의 지원 조회는 일반 앱 실행과 다를 수 있다.

- `status [variant]`: 기기/한국어 지원과 시스템 자산 설치 상태만 조회한다. 다운로드나 자산 예약을 하지 않는다.
- `prepare <variant> <output>`: 필요한 경우에만 locale 예약과 `AssetInstallationRequest.downloadAndInstall()`을 실행한다.
- `transcribe <variant> <input> <output>`: 설치된 자산만 사용한다. 준비되지 않았으면 실패하며 자산을 다운로드하지 않는다.

`SpeechAnalyzer`의 파일 입력 완료, 읽은 프레임 수, 명시적 finalize 성공을 확인한 후 최종 phrase 결과를 저장한다. 원본 결과 JSON, 입력/실행기/결과 해시, macOS 버전과 전사기 종류를 보존한다. Apple이 관리하는 자산의 개별 가중치 해시나 버전은 확인했다고 주장하지 않는다.

전사는 녹음 종료 후 수행한다. 최종 결과의 원래 구간과 문자열을 그대로 사용하며 단어 시각이나 confidence를 만들지 않는다. 선택적인 화자 분석은 해당 구간의 기존 majority-coverage 규칙을 사용한다. 여러 화자가 겹친 구간은 미지정으로 남긴다. 화자 분석을 끄면 모델 준비, live 화자 stream과 최종 분석을 실행하지 않는다. 켜도 전사를 먼저 저장하며 화자 단계 실패/취소로 저장된 전사를 버리지 않는다.

`build.py`는 설치된 Swift 컴파일러만 사용한다. 개발용 helper는 Cargo 출력 폴더에 생성하고, 배포 준비는 `verify-native.mjs`가 `.generated/native/apple-speech/`에 생성한다. Swift module cache는 Cargo target 아래에 둔다. 이 과정에서 시스템 언어 자산이나 모델 가중치를 받지 않는다.

API 계약: [SpeechAnalyzer](https://developer.apple.com/documentation/speech/speechanalyzer), [AssetInventory](https://developer.apple.com/documentation/speech/assetinventory), [DictationTranscriber](https://developer.apple.com/documentation/speech/dictationtranscriber), [WWDC25 277](https://developer.apple.com/videos/play/wwdc2025/277/).
