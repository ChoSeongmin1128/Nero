# Nero 배포

현재 앱은 `apps/desktop`의 Tauri 구현이다. 빌드, 서명, 공증과 설치 명령은 [desktop 안내](../apps/desktop/README.md#version-and-macos-distribution)를 따른다.

- macOS는 Developer ID 서명과 Apple 공증을 거친 앱/DMG를 사용한다. 설치 대상은 `/Applications/Nero.app`이다.
- 정본 설치 도구는 `apps/desktop/scripts/install-macos.py`다. 녹음과 후속 처리가 끝난 뒤 정상 종료하고, 검증된 앱을 같은 파일시스템에서 이동한다.
- 현재 앱 식별자는 `io.github.choseongmin1128.nero.desktop`이다. 이전 개발 식별자의 개인 자료는 명시적인 이전 절차와 영수증을 통해 보존한다. 일반 조회에서 빈 새 저장소를 초기화하거나 이전 자료를 암묵적으로 이관하지 않는다.
- 모델, 원음, 전사, 사용자 토큰, 인증서 개인키와 공증 키를 배포본이나 제품 Git에 넣지 않는다. 모델은 선택한 기능 준비 시 내려받아 검증한다.
- Windows 설치 산출물과 Windows 실제 실행은 macOS 배포와 별도로 검증한다. macOS DMG는 Windows 설치파일이 아니다.
- 현재/직전 공증 DMG와 작은 영수증만 유지한다. 모델/원음 전체 스냅샷, 중복 앱, 업로드 임시 ZIP을 릴리스별로 쌓지 않는다.

버전 규칙은 [versioning.md](versioning.md), 소스 커밋과 실제 배포 상태는 [release-history.md](release-history.md)를 따른다. 이전 Swift 배포 문서는 [격리된 원본](../legacy/swift/docs/distribution.md)으로 보존한다.
