# Nero Windows 시안

`index.html`은 공통 제품 화면을 iframe으로 재사용하는 Windows 데스크톱 시안이다. 색상과 밝기, 회의, 전사, 메모, 회의록은 공통 화면을 사용한다. Windows 표면은 이 디렉터리가 소유한다.

## 표면

- 32px 제목 표시줄과 오른쪽 최소화, 최대화/복원, 닫기
- 제목 영역 드래그와 더블클릭 최대화/복원
- 작업 표시줄과 알림 영역의 Nero 진입점
- 알림 영역 메뉴의 녹음 상태, 시간, 일시정지/계속, 종료, 창 복원
- 노치가 없는 부유 녹음 제어기, 드래그 이동과 숨기기/복원
- 오른쪽 아래 회의 알림과 준비/미루기/닫기
- 긴 제목 줄바꿈과 캡션 버튼 영역 보존
- Escape로 메뉴 닫기, 알림 영역 메뉴의 방향키/Home/End, Ctrl+Shift+G 창 복원

본창이 보일 때는 본창의 녹음 막대를 사용한다. 창 닫기와 최소화는 시안 창만 감추며 녹음을 종료하지 않고 부유 제어기를 표시한다. 같은 회의의 예정 알림은 녹음이 시작되면 닫는다. 녹음 종료는 별도 확인을 거친다. Windows OS 알림 템플릿, 트레이 API, 전역 단축키나 실제 창 관리 구현이 아니다. Ctrl+Shift+G는 이 브라우저 시안의 포커스 안에서 사용하는 검토용 명령이다.

## 공통 화면 연결

iframe은 `../prototype/index.html?platform=windows&session=windows-design`을 사용한다. 부모 연결 코드가 시안 전용 상태를 제공하며 원래 시안 저장 공간과 분리한다.

같은 origin과 iframe source를 확인한 `postMessage`만 처리한다.

- shell → 공통 화면: `{type: "windows-nero:command", action, ...payload}`
- 공통 화면 → shell: `{type: "windows-nero:ready"}`
- 공통 화면 → shell: `{type: "windows-nero:state", phase, title, elapsedLabel, error, ownerId, notification, countdownLabel}`
- 공통 화면에서 종료를 요청하면 `windows-nero:request-stop`의 `ownerId`로 확인창을 열고, 확인 후 동일한 `ownerId`를 stop 명령에 포함한다.
- action: `sync`, `open`, `prepare`, `start`, `pause`, `resume`, `stop`, `retry`, `snooze`, `dismiss`, `advance`
- `scenario`: `value`가 `upcoming`, `recording`, `paused`, `error`, `finished`, `long`
- `appearance`: `color`, `mode`

녹음 상태는 공통 화면에서 받은 값을 표시한다. shell은 창 위치, 메뉴, 알림 표시, 부유 제어기 표시만 페이지 메모리에 보관한다. shell 자체는 localStorage, 마이크, OS 알림, 외부 계정을 사용하지 않는다.

## 참조

- [Microsoft title bar design](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design)
- [Microsoft notification area](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area)
- [Microsoft app notifications](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/)

공통 색상은 `../appearance.css`, 아이콘은 `../icons.js`를 사용한다. Windows shell의 고정 중립색과 배치는 `windows.css`에 한정된다.
