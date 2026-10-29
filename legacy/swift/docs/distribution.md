# 앱스토어 외 배포

## 산출물

`package_app.sh release`는 실행기 / 리소스를 포함한 `.work/dist/Grove.app`을 만든다.
모델, 녹음, 전사, 계정 토큰, 서명 개인키와 공증 키는 앱에 넣지 않는다.
MOSS의 Metal 리소스, Nemotron의 모든 동적 라이브러리와 라이선스가 필요하다.
모델 서버를 운영하지 않고 공식 저장소의 검증한 고정 파일을 내려받는다.

개발 시 native workers는 `.work/native-workers`에서 준비한다.
MOSS는 `scripts/native-moss-harness/README.md`, Nemotron은 `scripts/native-nemotron/README.md`를 따른다.
이 작업은 앱 제작 단계에만 해당하며 사용자 Mac에는 해당 도구가 필요하지 않다.

## 서명 / 공증

Mac App Store 배포 인증서가 아닌 Developer ID Application 인증서와 개인키를 사용한다.
같은 개발자의 인증서는 여러 앱에 재사용할 수 있다. Apple API 키 또는 이미 등록한
notarytool Keychain 프로필로 공증한다. 앱별 CloudKit 프로비저닝 파일이나 다른 앱의
업데이트 서명 키는 Grove에 사용하지 않는다.

```bash
GROVE_SIGN_IDENTITY=<Developer-ID-certificate-SHA1> \
GROVE_NOTARY_PROFILE=<existing-notarytool-profile> \
./scripts/sign-notarize.sh
```

프로필 대신 `GROVE_NOTARY_KEY_PATH`, `GROVE_NOTARY_KEY_ID`, `GROVE_NOTARY_ISSUER_ID`를 사용할 수 있다.
키 값이나 인증서 암호를 저장소에 작성하지 않는다. 실제 계정과 경로는 비공개 인계 문서에서 확인한다.

스크립트는 내부 Mach-O, MOSS 실행 번들, 최상위 앱 순서로 Developer ID / Hardened Runtime /
보안 타임스탬프를 적용한다. 공증 Accepted 이후 티켓을 staple하고 Gatekeeper 검사를 한다.
최종 ZIP은 staple 이후 다시 생성한다. 결과와 로그는 `.work`에 둔다.
공증 실패를 경고 우회나 quarantine 삭제로 해결하지 않는다.

## 검증

- `swift test --scratch-path .work/swift-build --jobs 2`
- `python3 -m unittest discover -s tests -v`
- 공식 파일을 사용한 다운로드 중단 / 이어받기 / SHA256 검사
- 패키징된 실행기로 실제 파일 가져오기, 전사, 화자 분리, 편집 저장 / 재열기
- Mac 기본 전사 경로의 실제 결과와 화자 미분리 표시
- 첫 실행 준비 검사와 준비 완료 기록
- Developer ID, Hardened Runtime, 모든 내부 코드 서명, notarization, staple, Gatekeeper
- 설치 경로 `/Applications/Grove.app`에서 실행 / UI 확인

개발 도구가 없는 별도 Mac에서의 첫 다운로드와 실제 전사는 독립 검증 항목이다.
현재 Mac에서 제한된 PATH로 실행한 검사를 별도 Mac 검증이라고 기록하지 않는다.
Calendar 권한과 실제 계정 일정, Notion 토큰과 실제 부모 페이지가 필요한 검증은
모의 응답 검사와 구분한다. 개인 녹음 기반 결과와 화면은 비공개 인계에만 적는다.

설치 전 실행 중인 Grove의 녹음 / 처리 / 저장 상태를 확인한다. 원본 데이터를 유지하며,
필요한 기존 번들은 복구 가능하게 보관한다. 설치 확인 뒤 생성한 중복 앱은 휴지통으로 보낸다.

Apple 공식 안내: [Developer ID](https://developer.apple.com/developer-id/),
[macOS 공증](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## Notion OAuth

공식 Notion MCP의 공개 클라이언트 등록과 PKCE를 앱에서 처리한다. 앱 복귀 URL scheme은
Info.plist에 선언하고, 인증 창의 state / issuer / 주소 / 만료 검증을 유지한다.
Grove용 OAuth 비밀키나 별도 서버 설정을 배포본에 넣지 않는다.
MCP와 REST의 토큰을 서로 바꿔 사용하지 않는다. 공개 클라이언트 등록 정보와 토큰 쌍은
해당 Mac의 Keychain에 보관하며, 앱 업데이트로 재등록하거나 삭제하지 않는다.
외부 Notion 권한 승인과 실제 페이지 변경은 UI에서 사용자가 선택한 경우에만 실행한다.
