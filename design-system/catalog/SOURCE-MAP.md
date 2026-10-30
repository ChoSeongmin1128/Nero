# 소스와 토큰

## 정본

| 소스 | 책임 |
| --- | --- |
| `../appearance.css` | 중립색, 녹색 강조와 light/dark의 의미 색상 |
| `../typography.css` | 공통 글꼴과 글자 역할 |
| `../components.css`, `../components.js` | 공통 컨트롤 스타일과 반복 뷰 |
| `../controls.css`, `../controls.js` | 선택 메뉴와 날짜/시간 |
| `../style.css`, `../prototype/` | 제품 배치/기능 상태/이벤트 |
| `../desktop-preview.css`, `../desktop-preview-view.js` | macOS 노치/부유 녹음/알림 예제 |
| `../windows-preview/` | Windows 창/트레이/부유 녹음과 공통 iframe 연결 |
| `catalog.css`, `catalog.js` | 문서 배치와 공통 컴포넌트의 예제 소비처 |

`build-tokens.cjs`는 실제 CSS 선언/조건/소스 위치와 역할/테마를 추출한다. `tokens.json`의 `file/line/column`이 위치 기준이다. 문서에 복사한 줄 번호나 수치를 정본으로 두지 않는다.

## 생성

```sh
node design-system/catalog/build-tokens.cjs
node design-system/catalog/build-tokens.cjs --check
node design-system/catalog/check-tokens.cjs
```

같은 입력은 같은 결과를 만든다. 주석을 포함한 입력의 해시도 비교하므로 소스 변경 후 다시 생성한다. 제품 저장소/설치 앱의 기능 완료나 배포를 뜻하지 않는다.

## 역할

글꼴/크기/굵기/행간/자간은 typography.css의 명명된 역할을 사용한다. 한국어 루트는 단어 단위 줄바꿈과 긴 무공백 텍스트의 예외 줄바꿈을 공유한다. 색상은 canvas/surface/text/accent/focus/warning/recording 등의 역할로 관리한다. 간격/반경/높이는 공통 컴포넌트와 플랫폼 구조에서 필요한 값을 소유한다. `rawValues`와 `metrics`는 실제 사용값의 목록이며 보편 scale을 선언한 것이 아니다.

warning/recording 색상은 브랜드 색상과 독립이고 밝기에 따라 변한다. ready는 Mono의 회색 예외가 있다. 데스크톱 배경과 노치/트레이 표면도 브랜드 팔레트와 분리한다.

JSON의 `themes.resolved`는 소스에서 해석한 12개 색상/밝기 조합과 마지막 선언의 위치다. 이 추출기는 전체 CSS cascade/layout 엔진이 아니다. 실제 요소의 계산값, 줄바꿈, 잘림, 열린 popup 위치는 브라우저에서 확인한다. 플랫폼 내부 scope에만 정의한 값은 root 계산값으로 단정하지 않는다.

## 잔여 경계

공통 뷰와 제어기는 제품/카탈로그가 함께 사용한다. 모든 기능별 모달/그룹 편집/자료 연결을 범용화하지 않는다. 실제 OS 폰트/배율/접근성/녹음 동작은 별도 검증한다. 실제 앱의 기능 범위는 apps/desktop/README.md를 따른다. Windows 브라우저 시안은 네이티브 구현 완료가 아니다.
