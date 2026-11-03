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

공식 Claude Code를 통한 외부 로그인은 같은 운영체제 사용자의 Claude Code 계정을 공유합니다. Nero는 이 자격 증명을 복사하거나 임의의 `HOME`으로 전환하지 않습니다. 이 외부 로그인은 개발 앱과 운영 앱 각각의 데이터 및 Nero 전용 비밀 저장소와 별개입니다.

## Windows 설치 프로그램 서명 준비

`Desktop checks`의 수동 실행에서 `build_windows_installer`와 `sign_windows_installer`를 함께 선택하면 기본 브랜치의 빌드 산출물을 SignPath에 전달합니다. 원본 저장소의 기본 브랜치 실행만 허용하며 PR과 포크는 서명 비밀을 사용하지 않습니다. 설정 누락, 서명 거부, 검증 실패는 실패로 처리합니다.

SignPath 신청과 GitHub 신뢰 빌드 연결, 수동 승인 정책을 먼저 설정해야 합니다. 저장소 secret `SIGNPATH_API_TOKEN`에는 해당 정책의 제출 권한이 있는 토큰을, repository variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`, `SIGNPATH_CERTIFICATE_THUMBPRINT`에는 승인된 설정과 서명 인증서의 SHA-1 지문을 넣습니다. 인증서 변경 시 지문을 함께 갱신합니다. SignPath 기본 artifact configuration은 업로드 ZIP의 `Nero-setup.exe`에 Authenticode 서명을 적용하고 제품명과 버전 메타데이터 제한 및 수동 승인 조건을 강제해야 합니다. Foundation 가입 조건과 정책은 실제 신청 전에 확인해야 합니다.

워크플로는 서명 결과의 Windows 신뢰 상태, 인증서 지문, 타임스탬프를 확인하고 `windows-signed-installer` artifact로만 보관합니다. 이는 NSIS 설치 프로그램 서명이며 설치 파일에 포함된 개별 실행 파일 서명을 의미하지 않습니다. 릴리스나 업데이트 피드를 게시하지 않습니다. 배포에 사용하려면 최종 서명된 바이트로 updater `.sig`를 새로 생성하고 별도의 업데이트 검증을 완료해야 합니다.
