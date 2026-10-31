# 설치와 배포

설치 파일은 [GitHub Releases](https://github.com/ChoSeongmin1128/Nero/releases)에서 제공합니다.
각 버전의 지원 환경, 변경 사항과 알려진 제약을 함께 확인하세요.

| 플랫폼 | 설치 | 배포 검증 |
| --- | --- | --- |
| macOS / Apple Silicon | DMG를 열어 Nero를 응용 프로그램 폴더로 이동 | Developer ID 서명과 Apple 공증 |
| Windows / x64 | 현재 사용자용 NSIS 설치 프로그램 실행 | 앱 내 업데이트 서명 검증. 현재 Windows 게시자 서명은 제공하지 않음 |

수동 설치/업데이트 전에는 녹음과 처리가 끝난 뒤 Nero를 정상 종료하세요.
설치 프로그램은 실행 중인 앱을 강제 종료하지 않습니다.
앱 업데이트는 원음과 모델을 설치 파일에 넣거나 새로 복사하는 방식으로 처리하지 않습니다.
모델은 앱에서 필요한 기능을 준비할 때 별도로 내려받습니다.

앱 내 업데이트의 다운로드, 서명 검증과 복구는 [앱 업데이트](app-updates.md)를 확인하세요.
소스에서 빌드하거나 배포 파일을 만드는 방법은 [개발 안내](../apps/desktop/README.md#version-and-macos-distribution)에 있습니다.
버전과 데이터 보존 계약은 [버전 관리](versioning.md)를 따릅니다.
