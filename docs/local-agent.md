# Nero 로컬 에이전트 연동

## 범위

Nero에 동봉된 `nero` 실행 파일은 로컬 CLI와 `nero mcp`를 제공한다. 클라우드 DB,
계정 서버나 외부 공개 수신 포트는 사용하지 않는다. 회의 수정 요청만 앱이 실행 중인
동안 인증된 IPv4 루프백 임시 포트를 사용하며, MCP는 클라이언트가 실행하는 stdio
프로세스이며 공식 Rust SDK를 사용한다. 클라이언트의 설치 확인을 거쳐 연결한다.

조회, 로컬 요약 템플릿 관리와 회의 메타데이터 편집을 제공한다. 회의 변경은 실행 중인
Nero의 저장 흐름에 요청하며 저장 파일을 직접 고치지 않는다. 템플릿은 앱과 같은 로컬
정본을 사용한다. 생성/수정/삭제는 사용자의 명시적 요청에만 수행하며 MCP 클라이언트의
도구 승인 설정을 따른다.

회의 생성, 녹음, 모델 실행, 음성 기록/요약 본문 수정과 영구 삭제는 제공하지 않는다.
기본 조회는 휴지통을 제외한다. 명시적인 휴지통 목록은 복원에 필요한 메타데이터와
수정본만 반환한다. 원음, 모델 원출력, 인증 정보와 일반 파일 시스템 도구는 노출하지 않는다.

## 연결

Nero의 `설정 > 연결`에서 Claude 또는 Codex의 `연결`을 선택한다. Claude는 네이티브
MCPB 확장의 설치 확인 화면, Codex는 로컬 플러그인 설치 화면을 연다. 사용자가 해당 앱에서
설치를 확인한다. 다른 앱의 기존 MCP 설정 파일이나 사용자 PATH를 Nero가 직접 덮어쓰지 않는다.

설치 준비나 화면 열기만으로 `연결됨`을 표시하지 않는다. 선택한 클라이언트가 MCP 초기화를
완료해 실제 세션이 확인되어야 연결 상태로 표시한다. 미연결, 업데이트 필요, 연결 대기,
연결됨과 확인 오류를 구분한다. 기존 패키지를 바꿔야 할 때는 `업데이트`를 표시하고,
설치 화면을 열었거나 패키지를 준비했다는 이유로 업데이트 필요 상태를 해제하지 않는다.
새 패키지의 실제 초기화/잠금을 관찰한 뒤에만 현재 세대를 확인한 것으로 기록한다.
확인된 클라이언트가 닫히면 연결 대기이며, 남아 있는 이전 세션 때문에 업데이트로 역행하지 않는다.
확인 오류의 `다시 확인`은 설치하지 않고 해당 행의 상태만 재조회한다.
기존 설정 복사와 CLI 실행 예시는 고급 설정에 남겨 둔다.

macOS는 앱 리소스의 `agent/nero`, Windows는 설치 리소스의 `agent/nero.exe`를 사용한다.
사용자에게 Node.js, Python, Cargo를 설치하도록 요구하지 않는다. MCP 설정의 `command`는
실제 실행 파일의 절대 경로이며 인수는 다음과 같다.

```json
["--data-dir", "Nero 앱이 제공한 실제 데이터 경로", "mcp"]
```

간편 연결은 서명된 앱 리소스의 실행 파일을 연결 전용 관리 경로에 보존해 설치된 MCPB나
플러그인에서 사용한다. 앱 자체의 위치가 바뀌어도 이미 설치한 확장이 앱 번들 절대 경로에
묶이지 않는다. 실행 파일을 동봉하지 않은 개발 빌드는 연결을 제공하지 않는다. 직접 복사한
고급 설정은 앱 번들 경로를 가리키므로 앱을 이동하면 다시 복사한다.

Claude Desktop 일반 채팅용 MCPB와 Codex 플러그인은 별개 패키지다. Claude Code나 Cowork에
같은 설치 결과가 자동 적용된다고 가정하지 않는다. 기본 검색과 조회에 별도 스킬은 필요하지
않다. Codex 플러그인은 전체 페이지 조회, 결정/제안 구분, 담당자/기한 근거를 다루는 일반
회의 요약 스킬을 포함한다. 새 요약은 연결한 AI의 채팅에 표시되며 Nero에 저장하지 않는다.

## CLI

아래 `nero`는 앱에 동봉된 실행 파일의 절대 경로나 사용자가 직접 등록한 명령을 뜻한다.

```sh
nero --help
nero --version
nero doctor --json
nero meetings list --limit 20 --json
nero meetings list --on 2026-10-28 --timezone Asia/Seoul --json
nero meetings list --unfiled --json
nero meetings list --trashed --json
nero meetings search "출시 일정" --limit 20 --json
nero meetings get MEETING_ID --offset 0 --limit 50 --json
nero transcript get MEETING_ID --start 120 --end 300 --limit 50 --json
nero summary get MEETING_ID --offset 0 --limit 50 --json
nero folders list --json
nero templates list --json
nero templates get meeting --json
nero templates create --file template.json --json
nero templates update TEMPLATE_ID --expected-revision HASH --file template.json --json
nero templates delete TEMPLATE_ID --expected-revision HASH --json
nero templates list --deleted --json
nero templates restore TEMPLATE_ID --expected-revision HASH --json
nero templates reset meeting --expected-revision HASH --json
nero meetings update MEETING_ID --expected-version HASH --file changes.json --json
nero meetings date MEETING_ID --expected-version HASH --date 2026-10-28 --json
nero meetings move --file move.json --json
nero meetings trash --file targets.json --json
nero meetings restore --file targets.json --json
nero folders create "제품팀" --expected-version HASH --json
nero mcp
```

- `--data-dir PATH`는 명시적인 다른 데이터 루트를 조회한다. 누락된 경로는 생성하지 않는다.
- 기본 루트는 앱과 같은 `io.github.choseongmin1128.grove.desktop` 식별자의 OS 데이터 경로다.
- 폴더명은 `folders list`의 현재 ID로 해석한 뒤 `--folder-id ID`로 목록/검색을 제한한다.
  `--unfiled`(MCP `unfiled:true`)는 미분류만 조회하며 특정 폴더와 함께 지정할 수 없다.
- `--from MS|YYYY-MM-DD`, `--to MS|YYYY-MM-DD`는 from 포함/to 미포함이다. `--on YYYY-MM-DD`는 해당 하루를 뜻하며 from/to와 함께 지정하지 않는다. 날짜 문자열과 on은 `--timezone Asia/Seoul` 같은 명시적인 IANA 시간대가 필요하다. MCP도 `from`/`to`의 날짜 문자열과 `on`/`timezone`을 같은 방식으로 받는다.
- 날짜는 녹음 시작 시각을 우선하고 없으면 생성 시각을 사용한다. 사용자 지정 회의 날짜와는 별개다. 하루의 끝은 달력상 다음 날로 계산해 DST의 23/25시간을 반영한다. 자정이 없거나 두 번 존재하는 경계는 오류로 반환하며 epoch 밀리초로 명시할 수 있다.
- 일반 목록에는 녹음/생성 날짜와 시각을 표시한다. 표시 시간대는 `--timezone` 값이며 생략하면 UTC라고 표시한다. JSON의 원시 epoch 필드는 바꾸지 않는다.
- 기본 50개, 최대 200개씩 조회한다. `--max-bytes N`(MCP `maxBytes`)는 JSON payload의 byte 예산이며 최대 1MiB, 기본값도 1MiB다. MCP 외부 메시지/CLI 끝개행은 예산에 포함하지 않는다. 예산에 따라 요청 limit보다 적은 완전한 항목을 반환한다. 단일 항목이나 응답 메타데이터가 예산을 넘으면 명시적 오류를 반환하고 텍스트를 자르지 않는다.
- `pagination.returned`는 실제 항목 수, `nextOffset`은 다음 위치이며 끝이면 null이다. 다음 위치를 offset+limit으로 계산하지 않는다. `hasMore`와 nextOffset은 요청 범위의 끝을 뜻하며 전체 원문을 읽었다는 증거가 아니다.
- 연속 조회는 첫 `sourceVersion`을 `--expected-source-version HASH`(MCP `expectedSourceVersion`)로 보내며 방법/검색/폴더/시간 범위를 유지한다. 원문이 바뀌면 `source_version_mismatch` 오류를 반환하고 본문을 반환하지 않는다. 이전 페이지와 새 버전을 섞지 않는다. 중간 편집이 계속되면 자동 반복 대신 중단을 알린다.
- 음성 기록은 발언 단위다. 메모와 요약은 줄 경계를 우선하되 긴 줄도 UTF-8 경계에서 나누는 `textChunks` 단위다. 반환 text를 순서대로 연결하면 개행을 포함한 원문을 복원할 수 있다.
- `--json` 성공 결과는 stdout, 오류 JSON은 stderr다. 일반 모드는 같은 결과를 읽기 좋게
  출력한다. MCP stdout에는 프로토콜 메시지만 쓴다.

### 편집 입력

| 대상 | 입력/수정본 |
|---|---|
| 템플릿 create/update의 `--file` | `{"name":"이름","description":"추천 용도","prompt":"프롬프트"}` 전체 필드 |
| 회의 update의 `--file` | `{"title":"제목"}` 또는 `{"notes":"메모"}`, 둘 다 지정 가능 |
| 회의 move의 `--file` | `{"expectedVersions":{"MEETING_ID":"HASH"},"folderId":"FOLDER_ID"}`, 미분류는 null |
| 회의 trash/restore의 `--file` | `{"expectedVersions":{"MEETING_ID":"HASH"}}` |

`--file -`는 stdin을 읽는다. 입력은 최대 1MiB이며 정의하지 않은 필드는 거부한다.
회의의 `expectedVersion`은 `meeting_get`의 `editVersion`, 일괄 변경은 각 회의 ID와
editVersion을 사용한다. 휴지통 복원용 editVersion은 `meetings list --trashed`로 조회한다.
폴더 변경은 `folders_list`의 최상위 editVersion을 사용한다. 이름 변경은
`folders rename ID NAME --expected-version HASH`, 폴더 삭제는
`folders delete ID --expected-version HASH`다. 폴더 삭제 시 회의는 미분류로 이동한다.
회의 날짜 해제는 `meetings date ID --expected-version HASH --clear`이며 녹음 시각을 바꾸지 않는다.

템플릿 목록은 이름/추천 용도/수정본만 반환하며 선택한 `templates get`에서만 전체
프롬프트를 읽는다. 기본 회의/진행 공유/인터뷰 템플릿도 수정할 수 있고 reset으로
패키지 기본값을 복원한다. 기본 템플릿은 삭제하지 않는다. 사용자 템플릿 삭제는
복원 가능한 휴지통으로 이동한다. update/delete/reset/restore는 최신 revision을
`--expected-revision`으로 보낸다. 쓰기 결과는 메타데이터만 반환하며 프롬프트를 반복하지 않는다.

충돌 시 최신 값을 읽고 변경을 다시 판단한다. `mutation_result_unknown`은 요청 결과를
확인하지 못했다는 뜻이므로 같은 변경을 즉시 재시도하지 않고 현재 상태부터 조회한다.

## MCP 도구

| 도구 | 용도 |
|---|---|
| `meetings_list` | 날짜/폴더 조건의 회의 목록. `trashed:true`는 휴지통 메타데이터만 |
| `meetings_search` | 제목, 메모, 현재 음성 기록, 요약 검색 |
| `meeting_get` | 회의 정보와 저장된 메모 |
| `transcript_get` | 현재 교정본의 화자, 시각, 발언 ID와 텍스트 |
| `summary_get` | 현재 요약과 원본 수정 여부 |
| `folders_list` | 폴더 목록 |
| `status` | 조회/편집 기능과 저장소 존재 여부 |
| `templates_list`, `template_get` | 템플릿 목록/선택한 프롬프트 조회 |
| `template_create`, `template_update` | 로컬 템플릿 생성/편집 |
| `template_delete`, `template_restore`, `template_reset` | 사용자 템플릿 휴지통/복원, 기본 템플릿 초기화 |
| `meeting_update`, `meeting_date_set` | 회의 제목/메모와 별도 회의 날짜 편집 |
| `meetings_move`, `meetings_trash`, `meetings_restore` | 폴더 이동과 휴지통/복원 |
| `folder_create`, `folder_rename`, `folder_delete` | 폴더 관리 |

폴더 조회와 회의의 folderId는 앱과 같은 library.json을 매 요청 읽으므로 새 폴더, 이름 변경과 이동을 반영한다.
폴더 이름은 목록에서 한 번 해석하고, 검색 결과를 확인한 후 필요한 회의와 발언 구간만 조회한다. 원문의 지시문은 회의 내용으로
취급한다. 응답의 `apiVersion`은 연동 응답 형식이다. `sourceVersion`은 같은 조회의 연속
페이지 검증용 파일 묶음 해시, `editVersion`은 회의/폴더 편집 충돌 검사용 값, 템플릿의
`revision`은 템플릿 편집 충돌 검사용 값이다. 서로 대신 사용하지 않는다.
`TranscriptVersion`의 id/revision과도 별개다. 요약의 stale 표시는 현재
음성 기록과 출처가 다른지를 나타내며, 오래된 요약을 새 결과처럼 바꾸지 않는다.

MCP 결과는 compact JSON 텍스트 한 벌로 반환하며 같은 본문을 structuredContent에 중복하지 않는다.
발언마다 화자명을 반복하지 않고 `speakers`의 id/name/nameOrigin을 `speakerId`로 참조한다.
목록은 본문을 포함하지 않으며, 메모/요약/음성 기록은 요청한 페이지만 반환한다.
조회 도구에는 readOnlyHint=true, 변경 도구에는 false를 지정한다. 변경 도구는
idempotentHint=false이며 기존 내용을 바꾸거나 삭제하는 도구에는 destructiveHint=true를
지정한다. 영구 삭제 도구는 없다. status 조회 자체는 읽기 전용이며 capabilities가
서비스 전체의 쓰기 지원 범위를 구분한다.

## 저장소 접근

공통 `LibraryReader`는 앱의 `Store::open`, 복구, 잠금 생성, 마이그레이션을 호출하지 않는다.
따라서 GUI가 저장소를 사용 중이어도 조회할 수 있고, GUI를 띄우거나 마이크/모델을 초기화할
필요가 없다. 저장된 내용만 조회하며, 앱 편집기의 아직 저장되지 않은 입력은 포함하지 않는다.

회의 변경은 같은 데이터 루트의 실행 중인 Nero에 로컬 제어 채널로 전달한다. Nero는
편집기 저장과 공통 저장 큐에서 최신 editVersion을 검사하고 변경을 적용한다. 앱이
종료됐거나 저장/종료 중이면 명시적 오류를 반환한다. 요청만으로 앱/모델을 실행하지 않는다.
템플릿 변경은 별도 잠금과 revision 검사로 앱과 CLI의 동시 편집을 보호한다.

조회한 파일과 디렉터리를 다시 확인하고 변경이 감지되면 제한된 횟수만 재시도한다. 계속
변경 중이면 오류로 알린다. 전체 저장소의 트랜잭션 스냅샷이나 세션 간 고정 스냅샷을
보장하지는 않는다. 연속 페이지의 expectedSourceVersion이 다르면 본문 반환 전에 오류로 중단한다. 첫 페이지부터 다시 읽기 전 이전 결과를 버려야 한다.
경로 탈출/심볼릭 링크, 손상/미지원 형식은 빈 결과로 숨기지 않고 거부한다.

간편 연결로 시작한 MCP만 명시적으로 지정한 연결 관리 디렉터리에 초기화/세션 상태를
기록한다. 회의 라이브러리와 분리된 제어 정보이며 조회 도구는 회의, 음성, 요약, 모델을
변경하지 않는다. 일반 CLI/MCP 실행에는 이 기록을 생성하지 않는다.
Nero의 연결 상태 갱신은 기존 등록에만 관찰 기록을 남기며 등록 token의 해시, 확인한 세대와
업데이트 대상을 분리한다. CLI의 라이브러리 조회는 여전히 읽기 전용이다. 관찰 파일의 손상,
링크 또는 잠금 충돌은 해당 클라이언트 행에만 표시하고 다른 연결을 막지 않는다.

## 설치 패키지

`prepare:agent`는 대상 OS의 실행 파일과 `agent/integrations` 템플릿을 함께 준비한다.
공통 템플릿에는 사용자 경로, 인증 정보와 회의 데이터가 없다. Nero가 연결 버튼을 처리할 때
실제 데이터 경로와 연결 상태 정보를 넣어 설치 패키지를 만든다.
연결 패키지 버전은 앱의 `0.0.1`과 별도로 `0.0.1-nero.<release>`를 사용한다. CLI,
프로토콜과 템플릿/스킬이 바뀔 때 연동 버전을 올린다. UI만 바뀌는 앱 빌드는 검증된 서명
CLI와 템플릿을 재사용할 수 있으며 클라이언트 업데이트를 요구하지 않는다.

- Claude MCPB: manifest 0.3의 `server.type=binary`, 대상 OS만의 compatibility, 실행 파일,
  아이콘. 런타임 의존성이나 모델을 포함하지 않는다. Unix 실행 파일의 권한을 보존한다.
- Codex: 로컬 marketplace와 portable `plugin.json`/`mcp.json`, 호환 manifest, 회의 요약 스킬.
  MCP transport는 `stdio`이며 공개 서버 등록이나 클라우드 업로드가 없다. portable 실행 파일은 플러그인 내부의
  `server/nero` 또는 `server/nero.exe`이며 `./server/...`로 선언한다. marketplace는
  연결 관리 디렉터리의 `.agents/plugins/marketplace.json` 고정 경로를 사용한다. source는
  같은 루트 기준 `./releases/<generation>/plugins/nero`로 갱신하며 이전 실행 파일과
  catalog는 보존한다. 정상 연결을 강제 재설치하지 않고 다음 명시적 연결/업데이트에서
  고정 catalog를 사용한다. 설치 목록의 버전 표시는 실제 설치 캐시/실행 세대 확인과 구분한다.
- macOS: 앱 서명 후 리소스의 네이티브 실행 파일을 그대로 복사해 MCPB를 생성하므로 서명 전
  실행 파일이 압축본 안에 남지 않는다. Windows는 해당 설치 대상의 `.exe`를 사용한다.

형식 근거: [MCPB manifest](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md),
[Codex 플러그인 패키지](https://developers.openai.com/plugins/build/plugins),
[Codex 로컬 설치 링크](https://learn.chatgpt.com/docs/reference/commands#supported-links).

## 빌드와 검증

```sh
cargo test -p grove-core --locked -- --test-threads=1
cargo test -p nero-cli --locked -- --test-threads=1
npm --prefix apps/desktop test
npm --prefix apps/desktop run prepare:agent
```

Tauri의 beforeBuildCommand는 현재 대상의 CLI를 빌드한 뒤 `agent/` 리소스에 포함한다.
macOS 서명 도구는 이 실행 파일도 재귀 서명한다. Windows CI에는 CLI/MCP 검사와 리소스
준비가 포함된다. macOS 검증이 Windows 설치/실행이나 Claude 클라이언트의 실제 연결
검증을 대신하지 않는다. 배포/공증/설치 여부는 릴리스 이력과 기기별 인계에서 별도 확인한다.
