# Notion 연결

이 문서는 현재 소스의 동작을 설명한다. 배포된 버전의 기능은 릴리스 안내를 확인한다.

Nero의 Notion 연결은 사용자가 지정한 페이지를 회의 자료 또는 대화의 외부 참고 자료로 읽는다.
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
  macOS는 제한된 단일 항목을 사용하고, Windows는 1024-byte chunk와 검증된 generation index로 항목 크기 제한을 지킨다. 이전 macOS 청크 항목은 전체 내용을 확인한 뒤에만 단일 항목으로 이전하며 원래 항목을 보존한다.
  `invalid_grant`는 자동 반복하지 않고 재연결이 필요하다고 알린다.
- 연결 해제는 Nero 전용 자격 증명만 제거한다. 이전 항목 정리가 실패하면 현재 항목을 보존해 재시도할 수 있게 한다. 이미 읽은 문서와 이전 대화의 출처는 보존한다.

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
| 회의별 자료 | 최대 8개, 페이지당 참조 하나 |
| 문서 본문 | 문서당 256 KiB |
| 대화 참고 묶음 | 본문 합계 512 KiB |
| HTTP 응답 | 2 MiB |
| HTTP 요청 | 최대 30초 |
| 문서 읽기 묶음 | 최대 60초 |
| 인증 대기 | 최대 10분 |

시작한 refresh 거래는 사용자 취소와 분리된 30초 요청 한계를 사용한다. 받아들인 회전 token을
원자적으로 저장한 뒤 원래 호출에 취소를 반환한다. 호출 future가 사라져도 이미 시작한 거래의
저장은 계속된다. 종료/업데이트 준비는 새 OAuth/fetch/쓰기 진입을 닫고 대기 중 인증과 자료 읽기를
취소한 뒤 진행 중 refresh/save만 최대 30초 기다린다. 대기 callback이나 사용자 로그인을 기다리지 않는다.
준비 guard가 유지되는 동안 새 쓰기는 거부하며, 실패하거나 guard가 해제되면 새 작업을 다시 허용한다.

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

회의별 자료는 본문을 복제하지 않고 페이지와 불변 `sourceId`만 별도 저장한다. 회의에 추가하거나 새로고침할 때 사용자가 지정한 페이지만 읽는다. 새로고침에 성공하면 해당 회의의 참조를 새 snapshot으로 교체하며, 실패나 변경 충돌 시 기존 참조를 유지한다. 회의를 첨부할 때 표시되는 자료는 대화에서 개별 제외할 수 있다. 이미 선택한 대화 자료와 과거 메시지의 출처는 회의 자료 변경으로 자동 교체되지 않는다.

회의 자료의 저장된 snapshot에서 제목, Markdown 제목과 인라인 코드 표기를 용어 후보로 볼 수 있다.
후보는 사용자가 선택하고 편집해 저장하며, 각 항목에는 원본 `sourceId`가 남는다.
같은 표기는 대소문자를 구분하지 않고 중복 저장하지 않는다. 자료를 갱신하거나 제거해도
승인된 용어와 출처는 유지되며 사용자가 직접 수정하거나 제거할 수 있다. 편집 중에 자료를 갱신해도
갱신 전 후보에서 고른 용어는 그 페이지가 회의에 연결되어 있고 이전 snapshot을 읽을 수 있으면 원래 `sourceId`로 저장한다. 용어집은 회의별로
관리하고 회의를 영구 삭제할 때 함께 제거한다. 같은 페이지가 연결된 다른 회의에서
승인한 용어는 사용자가 개별 선택해 가져올 수 있으며 원래 snapshot의 `sourceId`가 유지된다. 이 기능은 Notion 본문을 AI에 보내거나
자동 교정 규칙을 만들지 않는다. 인식 작업은 시작할 때 회의 용어집 snapshot을 고정한다.
Whisper는 승인된 표기를 길이 제한이 있는 prompt로 전달하고, Nemotron과 macOS 전사는 전달하지 않는다.
작업 원본의 `glossaryContext`에는 snapshot revision, hash와 prompt 사용 여부가 남는다.
기존 용어 교정 기능은 별도 설정과 규칙을 따른다.

연결이 해제되어도 저장한 자료의 목록과 제거 기능을 사용할 수 있다. 저장된 본문을 읽을 수 없는 항목도 목록에 표시하고 제거할 수 있다. 회의를 휴지통으로 옮기면 참조를 보존하고 영구 삭제 후 참조를 정리한다. 공통 문서 캐시는 회의 참조 제거로 삭제하지 않는다. 자료 목록은 각 문서를 개별 확인하며, AI 요청에 선택된 본문에는 위의 합계 한도를 적용한다.

## 공식 계약

- [Notion custom MCP client](https://developers.notion.com/guides/mcp/build-mcp-client)
- [Notion supported tools](https://developers.notion.com/guides/mcp/mcp-supported-tools)
- [MCP transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
- [MCP protocol versions](https://modelcontextprotocol.io/docs/2026-07-28/learn/versioning)
