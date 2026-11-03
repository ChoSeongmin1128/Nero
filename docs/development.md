# 개발 실행

`apps/desktop`에서 실행합니다.

```sh
npm ci
npm run tauri -- dev
```

`npm run dev`는 Vite 웹 화면만 실행합니다. 네이티브 개발 앱은 `Nero Dev`이며 식별자는 `io.github.choseongmin1128.nero.desktop.dev`입니다. 일반 Nero와 앱 데이터, 템플릿, 모델 경로, 연결 디렉터리 및 Nero 자체 비밀 저장소가 분리됩니다. 모델 가중치나 운영 자격 증명을 개발 데이터에 복사하지 않습니다. 개발 검증은 합성 입력을 사용하며, 실제 모델 검증이 필요한 경우 기존 검증된 모델 파일을 읽기 전용으로 지정할 수 있는 경로를 사용합니다. 개발 앱은 공개 업데이트를 확인하거나 설치하지 않으며 외부 Claude/ChatGPT의 Nero MCP 등록도 변경하지 않습니다.

환경은 컴파일 프로필과 `development` 기능으로 결정합니다. 실행 시 환경변수로 전환하지 않습니다. Tauri 설정의 앱 이름/식별자가 컴파일된 환경과 다르면 플러그인, 창, 저장소를 초기화하기 전에 종료합니다. `cargo run`으로 기본 운영 설정을 그대로 사용하는 디버그 실행은 거부됩니다. Dev CLI에서도 운영/Grove 데이터 경로를 `--data-dir`로 지정할 수 없습니다.

```sh
npm run tauri -- build --debug --bundles app --ci -- --locked
npm run tauri -- build --features development --bundles app --ci -- --locked
npm run tauri -- build --bundles app --ci -- --locked
```

첫 번째는 디버그 개발 빌드, 두 번째는 릴리스 최적화 개발 빌드, 세 번째는 일반 운영 빌드입니다. Windows에서는 `app` 대신 `nsis`를 사용합니다. 래퍼는 개발 프로파일에 개발 설정과 기능을 함께 적용하고 모순되는 `NERO_BUILD_ENV`나 사용자 지정 설정을 거부합니다. 개발용 에이전트는 별도로 빌드하며 서명된 운영 에이전트를 재사용하지 않습니다.

공식 Claude Code를 통한 외부 로그인은 같은 운영체제 사용자의 Claude Code 계정을 공유합니다. Nero는 이 자격 증명을 복사하거나 임의의 `HOME`으로 전환하지 않습니다. 이 외부 로그인은 개발 앱과 운영 앱 각각의 데이터 및 Nero 전용 비밀 저장소와 별개입니다.
