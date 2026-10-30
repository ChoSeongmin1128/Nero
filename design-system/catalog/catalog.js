(() => {
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const icon = window.NeroIcons;
  const UI = window.NeroUI;
  const sections = [['overview','기본 원칙'],['foundations','색상과 토큰'],['typography','글자와 아이콘'],['controls','버튼과 입력'],['meetings','회의 목록'],['transcript','전사'],['minutes','회의록'],['people','사람과 그룹'],['sources','참고 자료'],['desktop','데스크톱'],['states','상태와 규칙']];
  const media = matchMedia('(prefers-color-scheme: dark)');
  const state = { color:'green', mode:'light', minutes:'다음 모임에서는 각자 고른 책의 첫 장을 함께 읽습니다.\n읽은 부분에서 남은 질문을 모아 이야기합니다.', minutesMode:'read', turn:'다음 모임에서는 각자 고른 책의 첫 장을 함께 읽으면 좋겠습니다.', attendees:[0,1], group:'all', inventory:null };
  const people = [{name:'참석자 A',role:'진행'},{name:'참석자 B',role:'기록'},{name:'참석자 C',role:'발표'}];
  const meetings = [{title:'10월 독서 모임',phase:'finished',date:'2026.10.19',count:3},{title:'다음 모임 준비',phase:'ready',date:'2026.10.26',count:2},{title:'새로운 책 선정과 다음 분기 모임 방식에 대한 논의 및 일정 조율',phase:'recording',date:'2026.10.19',count:8},{title:'날짜를 정하지 않은 모임',phase:'ready',date:'날짜 미정',count:0}];
  const phaseLabel = phase => ({ready:'준비',recording:'녹음 중',paused:'일시정지',finished:'종료'})[phase];
  const card = (title,source,body,extra='') => `<article class="ds-card ${extra}"><header class="ds-card-head"><h3>${title}</h3><code>${source}</code></header><div class="ds-specimen">${body}</div></article>`;
  const section = (id,description,body) => `<section id="${id}" class="ds-section"><span class="ds-kicker">${String(sections.findIndex(s=>s[0]===id)).padStart(2,'0')}</span><h2>${sections.find(s=>s[0]===id)[1]}</h2><p class="ds-lead">${description}</p>${body}</section>`;
  const contentTabsExample = `<div class="ds-reader-example"><div id="sample-content-tabs" class="content-tabs" role="tablist" aria-label="회의 내용 예제"><button id="catalog-summary-tab" type="button" role="tab" aria-selected="true" aria-controls="catalog-summary-panel">요약</button><button id="catalog-transcript-tab" type="button" role="tab" aria-selected="false" aria-controls="catalog-transcript-panel" tabindex="-1">음성 기록</button><button id="catalog-notes-tab" type="button" role="tab" aria-selected="false" aria-controls="catalog-notes-panel" tabindex="-1">메모</button></div><div class="ds-reader-panels"><section id="catalog-summary-panel" class="ds-reader-panel" role="tabpanel" aria-labelledby="catalog-summary-tab" tabindex="0"><h3>결정 사항</h3><p>다음 모임의 일정은 참석 가능한 시간을 확인한 뒤 정합니다.</p><h3>후속 작업</h3><p>참석 가능 시간을 모으고, 다음 회의에서 일정을 확정합니다.</p><h3>논의 내용</h3><p>발표와 자유 토론을 함께 진행하는 방식을 검토했습니다. 각 순서에 사용할 시간은 아직 정하지 않았습니다.</p><h3>다음 회의 안건</h3><p>진행 순서와 준비할 자료를 확인합니다.</p></section><section id="catalog-transcript-panel" class="ds-reader-panel" role="tabpanel" aria-labelledby="catalog-transcript-tab" tabindex="0" hidden><p>00:12 화자 1</p><p>다음 모임은 참석 가능한 시간을 먼저 확인하면 좋겠습니다.</p><p>00:25 화자 2</p><p>시간을 모아서 다음 회의에서 정하겠습니다.</p></section><section id="catalog-notes-panel" class="ds-reader-panel" role="tabpanel" aria-labelledby="catalog-notes-tab" tabindex="0" hidden><textarea class="text-input" aria-label="회의 메모 예제" placeholder="회의 메모"></textarea></section></div></div><p class="ds-rule">내용 탐색은 .content-tabs, 작은 보기 옵션은 .segment를 사용합니다. 한 패널만 표시하며 방향키와 Home/End로 탭을 전환합니다. 패널을 다시 만들지 않아 입력과 스크롤이 유지됩니다.</p>`;
  $('#catalog-nav').innerHTML = sections.map(([id,label])=>`<a href="#${id}">${label}</a>`).join('');
  $('#catalog-main').innerHTML = `<section id="overview" class="ds-section"><span class="ds-kicker">Nero / Components</span><h1>Nero 디자인 시스템</h1><p class="ds-lead">MVP 기준은 녹음, 한국어 전사, 화자 구분과 요약입니다. 자료 질문과 지식 관리는 확장 검토 예제입니다. 위에서 밝기를 바꾸고 각 예제에서 선택, 편집, 오류 복구를 확인할 수 있습니다.</p><div class="ds-principles"><div><h3>읽는 면은 조용하게</h3><p>주변 면은 낮은 채도로 구별하고, 대화와 문서를 읽는 중앙 면은 중립색으로 유지합니다.</p></div><div><h3>행동과 상태를 구분</h3><p>주요 행동은 강조색, 선택은 약한 채움과 경계, 녹음과 경고는 별도의 의미색을 사용합니다.</p></div><div><h3>맥락과 입력 보존</h3><p>읽던 위치, 선택한 참석자, 수정한 글을 유지합니다. 적용과 취소를 구분하고 불확실한 저장은 먼저 확인합니다.</p></div></div><p class="ds-rule">공통 화면과 플랫폼별 표면을 구분합니다. <a href="../windows-preview/">Windows 시안</a>에서는 동일한 제품 화면에 창과 트레이를 적용합니다. 컴포넌트 검토용 예제이며 실제 앱의 기능 범위와 구분합니다. 예제는 이 페이지의 메모리 안에서만 동작하며 새로고침하면 초기화됩니다.</p></section>` +
  section('foundations','중립색 바탕에 녹색을 주요 행동에만 사용합니다. 밝게, 어둡게, 시스템 모드를 지원합니다.',`<div class="ds-grid">${card('역할 색상','appearance.css','<div id="ds-swatches" class="ds-swatches"></div>','ds-full')}${card('원본 토큰','tokens.json / computed CSS','<div class="ds-token-tools"><input id="token-search" type="search" aria-label="토큰 검색" placeholder="토큰 이름 검색"><span id="token-count" class="ds-muted"></span></div><div class="ds-token-scroll"><table class="ds-table"><thead><tr><th>토큰</th><th>현재 값</th><th>원본</th></tr></thead><tbody id="token-rows"></tbody></table></div><p id="token-status" class="ds-rule"></p>','ds-full')}</div>`)+
  section('typography','한글과 영문은 공통 시스템 글꼴을 사용합니다. 크기, 굵기와 줄간격은 역할별 토큰으로 관리합니다.', `<div class="ds-grid">${card('글자 역할','typography.css', `<div id="type-role-samples">${[
    ['title','회의 제목과 긴 한국어 이름','회의 제목'],
    ['heading','회의록','구역 제목'],
    ['subtitle','결정 사항','소제목'],
    ['reading','출시일은 10월 23일입니다. QA 결과를 확인한 뒤 결정합니다.','읽기 본문'],
    ['body','회의 메모와 입력 내용은 같은 글자 기준을 사용합니다.','기본 본문'],
    ['label','회의 준비','버튼과 입력 이름'],
    ['caption','참석자 36명 / 2026.10.19 14:30','보조 정보'],
    ['timer','01:23:45','녹음 시간'],
    ['code','--text-body-size: 14px','코드'],
  ].map(([role, text, label]) => `<div class="ds-type-sample"><div class="type-${role}" data-type-role="${role}">${esc(text)}</div><span class="ds-type-label ds-type-meta" data-type-description="${role}">${label}</span></div>`).join('')}</div><p class="ds-rule">아래 값은 현재 화면의 CSS 계산값입니다. 실제 Windows 글꼴과 배율 검증은 별도로 진행합니다.</p>`)}${card('아이콘','icons.js / Lucide', `<div class="ds-icons">${['mic','file-text','users','folder-open','search','calendar-days','pause','play','square','ellipsis','copy','pencil','check','x','arrow-left'].map(name => `<div class="ds-icon-sample">${icon(name)}<span>${name}</span></div>`).join('')}</div>`)}</div>`) +
  section('controls','주요 행동을 구분하되 모든 버튼을 강조하지 않습니다. 마우스와 Tab 키로 hover와 focus를 확인할 수 있습니다.',`<div class="ds-grid">${card('행동의 우선순위','button / .primary / .quiet',`<div class="ds-example-row">${UI.button({html:icon('mic')+'녹음 시작',className:'primary',withIcon:true,attrs:{'data-demo':'start'}})}${UI.button({text:'새 회의',attrs:{'data-modal':'new'}})}${UI.button({text:'취소',className:'quiet',attrs:{'data-demo':'quiet'}})}</div><div class="ds-example-row" style="margin-top:18px"><button class="small" data-demo="small">적용</button><button class="icon quiet" aria-label="복사 예제" data-demo="icon">${icon('copy')}</button><button disabled>준비 중</button><button class="primary" disabled>녹음 시작</button></div><p class="ds-rule">주요 버튼은 .primary, 보조 행동은 기본 버튼, 낮은 우선순위는 .quiet를 사용합니다. 비활성화는 실제 disabled 속성입니다.</p>`)}${card('입력과 선택','input / .field / .segment',`<label class="field">회의 이름<input class="text-input" id="sample-title" placeholder="예: 10월 독서 모임" value="10월 독서 모임"></label><label class="field">메모<textarea class="text-input" rows="2" placeholder="미리 나눌 이야기"></textarea></label><div class="ds-example-row"><label><input type="checkbox" checked> 참고 자료 포함</label><label><input type="checkbox" disabled> 사용 불가</label></div><p class="ds-rule">이름이 비어 있으면 다음 행동을 비활성화합니다. 입력 중인 값을 테마 변경이나 다른 조작으로 다시 만들지 않습니다.</p>`)}${card('제목과 입력 상태','.text-input / .text-input--title',`<label class="field">회의 제목<input class="text-input text-input--title" aria-label="제목 입력 예제" value="새 회의"></label><label class="field">읽기 전용<input class="text-input" readonly value="저장된 회의"></label><label class="field">사용 불가<input class="text-input" disabled value="입력 불가"></label><label class="field">내용 있는 메모<textarea class="text-input" rows="3">지난 회의에서 정한 항목을 확인하고 오늘 논의할 내용을 기록합니다.</textarea></label>`)}${card('날짜와 시간','input[type=date] / input[type=datetime-local]',`<label class="field">회의 날짜<input id="sample-date" type="date" value="2026-10-19"></label><label class="field">회의 시작<input id="sample-datetime" type="datetime-local" value="2026-10-19T14:30"></label><label class="field">선택 가능 기간<input id="sample-date-range" type="date" min="2026-10-10" max="2026-10-25" value="2026-10-19"></label><div id="date-example-result" class="ds-date-result" role="status">2026-10-19 / 14:30</div>`)}${card('단일 선택',' .segment',`<div class="segment" id="sample-segment" role="group" aria-label="전사 표시"><button class="active" aria-pressed="true">읽기</button><button aria-pressed="false">원문</button></div><p id="segment-note" class="ds-rule">읽기: 수정한 내용을 표시합니다.</p>`)}${card('회의 내용 탐색',' .content-tabs / [role=tablist]',contentTabsExample,'ds-full')}${card('모달',' .modal / .modal-footer',`<button data-modal="new">새 회의 열기</button><p class="ds-rule">이름 입력 후 준비를 시작합니다. Escape와 취소는 입력을 적용하지 않고 닫습니다. 닫은 뒤에는 열었던 버튼으로 포커스를 돌려줍니다.</p>`)}</div>`)+
  section('meetings','한 행에서 제목, 참석자 수, 날짜, 상태를 읽습니다. 제목을 검색하거나 행을 눌러 전체 제목을 확인할 수 있습니다.',card('회의 목록',' .library-row',`<div class="ds-library"><label class="library-search">${icon('search')}<input id="meeting-search" aria-label="회의 검색" placeholder="회의 검색"></label><div id="meeting-results"></div></div><p class="ds-rule">행 높이 44px. 제목은 한 줄 말줄임, 날짜는 연도를 포함합니다. 날짜 미정은 생성일로 대신 표시하지 않습니다.</p>`))+
  section('transcript','발언자, 시간과 대화를 함께 읽습니다. 현재 발언과 미확인 이름을 따로 표시하고 원문과 수정본을 구분합니다.',card('발언과 수정',' .utterance / .speaker-rail',`<div class="ds-controls"><label>예제<select id="transcript-state"><option value="normal">기본</option><option value="current">녹음 중</option><option value="unknown">이름 미확인</option><option value="empty">빈 전사</option></select></label></div><div id="transcript-example" class="transcript"></div><p class="ds-rule">수정 버튼은 hover와 키보드 포커스에서 나타납니다. 시간을 누르면 해당 발언을 선택하는 예제를 표시합니다.</p>`))+
  section('minutes','읽기에서는 내용만큼 공간을 사용하고, 편집에서는 같은 내용을 이어서 수정합니다.',card('읽기와 편집',' .minutes-document / .editor',`<div class="minutes-heading"><h2>회의록</h2><div class="segment" id="minutes-mode" role="group" aria-label="회의록 보기"><button class="active" aria-pressed="true" data-mode="read">읽기</button><button aria-pressed="false" data-mode="edit">편집</button></div></div><div id="minutes-example"></div><p class="ds-rule">예제 입력은 읽기 전환과 테마 변경 후에도 유지됩니다. 실제 AI 초안 생성이나 외부 저장은 수행하지 않습니다.</p>`))+
  section('people','사람의 이름과 역할을 함께 표시합니다. 그룹은 참석자를 고르는 지름길이며, 선택 결과와 원본 그룹은 분리됩니다.',card('사람과 그룹',' .people-grid / .selector-person',`<div class="people-grid"><nav class="group-list" aria-label="사람 예제 그룹"><button class="active" data-group="all">모든 사람 <span class="group-count">3</span></button><button data-group="reading">독서 모임 <span class="group-count">2</span></button><button data-group="empty">새 그룹 <span class="group-count">0</span></button></nav><section class="people-directory"><div id="people-example"></div></section></div><div class="ds-example-row" style="margin-top:22px"><button data-modal="attendees">참석자 선택</button><span id="attendee-count" class="ds-muted">2명 선택</span></div><p class="ds-rule">참석자 선택의 적용은 명단을 반영하고, 취소는 변경을 버립니다. 참석자 선택은 발언자 이름 자동 확정이나 목소리 등록이 아닙니다.</p>`))+
  section('sources','정상 자료에는 반복 안내를 붙이지 않습니다. 접근 실패와 연결 끊김에는 복구할 수 있는 행동을 제공합니다.',`<div class="ds-grid">${card('자료 선택',' .selector-person / .badge',`<label class="selector-person"><input type="checkbox" checked><span class="grow">함께 읽을 책과 질문 모음</span><span class="badge">문서</span></label><label class="selector-person"><input type="checkbox"><span class="grow">다음 분기 모임 진행 방식과 참여자별 발표 순서 정리 문서</span><span class="badge">노트</span></label><label class="selector-person"><input type="checkbox" disabled><span class="grow">접근할 수 없는 자료</span><span class="badge warn">사용 불가</span></label>`)}${card('접근 실패와 복구',' .notice',`<div id="source-recovery"><div class="notice"><strong>자료를 가져오지 못했습니다</strong><p>연결 상태를 확인하고 다시 시도해주세요.</p><button class="small" id="source-retry">다시 가져오기</button></div></div><p class="ds-rule">복구 과정은 가상 상태입니다. 예제는 외부 계정에 연결하지 않습니다.</p>`)}</div>`)+
  section('desktop','공통 녹음 상태를 macOS의 메뉴와 노치에 표시한 브라우저 시안입니다. 상황별 알림과 녹음 제어를 직접 확인할 수 있습니다.',`<article class="ds-card"><div id="desktop-example" class="ds-desktop-host"></div></article><p class="ds-rule">현재 노치와 데스크톱 배경은 macOS 검토용입니다. Windows의 트레이, 부유 제어기, 알림은 <a href="../windows-preview/">Windows 시안</a>에서 같은 제품 화면으로 비교할 수 있습니다. 두 플랫폼 모두 브라우저 시안이며 네이티브 구현은 아닙니다. 드래그 위치 검증과 OS 권한은 원래 제품 시안의 별도 검토 범위입니다.</p>`)+
  section('states','상태를 색상만으로 전달하지 않고 문구와 행동을 함께 둡니다.',`<div class="ds-grid">${card('상태 표현',' .status-chip / .badge / .notice',`<div class="ds-state-line"><strong>준비</strong><span class="status-chip ready">준비</span><span class="ds-muted">녹음은 명시적으로 시작</span></div><div class="ds-state-line"><strong>녹음</strong><span class="status-chip recording">녹음 중</span><span class="ds-muted">일시정지 / 종료</span></div><div class="ds-state-line"><strong>일시정지</strong><span class="status-chip paused">일시정지</span><span class="ds-muted">종료와 구분</span></div><div class="ds-state-line"><strong>진행 중</strong><button disabled>가져오는 중...</button><span class="ds-muted">중복 요청 방지</span></div><div class="ds-state-line"><strong>확인 필요</strong><span class="badge warn">저장 결과 확인 필요</span></div>`)}${card('적용 범위','현재 시안',`<div class="ds-table-wrap"><table class="ds-table"><tbody><tr><th>공통</th><td>색상 역할, 글자, 버튼, 입력, 회의 목록, 전사, 회의록, 참석자와 자료 선택</td></tr><tr><th>macOS 시안</th><td>메뉴바, 노치, 분리 제어기, 데스크톱 알림</td></tr><tr><th>Windows 시안</th><td><a href="../windows-preview/">Windows 시안 열기</a>: 제목 표시줄, 트레이 진입점, 부유 제어기, 알림과 포커스 복귀</td></tr><tr><th>미확정</th><td>로고, 설정 계층, 플랫폼별 네이티브 구현</td></tr></tbody></table></div>`)}</div><div class="ds-table-wrap"><table class="ds-table"><thead><tr><th>상황</th><th>규칙</th><th>이 페이지의 예제</th></tr></thead><tbody><tr><td>hover / focus</td><td>행동을 드러내고 포커스 링은 선택 경계와 구분</td><td>버튼, 행, 전사 수정</td></tr><tr><td>disabled</td><td>실제 조작을 차단하며 이유가 되는 상태를 주변에 표시</td><td>빈 회의 이름, 사용 불가 자료</td></tr><tr><td>empty</td><td>없는 내용을 지어내지 않고 다음 행동을 안내</td><td>검색 결과, 빈 그룹, 빈 전사</td></tr><tr><td>loading / error</td><td>중복 요청을 막고 실패 후 재시도 경로 제공</td><td>자료 다시 가져오기, 마이크 오류</td></tr><tr><td>긴 제목</td><td>목록은 말줄임, 상세는 줄바꿈, 전체 제목 접근 가능</td><td>회의 목록과 데스크톱 제목 예시</td></tr><tr><td>저장 불확실</td><td>같은 저장을 재요청하지 않고 기존 결과부터 확인</td><td>규칙만 문서화, 외부 쓰기 없음</td></tr></tbody></table></div>`);

  function renderTypeMetrics() {
    $$('[data-type-role]').forEach(node => {
      const style = getComputedStyle(node);
      const label = $('[data-type-description="' + node.dataset.typeRole + '"]');
      if (label) label.textContent = style.fontSize + ' / ' + style.lineHeight + ' / ' + style.fontWeight;
    });
  }
  renderTypeMetrics();
  let toastTimer;
  function announce(message) { const node=$('#ds-announcement'); node.textContent=message; node.hidden=false; clearTimeout(toastTimer); toastTimer=setTimeout(()=>node.hidden=true,3500); }
  const roleTokens = ['--canvas','--surface','--surface-secondary','--surface-tint','--text','--text-secondary','--accent-fill','--accent-subtle','--selection-border','--focus','--recording-text','--warning-text'];
  function applyTheme() { const mode=state.mode==='system'?(media.matches?'dark':'light'):state.mode; document.documentElement.dataset.color=state.color; document.documentElement.dataset.mode=mode; $('#resolved-mode').textContent=state.mode==='system'?`현재 ${mode==='dark'?'어둡게':'밝게'}`:''; $$('.ds-theme').forEach(n=>{n.dataset.mode=mode;n.setAttribute('aria-pressed',String(n.dataset.palette===state.color));}); renderTokens(); window.NeroControls?.refresh(); }
  function renderTokens() { const css=getComputedStyle(document.documentElement); const theme=state.inventory?.themes?.resolved?.find(t=>t.color===state.color&&t.mode===document.documentElement.dataset.mode); const desktopHost=$('#desktop-example .desktop-preview'); const tokenProbe=document.createElement('span'); tokenProbe.className='desktop-recorder'; tokenProbe.hidden=true; desktopHost?.append(tokenProbe); const platformCSS=desktopHost?getComputedStyle(tokenProbe):null; const valueFor=t=>css.getPropertyValue(t.name).trim()||theme?.resolvedValues?.[t.name]||theme?.values?.[t.name]||platformCSS?.getPropertyValue(t.name).trim()||'현재 범위에서 미확인'; $('#ds-swatches').innerHTML=roleTokens.map(name=>`<div class="ds-swatch"><div class="ds-swatch-color" style="background:var(${name})"></div><div class="ds-swatch-copy">${name.slice(2)}<code>${esc(css.getPropertyValue(name).trim())}</code></div></div>`).join(''); const query=$('#token-search').value.toLowerCase(); const unique=new Map(); (state.inventory?.tokens||roleTokens.map(name=>({name,file:'appearance.css'}))).forEach(t=>{if(!unique.has(t.name))unique.set(t.name,t);}); const tokens=[...unique.values()].filter(t=>t.name?.toLowerCase().includes(query)); $('#token-count').textContent=`${tokens.length}개`; $('#token-rows').innerHTML=tokens.map(t=>`<tr><td><code>${esc(t.name)}</code></td><td><code>${esc(valueFor(t))}</code></td><td><code>${esc(t.file)}${t.line?`:${t.line}`:''}</code></td></tr>`).join(''); tokenProbe.remove(); }
  $('#ds-mode').addEventListener('change',e=>{state.mode=e.target.value;applyTheme();}); media.addEventListener('change',()=>{if(state.mode==='system')applyTheme();}); $('#token-search').addEventListener('input',renderTokens);
  fetch('tokens.json').then(r=>{if(!r.ok)throw Error();return r.json();}).then(data=>{state.inventory=data;$('#token-status').textContent='현재 값은 선택한 테마의 실제 CSS 계산값 또는 원본에서 해결한 값입니다. 플랫폼 토큰은 같은 테마의 데스크톱 구성 요소에서 계산합니다. 원본 전체 선언과 조건은 tokens.json에서 확인할 수 있습니다.';renderTokens();}).catch(()=>{$('#token-status').textContent='전체 토큰 목록을 읽지 못했습니다. 역할 색상은 현재 CSS 값으로 표시합니다. localhost에서 다시 열어주세요.';});
  function renderMeetings() {
    const query = $('#meeting-search').value.toLowerCase();
    const rows = meetings.filter(meeting => meeting.title.toLowerCase().includes(query));
    $('#meeting-results').innerHTML = rows.length ? rows.map(meeting => UI.meetingRow({
      title: meeting.title, phase: meeting.phase, count: meeting.count, date: meeting.date,
      label: phaseLabel(meeting.phase), attrs: { 'data-meeting': meeting.title },
    })).join('') : '<div class="library-empty"><h2>일치하는 회의가 없습니다</h2>' + UI.button({ text: '검색 초기화', attrs: { id: 'clear-meeting-search' } }) + '</div>';
  }
  $('#meeting-search').addEventListener('input',renderMeetings);
  function renderTranscript(editing = false) {
    const mode = $('#transcript-state').value;
    $('#transcript-example').innerHTML = mode === 'empty' ? '<div class="q-empty"><h2>기록된 대화가 없습니다</h2></div>' : UI.utterance({
      speaker: mode === 'unknown' ? '발언자 1' : '참석자 A', time: '00:42',
      unknown: mode === 'unknown', current: mode === 'current', editing, text: state.turn,
      timeAttrs: { 'data-turn-time': 'true' },
      contentHTML: `<label class="field">전사 수정<textarea id="turn-edit" rows="3">${esc(state.turn)}</textarea></label><div class="row">${UI.button({ text: '적용', className: 'primary small', attrs: { id: 'turn-save' } })}${UI.button({ text: '취소', className: 'small', attrs: { id: 'turn-cancel' } })}</div>`,
      actionsHTML: UI.button({ text: '수정', className: 'small quiet', attrs: { id: 'turn-start' } }),
    });
  }
  $('#transcript-state').addEventListener('change',()=>renderTranscript());
  function renderMinutes() {
    $('#minutes-example').innerHTML = state.minutesMode === 'edit' ? `<div class="editor"><label class="field">회의 내용<textarea id="minutes-input" rows="7">${esc(state.minutes)}</textarea></label></div>` : UI.minutes([{ title: '회의 내용', text: state.minutes }]);
    $$('#minutes-mode button').forEach(button => { const active = button.dataset.mode === state.minutesMode; button.classList.toggle('active', active); button.setAttribute('aria-pressed', active); });
  }
  $('#minutes-example').addEventListener('input',e=>{if(e.target.id==='minutes-input')state.minutes=e.target.value;}); $('#minutes-mode').addEventListener('click',e=>{const b=e.target.closest('button');if(b){state.minutesMode=b.dataset.mode;renderMinutes();}});
  function renderPeople() {
    const list = state.group === 'empty' ? [] : state.group === 'reading' ? people.slice(0, 2) : people;
    $('#people-example').innerHTML = list.length ? list.map(person => UI.personRow({ ...person, initial: person.name.slice(-1) })).join('') : '<div class="people-directory-empty">그룹에 등록된 사람이 없습니다.</div>';
    $$('[data-group]').forEach(button => { button.classList.toggle('active', button.dataset.group === state.group); button.setAttribute('aria-pressed', button.dataset.group === state.group); });
  }
  const dialog=$('#ds-dialog'); let returnFocus;
  function openDialog(kind, title) {
    returnFocus = document.activeElement;
    const cancel = UI.button({ text: kind === 'meeting' ? '닫기' : '취소', attrs: { 'data-dialog-close': 'true' } });
    const content = kind === 'attendees' ? {
      title: '회의 참석자',
      bodyHTML: people.map((person, index) => UI.personOption({ ...person, initial: person.name.slice(-1), checked: state.attendees.includes(index), attrs: { name: 'attendee', value: index } })).join(''),
      actionsHTML: cancel + UI.button({ text: '적용', className: 'primary', attrs: { id: 'attendees-apply' } }),
    } : kind === 'meeting' ? { title, actionsHTML: cancel } : {
      title: kind === 'prepare' ? '회의 준비' : '새 회의',
      bodyHTML: `<label class="field">회의 이름<input id="new-meeting-title" placeholder="예: 10월 독서 모임" value="${kind === 'prepare' ? esc(title) : ''}"></label>`,
      actionsHTML: cancel + UI.button({ text: kind === 'prepare' ? '녹음 시작' : '회의 준비', className: 'primary', disabled: kind !== 'prepare', attrs: { id: 'new-meeting-apply' } }),
    };
    dialog.innerHTML = UI.dialogContent({ ...content, titleId: 'ds-dialog-title' });
    dialog.dataset.kind = kind;
    dialog.showModal();
  }
  dialog.addEventListener('close',()=>returnFocus?.focus()); dialog.addEventListener('input',e=>{if(e.target.id==='new-meeting-title')$('#new-meeting-apply').disabled=!e.target.value.trim();});
  function selectContentTab(tab) {
    $$('#sample-content-tabs [role="tab"]').forEach(button => {
      const selected = button === tab;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
      document.getElementById(button.getAttribute('aria-controls')).hidden = !selected;
    });
  }
  $('#sample-content-tabs').addEventListener('keydown', event => {
    if (event.isComposing || event.keyCode === 229 || event.altKey || event.ctrlKey || event.metaKey) return;
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    const tabs = $$('#sample-content-tabs [role="tab"]');
    const index = tabs.indexOf(event.target);
    if (index < 0) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    selectContentTab(tabs[next]);
    tabs[next].focus();
  });
  document.addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;
    if(b.closest('#sample-content-tabs'))selectContentTab(b);
    if(b.dataset.demo)announce('버튼 동작 예제입니다. 실제 녹음은 시작하지 않습니다.');
    if(b.dataset.modal)openDialog(b.dataset.modal);
    if(b.dataset.meeting)openDialog('meeting',b.dataset.meeting);
    if(b.hasAttribute('data-dialog-close'))dialog.close();
    if(b.id==='attendees-apply'){state.attendees=$$('input[name=attendee]:checked',dialog).map(n=>Number(n.value));$('#attendee-count').textContent=`${state.attendees.length}명 선택`;dialog.close();}
    if(b.id==='new-meeting-apply'){if(dialog.dataset.kind==='prepare'){desktop.capture={phase:'recording',title:$('#new-meeting-title').value,elapsedLabel:'00:00'};desktop.notification=null;desktop.control='expanded';renderDesktop();}else announce(`"${$('#new-meeting-title').value.trim()}" 준비 예제`);dialog.close();}
    if(b.id==='clear-meeting-search'){$('#meeting-search').value='';renderMeetings();$('#meeting-search').focus();}
    if(b.id==='turn-start'){renderTranscript(true);$('#turn-edit').focus();} if(b.id==='turn-cancel')renderTranscript(); if(b.id==='turn-save'){state.turn=$('#turn-edit').value;renderTranscript();}
    if(b.hasAttribute('data-turn-time'))announce('00:42 발언을 선택했습니다. 예제 오디오는 재생하지 않습니다.');
    if(b.dataset.group){state.group=b.dataset.group;renderPeople();}
    if(b.closest('#sample-segment')){$$('#sample-segment button').forEach(n=>{n.classList.toggle('active',n===b);n.setAttribute('aria-pressed',n===b);});$('#segment-note').textContent=b.textContent==='읽기'?'읽기: 수정한 내용을 표시합니다.':'원문: 원래 전사 내용을 보존합니다.';}
    if(b.id==='source-retry'){b.disabled=true;b.textContent='가져오는 중...';$('#source-recovery').setAttribute('aria-busy','true');setTimeout(()=>{$('#source-recovery').innerHTML='<p class="ds-rule" role="status">자료를 다시 가져왔습니다.</p><button class="small" id="source-reset">실패 상태 다시 보기</button>';$('#source-recovery').removeAttribute('aria-busy');},1000);}
    if(b.id==='source-reset')$('#source-recovery').innerHTML='<div class="notice"><strong>자료를 가져오지 못했습니다</strong><p>연결 상태를 확인하고 다시 시도해주세요.</p><button class="small" id="source-retry">다시 가져오기</button></div>';
  });
  const desktop={open:true,scene:'document',notification:'upcoming',control:'expanded',overlay:{placement:'notch',expanded:true},meeting:{title:'10월 독서 모임',countdownLabel:'5분 후'},capture:{phase:'idle',elapsedLabel:'00:00'}};
  function renderDesktop(focusAction) { $('#desktop-example').innerHTML=NeroDesktopView.render(desktop); const notice=$('[data-desktop-title-preview]'); const title=$('.desktop-notice-title'); if(notice&&title){notice.dataset.titleTruncated=String(title.scrollWidth>title.clientWidth);notice.addEventListener('pointerenter',()=>{notice.dataset.titleHovered='true';delete notice.dataset.titleDismissed;});notice.addEventListener('pointerleave',()=>{delete notice.dataset.titleHovered;delete notice.dataset.titleDismissed;});notice.addEventListener('focusin',()=>delete notice.dataset.titleDismissed);notice.addEventListener('keydown',e=>{if(e.key==='Escape')notice.dataset.titleDismissed='true';});} if(focusAction){const candidates=$$('[data-action]', $('#desktop-example')); candidates.find(b=>b.dataset.action===focusAction)?.focus({preventScroll:true});} }
  $('#desktop-example').addEventListener('click',e=>{const b=e.target.closest('[data-action]');if(!b)return;const action=b.dataset.action.replace('desktop:',''); const value=b.dataset.value;
    if(action==='scenario'){desktop.error=null;desktop.otherRecording=false;desktop.menuOpen=false;desktop.confirmStop=false;desktop.control='expanded';desktop.overlay={placement:'notch',expanded:true};desktop.exampleCase=['long-title','unbroken-title'].includes(value)?value:null;desktop.meeting.title=value==='long-title'?'다음 분기 모임의 진행 방식과 참여자별 발표 순서를 정하기 위한 사전 논의':value==='unbroken-title'?'다음분기모임의진행방식과참여자별발표순서를정하기위한사전논의':'10월 독서 모임';desktop.capture={phase:['recording','paused'].includes(value)?value:['permission','mic-disconnected','other-recording'].includes(value)?'recording':value==='finished'?'finished':'idle',title:desktop.meeting.title,elapsedLabel:'18:42'};desktop.notification=['upcoming','long-title','unbroken-title','other-recording'].includes(value)?'upcoming':value==='finished'?'finished':null;if(['permission','mic-disconnected'].includes(value))desktop.error=value;if(value==='other-recording'){desktop.otherRecording=true;desktop.capture.title='이전 모임';}}
    else if(action==='pause')desktop.capture.phase='paused'; else if(action==='resume')desktop.capture.phase='recording'; else if(action==='toggle-expanded')desktop.overlay.expanded=!desktop.overlay.expanded;
    else if(action==='toggle-menu'){desktop.menuOpen=!desktop.menuOpen;desktop.menuOrigin=value;}
    else if(action==='detach'||action==='dock'){desktop.overlay.placement=action==='detach'?'floating':'notch';desktop.menuOpen=false;}
    else if(action==='hide-control')desktop.control='hidden'; else if(action==='restore-control'){desktop.control='expanded';desktop.showControlInApp=true;}
    else if(action==='stop'){openStop();return;} else if(action==='retry'){desktop.error=null;desktop.capture.phase='recording';}
    else if(action==='dismiss-reminder')desktop.notification=null;else if(action==='snooze-reminder')desktop.notification='snoozed';else if(action==='scene')desktop.scene=value;
    else if(action==='advance'){desktop.capture.elapsedLabel='23:42';desktop.meeting.countdownLabel='곧 시작';}
    else if(action==='start'){openDialog('prepare',desktop.meeting.title);return;}
    else if(action==='open-app'){openDialog('meeting',desktop.capture.title||desktop.meeting.title);return;}
    renderDesktop(b.dataset.action);
  });
  function openStop(){returnFocus=document.activeElement;dialog.dataset.kind='stop';dialog.innerHTML=`<h2 id="ds-dialog-title">녹음을 종료할까요?</h2><p>${esc(desktop.capture.title||desktop.meeting.title)}</p><div class="modal-footer"><span>${desktop.capture.elapsedLabel}</span><div class="row"><button data-dialog-close>돌아가기</button><button class="primary" id="desktop-finish">녹음 종료</button></div></div>`;dialog.showModal();}
  dialog.addEventListener('click',e=>{if(e.target.closest('#desktop-finish')){desktop.capture.phase='finished';desktop.notification='finished';desktop.menuOpen=false;desktop.error=null;dialog.close();renderDesktop();}});
  let navigationFrame;
  function updateCurrentSection() {
    const boundary = $('.ds-toolbar').getBoundingClientRect().bottom + 80;
    const visible = $$('.ds-section').find(node => node.getBoundingClientRect().bottom > boundary);
    if (!visible) return;
    $$('#catalog-nav a').forEach(link => { if (link.hash === '#' + visible.id) link.setAttribute('aria-current', 'location'); else link.removeAttribute('aria-current'); });
    $('#section-label').textContent = sections.find(([id]) => id === visible.id)[1];
  }
  function queueNavigation() { cancelAnimationFrame(navigationFrame); navigationFrame = requestAnimationFrame(updateCurrentSection); }
  window.addEventListener('scroll', queueNavigation, { passive: true });
  window.addEventListener('resize', queueNavigation);
  window.addEventListener('hashchange', queueNavigation);
  queueNavigation();
  renderMeetings();renderTranscript();renderMinutes();renderPeople();renderDesktop();applyTheme();window.NeroControls?.enhanceAll();
  ['sample-date','sample-datetime','sample-date-range'].forEach(id=>$('#'+id).addEventListener('change',e=>{$('#date-example-result').textContent=e.target.value||'날짜 미정';}));
})();
