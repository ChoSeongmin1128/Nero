# 개발 실행

`apps/desktop`에서 실행합니다.

```sh
npm ci
npm run tauri -- dev
```

`npm run dev`는 Vite 웹 화면만 실행합니다. 네이티브 개발 앱은 `Nero Dev`이며 식별자는 `io.github.choseongmin1128.nero.desktop.dev`입니다. 일반 Nero와 앱 데이터, 템플릿, 모델 경로, 연결 디렉터리 및 Nero 자체 비밀 저장소가 분리됩니다. 모델 가중치나 운영 자격 증명을 개발 데이터에 복사하지 않습니다. 개발 검증은 합성 입력을 사용하며, 실제 모델 검증이 필요한 경우 기존 검증된 모델 파일을 읽기 전용으로 지정할 수 있는 경로를 사용합니다. 개발 앱은 공개 업데이트를 확인하거나 설치하지 않으며 외부 Claude/ChatGPT의 Nero MCP 등록도 변경하지 않습니다.

환경은 컴파일 프로필과 `development` 기능으로 결정합니다. 실행 시 환경변수로 전환하지 않습니다. Tauri 설정의 앱 이름/식별자가 컴파일된 환경과 다르면 플러그인, 창, 저장소를 초기화하기 전에 종료합니다. `cargo run`으로 기본 운영 설정을 그대로 사용하는 디버그 실행은 거부됩니다. Dev CLI에서도 운영/Grove 데이터 경로를 `--data-dir`로 지정할 수 없습니다.

개발 프로필은 최소 디버그 정보를 사용하고 후처리 strip을 비활성화합니다. macOS의 동적 라이브러리 정렬 오류를 피하기 위한 설정이며, 전체 디버그 정보를 만들거나 빌드된 바이너리를 사후 수정하지 않습니다.

```sh
npm run tauri -- build --debug --bundles app --ci -- --locked
npm run tauri -- build --features development --bundles app --ci -- --locked
npm run tauri -- build --bundles app --ci -- --locked
```

첫 번째는 디버그 개발 빌드, 두 번째는 릴리스 최적화 개발 빌드, 세 번째는 일반 운영 빌드입니다. Windows에서는 `app` 대신 `nsis`를 사용합니다. 래퍼는 개발 프로파일에 개발 설정과 기능을 함께 적용하고 모순되는 `NERO_BUILD_ENV`나 사용자 지정 설정을 거부합니다. 개발용 에이전트는 별도로 빌드하며 서명된 운영 에이전트를 재사용하지 않습니다.

macOS의 개발용 `--bundles app` 빌드는 유효한 `Apple Development` 인증서가 하나 있으면 완성된 Nero Dev 번들을 그 인증서로 서명합니다. 여러 인증서가 있거나 다른 로컬 인증서를 쓸 때는 해당 실행에 `NERO_DEV_SIGN_IDENTITY`를 지정합니다. 앱 식별자는 개발용으로 고정하고, 패키지 작업자와 검증 영수증의 바이트는 변경하지 않습니다. 운영 배포 서명/공증은 별도 절차입니다. `--no-sign`과 명시적인 임시 서명(`-`) 선택도 존중합니다. 인증서가 없으면 기존 임시 서명을 유지하고 경고합니다. 인증서를 만들거나 시스템 신뢰/키체인 접근 제어를 변경하지 않습니다.

키체인 실기 검사는 이 서명된 Dev 번들에서 수행합니다. 임시 서명의 코드 해시가 바뀌면 이전 앱의 키체인 허용이 새 빌드에 이어지지 않을 수 있습니다. 인증서 기반 서명으로 처음 전환할 때는 OS에서 한 번 접근 허용이 필요할 수 있습니다. 이후에도 인증서나 앱 식별자를 바꾸면 다시 승인해야 할 수 있습니다. `tauri dev`의 번들 없는 실행과 서명하지 않은 테스트 실행은 이 보존을 보장하지 않습니다.

공식 Claude Code를 통한 외부 로그인은 같은 운영체제 사용자의 Claude Code 계정을 공유합니다. Nero는 이 자격 증명을 복사하거나 임의의 `HOME`으로 전환하지 않습니다. 이 외부 로그인은 개발 앱과 운영 앱 각각의 데이터 및 Nero 전용 비밀 저장소와 별개입니다.

## Windows 검증

Windows용 빌드와 설치 검사는 Windows의 MSVC/WebView2 및 `--bundles nsis` 경로를 사용합니다. Dev 설치 프로그램의 이름, 식별자, 설치/제거 경로와 패키지 내부 CLI를 검사한 결과와 실제 Dev 설치/GUI/한국어 IME/로그인/마이크 검증은 구분합니다. macOS 또는 Linux 검사 통과로 Windows 실기기 검증을 대체하지 않습니다. 테스트 순서와 설치 프로그램 제약은 [데스크톱 빌드](../apps/desktop/README.md#windows-installation-and-updates), 음성 실행 계약은 [Windows 전사 모델](windows-speech-models.md)을 확인하세요.

## 모델 평가와 산출물

[로컬 모델 평가 도구](../scripts/model-evaluation/README.md)는 앱과 분리된 연구용 CLI입니다. `plan`/`doctor`는 사전 조건을 검사하고 명시적인 `run`만 모델을 실행합니다. 모델과 평가용 음성, 정답, 독립 문맥의 실제 경로를 설정해야 하며 예시 설정이나 합성 단위 테스트만으로 실제 추론 준비가 완료되지는 않습니다. 현재 실행기는 macOS/Linux의 POSIX 프로세스 그룹을 사용합니다. Mini의 어댑터는 Linux/CUDA 경로이며 Mac 경로와 Windows 네이티브 평가 실행기는 구현되지 않았습니다.

모델 가중치는 공통 로컬 경로에서 읽고 실험마다 복제하지 않습니다. 평가 원본과 결과는 Git에서 제외되는 비공개 경로에 보존합니다. 재생성 가능한 옛 빌드와 캐시는 사용 여부 및 참조를 확인한 후 정리하며, 현재 Dev 앱, 원본/모델/회의 데이터와 고유 검증 근거는 유지합니다. 공통 Cargo 대상은 `.work/rust-target`입니다. 모델 실행과 무거운 빌드/검사는 한 번에 하나만 수행합니다.

## Windows 설치 프로그램 서명 준비

`Desktop checks`의 수동 실행에서 `build_windows_installer`와 `sign_windows_installer`를 함께 선택하면 기본 브랜치의 빌드 산출물을 SignPath에 전달합니다. 원본 저장소의 기본 브랜치 실행만 허용하며 PR과 포크는 서명 비밀을 사용하지 않습니다. 설정 누락, 서명 거부, 검증 실패는 실패로 처리합니다.

SignPath 신청과 GitHub 신뢰 빌드 연결, 수동 승인 정책을 먼저 설정해야 합니다. 저장소 secret `SIGNPATH_API_TOKEN`에는 해당 정책의 제출 권한이 있는 토큰을, repository variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`, `SIGNPATH_CERTIFICATE_THUMBPRINT`에는 승인된 설정과 서명 인증서의 SHA-1 지문을 넣습니다. 인증서 변경 시 지문을 함께 갱신합니다. SignPath 기본 artifact configuration은 업로드 ZIP의 `Nero-setup.exe`에 Authenticode 서명을 적용하고 제품명과 버전 메타데이터 제한 및 수동 승인 조건을 강제해야 합니다. Foundation 가입 조건과 정책은 실제 신청 전에 확인해야 합니다.

워크플로는 서명 결과의 Windows 신뢰 상태, 인증서 지문, 타임스탬프를 확인하고 `windows-signed-installer` artifact로만 보관합니다. 이는 NSIS 설치 프로그램 서명이며 설치 파일에 포함된 개별 실행 파일 서명을 의미하지 않습니다. 릴리스나 업데이트 피드를 게시하지 않습니다. 배포에 사용하려면 최종 서명된 바이트로 updater `.sig`를 새로 생성하고 별도의 업데이트 검증을 완료해야 합니다.

### 공통 모델 재사용

Dev 검증은 모델을 복사하지 않고 명시적 읽기 전용 경로를 주입할 수 있다. Dev 데이터의 `preferences/model-source.json`에 `{"schemaVersion":1,"root":"/absolute/existing-library"}`를 두면 로컬 준비 모델이 없는 경우 그 경로의 고정 전사/화자 모델 디렉터리를 조회한다. 요약 모델은 이 경로 주입 대상이 아니다. 해당 보관함을 열거나 자격 증명을 읽지 않으며 공통 모델의 검증 표시/가중치/설정을 쓰거나 정리하지 않는다. 실제 사용 전 파일 해시를 검사한다. 이 옵션은 development 프로필에서만 읽으며 일반 사용자의 자동 모델 탐색이나 설치 정책이 아니다.
