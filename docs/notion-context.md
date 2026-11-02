# Notion 참고 자료

Nero의 Notion 연결은 사용자가 지정한 페이지를 대화의 외부 참고 자료로 읽는다.
회의 발언, 음성 기록, 화자 이름이나 목소리 신원에 이 본문을 섞어 저장하지 않는다.
대화 서비스는 `context_bundle(source_ids)`의 `kind: external_reference`와 각 문서의
출처를 유지해 모델에 전달한다. 문서 안의 지시, 도구 호출이나 외부 링크는 실행하지 않는다.

## 연결

- 공식 `https://mcp.notion.com/mcp`를 사용한다. API 키 입력과 다른 앱의 연결 설정은 사용하지 않는다.
- 보호 자원 메타데이터와 OAuth 서버 메타데이터를 읽고 issuer와 Notion 소유 HTTPS endpoint를 검증한다.
- Public client DCR, PKCE S256, 임의 state, loopback callback과 `resource`를 사용한다.
  callback은 정확한 host/path/state를 확인하며, `iss`가 있거나 서버가 필수라고 명시하면 issuer도 확인한다.
- OAuth의 기본 grant와 별개로 Nero가 노출하는 동작은 페이지 읽기로 제한한다.
  MCP의 `notion-fetch`만 고정 호출한다. 검색, 쓰기, 첨부 다운로드, 다른 서비스 조회는 제공하지 않는다.
- 연결 확인은 token 교환과 MCP bootstrap을 완료한 뒤 새 보안 저장소 슬롯과 연결 메타데이터를 기록한다.
  취소나 완료 전 오류는 이전 연결과 저장한 문서를 보존한다.
- macOS Keychain/Windows Credential Manager의 Nero 전용 `notion-context` 항목에 client/token 정보를 저장한다.
  refresh token이 바뀌면 access/refresh token을 같은 보안 저장소 객체에 저장한다.
  `invalid_grant`는 자동 반복하지 않고 재연결이 필요하다고 알린다.
- 연결 해제는 Nero 전용 자격 증명만 제거한다. 이미 읽은 문서와 이전 대화의 출처는 보존한다.

## MCP

Notion custom-client 가이드의 `initialize` → `notifications/initialized` 흐름을 구현한다.
호환 프로필은 `2025-11-25`와 `2025-06-18`이다. 협상된 버전 외의 응답은 오류로 중단한다.
MCP `2026-07-28`의 `server/discover`와 요청별 협상은 이 프로필에 포함되지 않는다.

각 POST는 JSON-RPC 요청 하나와 `Accept: application/json, text/event-stream`을 보낸다.
협상 버전은 `MCP-Protocol-Version`에, bootstrap 응답의 opaque session ID는 이후
`Mcp-Session-Id`에 보낸다. JSON과 SSE 응답을 모두 읽는다. 서버의 instructions,
sampling, elicitation이나 다른 server request는 실행하지 않는다.

`tools/list`는 `notion-fetch` 지원 여부 확인에만 사용한다. 페이지 읽기에는 정규화한
UUID를 `notion-fetch`의 `id`로 전달한다. 본문에 등장하는 하위 페이지나 누락된 subtree는
자동으로 따라가지 않는다. 누락 여부와 알려진 개수는 출처 메타데이터에 남긴다.

## 범위와 저장

| 항목 | 한계 |
| --- | --- |
| 한 요청의 명시 페이지 | 최대 8개, 같은 ID는 중복 제거 |
| 문서 본문 | 문서당 256 KiB |
| 대화 참고 묶음 | 본문 합계 512 KiB |
| HTTP 응답 | 2 MiB |
| HTTP 요청 | 최대 30초 |
| 문서 읽기 묶음 | 최대 60초 |
| 인증 대기 | 최대 10분 |

진행 알림이 와도 전체 시간 한계를 늘리지 않는다. 취소/시간 초과는 해당 요청을 중단하고,
가능하면 MCP cancellation 알림을 짧은 제한 안에서 보낸다. 부분 성공 목록을 성공 응답으로
반환하지 않는다. 지원하지 않는 문서 종류나 다른 페이지의 응답은 출처로 채택하지 않는다.

문서 snapshot은 제목, URL, provider의 last-edited 값, 읽은 시각, 본문 SHA256과 실제 본문을
포함한다. 제목과 수정 시각이 응답에 없으면 null을 보존한다. 본문을 추정하거나 요약해서
원문처럼 보관하지 않는다. 읽힌 내용은 문서 전체를 보장하는 증거가 아니며, provider가
truncation을 보고하면 그 사실과 알려진 누락 개수를 보존한다.

`sourceId`는 문서 ID와 snapshot hash로 구성한 불투명한 revision ID다. 다시 읽은 문서는
새 snapshot으로 보관해 이전 대화의 원문 출처를 바꾸지 않는다. 저장본 읽기는 원격 요청이나
라이브러리 초기화/이전을 실행하지 않는다. 파일과 상위 폴더의 symlink 및 내용/hash 불일치를 거부한다.

## 공식 계약

- [Notion custom MCP client](https://developers.notion.com/guides/mcp/build-mcp-client)
- [Notion supported tools](https://developers.notion.com/guides/mcp/mcp-supported-tools)
- [MCP transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
- [MCP protocol versions](https://modelcontextprotocol.io/docs/2026-07-28/learn/versioning)
