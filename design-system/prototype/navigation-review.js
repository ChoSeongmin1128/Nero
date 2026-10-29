import { createNotificationFlow, notificationTransition, notificationProjection, restoreNotificationFlow, notificationLabels } from '../notification-flow.mjs';
import { readStoredSnapshot } from './notification-snapshot.mjs';
import { mountActionMenus } from '../action-menu.mjs';
import { mountNavigationSidebar } from './navigation-sidebar.js';
import { recentMeetingRow, renderMeetingMenu } from '../navigation.mjs';

const $=id=>document.getElementById(id), icon=NeroIcons;
let meetings=[{id:1,title:'제품 검토 및 다음 배포 일정 논의',folder:'제품팀',date:'10. 29. 10:30',length:'52:17'},{id:2,title:'고객 인터뷰 - 사용 경험과 개선 요청',folder:'인터뷰',date:'10. 28. 14:00',length:'38:12'},{id:3,title:'운영 개선 사항 검토',folder:'제품팀',date:'10. 27. 11:00',length:'24:08'},{id:4,title:'새로운 아이디어 메모',folder:null,date:'10. 26. 16:00',length:'12:40'}];
const menus=mountActionMenus({owner:document});
let mode=localStorage.getItem('nero-proposal-mode')||'folders',filter='all',selected=1,transcript=false,processingState='normal';
$('toggleNav').innerHTML=icon('panel-left-close');
const initialFolders=['제품팀','인터뷰'];
let userFolders=[...initialFolders];
const demoStorageKey='nero-notification-demo-v1';
const initialMeetings=meetings.map(meeting=>({...meeting}));
let flow=createNotificationFlow(),flowTarget=1,flowFeedback='대상 회의를 선택하고 변환을 시작하세요.';
try { const saved=readStoredSnapshot(JSON.parse(sessionStorage.getItem(demoStorageKey)),initialFolders); if(saved) { meetings=saved.meetings;userFolders=saved.userFolders;flow=saved.flow;flowFeedback='저장된 예시를 복구했습니다. 진행 중이던 변환은 중단되었습니다.'; } } catch {}
function persistDemo(){try{sessionStorage.setItem(demoStorageKey,JSON.stringify({version:1,meetings,userFolders,flow}));}catch{flowFeedback='이 브라우저에서는 예시 상태를 저장할 수 없습니다.';}}
function jobFor(id){return flow.jobs.find(job=>job.meetingId===id);}
function applyFlow(type,id=flowTarget,attempt=jobFor(id)?.attempt){const before=flow;flow=notificationTransition(flow,{type,meetingId:id,attempt});flowFeedback=flow===before?'이미 반영되었거나 현재 시도와 다른 결과입니다. 상태를 유지합니다.':`${meetings.find(m=>m.id===id)?.title||'알림'}: ${jobFor(id)?notificationLabels[jobFor(id).status==='processing'?jobFor(id).phase:jobFor(id).status]:'정리됨'}`;persistDemo();}
function renderFlowControls(){
 const available=meetings.filter(m=>!m.deletedAt);if(!available.some(m=>m.id===flowTarget))flowTarget=available[0]?.id;
 $('flow-meeting').innerHTML=available.map(m=>`<option value="${m.id}" ${m.id===flowTarget?'selected':''}>${escape(m.title)}</option>`).join('');
 const job=jobFor(flowTarget),running=job?.status==='processing';
 const enabled={start:!running,advance:running&&job.phase!=='saving',completed:running,failed:running,cancelled:running,retry:['failed','cancelled','interrupted'].includes(job?.status),duplicate:job&&job.status!=='processing',stale:job?.attempt>1};
 for(const [action,active] of Object.entries(enabled))document.querySelector(`[data-flow="${action}"]`).disabled=!active;
 $('flow-status').textContent=flowFeedback;
}

const escape=value=>String(value??'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const navigation=mountNavigationSidebar({shell:$('shell'),aside:document.querySelector('aside'),content:document.querySelector('main'),trigger:$('toggleNav'),backdrop:document.querySelector('.backdrop')});
const syncNav=()=>navigation.render();
addEventListener('resize',syncNav);
document.querySelector('.recent-chevron').innerHTML=icon('chevron-down');
const recent=document.querySelector('.recent');recent.open=localStorage.getItem('nero-proposal-recents')!=='false';recent.ontoggle=()=>localStorage.setItem('nero-proposal-recents',recent.open);

function render(){ menus.close(); syncNav(); persistDemo(); renderFlowControls();
const live=meetings.filter(m=>!m.deletedAt);
document.querySelector('.sidebar-utilities [data-filter="trash"] .count').textContent=meetings.filter(m=>m.deletedAt).length;
processingState=jobFor(selected)?.status||'normal';
document.querySelectorAll('.sidebar-utilities [data-filter]').forEach(button=>{if(!selected&&button.dataset.filter===filter)button.setAttribute('aria-current','page');else button.removeAttribute('aria-current');});

$('recents').innerHTML=live.map(m=>recentMeetingRow({menu:renderMeetingMenu(m,{escape,icon}),id:m.id,title:m.title,metadata:[m.folder||'미분류',m.date,m.length],active:selected===m.id,escape})).join('');
const notifications=notificationProjection(flow,meetings);const unread=notifications.filter(job=>job.status==='processing'||!job.acknowledged);
$('notification-count').hidden=!unread.length;$('notification-count').textContent=unread.length;
const meeting=meetings.find(item=>item.id===selected);
const folder=meeting ? (meeting.folder||'unfiled') : filter;
const folderName=folder==='unfiled'?'미분류':folder==='trash'?'휴지통':folder==='notifications'?'알림':folder;
$('breadcrumbs').innerHTML=`<button class="quiet" data-home>회의 목록</button>${folder!=='all'?`<span class="breadcrumb-separator" aria-hidden="true">${icon('chevron-right')}</span><button class="quiet" data-filter="${folder}" ${selected?'':'aria-current="page"'}>${escape(folderName)}</button>`:''}${meeting?`<span class="breadcrumb-separator" aria-hidden="true">${icon('chevron-right')}</span><span class="breadcrumb-current" aria-current="page" title="${escape(meeting.title)}">${escape(meeting.title)}</span>`:''}`;
const navItem=([id,name,i,count])=>`<button data-filter="${id}" ${!selected&&filter===id?'aria-current="page"':''}>${icon(i)}${name}<span class="count">${count}</span></button>`;
$('nav').innerHTML=`<div class="library-root">${navItem(['all','회의 목록','book-open',live.length])}<button class="quiet add-folder" data-add-folder aria-label="새 폴더" title="새 폴더">${icon('plus')}</button></div><div class="library-children">${navItem(['unfiled','미분류','inbox',live.filter(m=>!m.folder).length])}${userFolders.map(f=>navItem([escape(f),escape(f),'folder',live.filter(m=>m.folder===f).length])).join('')}</div>`;$('transcriptToggle').hidden=!selected||Boolean(meeting?.draft);$('transcriptToggle').setAttribute('aria-expanded',transcript);
if(!selected&&filter==='notifications'){
$('page').innerHTML=`<div class="heading"><div><h1 tabindex="-1">알림</h1><small>${notifications.filter(job=>job.status==='processing').length}개 진행 중 / ${unread.filter(job=>job.status!=='processing').length}개 확인 필요</small></div><button class="quiet" data-clear-acknowledged ${notifications.some(job=>job.acknowledged)?'':'disabled'}>확인한 알림 정리</button></div><div class="notification-list">${notifications.map(job=>{const running=job.status==='processing',retryable=['failed','cancelled','interrupted'].includes(job.status);return `<article class="notification-item" data-notification="${job.meetingId}" data-job-status="${job.status}"><div><button class="quiet notification-title" data-open="${job.meetingId}">${escape(job.title)}</button><p>${job.label}${job.acknowledged?' / 확인함':''}</p>${job.status==='failed'?'<p>변환을 마치지 못했습니다. 음성과 메모는 유지됩니다.</p>':job.status==='interrupted'?'<p>화면을 다시 연 뒤 중단된 작업입니다. 다시 시도할 수 있습니다.</p>':''}</div><div class="notification-actions">${running?`<button class="quiet" data-job-cancel="${job.meetingId}">취소</button>`:`${retryable?`<button data-job-retry="${job.meetingId}">다시 시도</button>`:''}${job.acknowledged?`<button class="quiet" data-notification-dismiss="${job.meetingId}">정리</button>`:`<button class="quiet" data-notification-acknowledge="${job.meetingId}">확인</button>`}`}</div></article>`;}).join('')||'<p class="notification-empty muted">알림이 없습니다.</p>'}</div>`;return;
}
if(selected){const m=meetings.find(m=>m.id===selected);if(m.draft){$('page').innerHTML=renderDraft(m);return;}$('page').innerHTML=`<h1>${escape(m.title)}</h1><p class="metadata"><span>2026. ${escape(m.date)}</span><span>${escape(m.folder||'미분류')}</span></p>${statusMarkup()}<div class="player"><button class="quiet icon" id="play" aria-label="예시 재생 상태">${icon('play')}</button><span>0:00</span><input type="range" aria-label="재생 위치" min="0" max="3137" value="0"><span>${m.length}</span><details class="speed"><summary>1x</summary><div class="speed-menu">${[.75,1,1.25,1.5,2].map(n=>`<button data-speed="${n}">${n}x${n===1?' 선택':''}</button>`).join('')}</div></details></div><div class="detail-body"><article class="summary"><h2>요약</h2><section><h2>핵심</h2><p>녹음 안정성과 오류 복구를 우선 개선하고, 배포 전 긴 회의에서 다시 확인합니다.</p></section><section><h2>결정 사항</h2><ul><li>금요일까지 오류를 수정한 뒤 다음 주 배포 여부를 결정합니다. <button class="citation" data-cite>18:42</button></li><li>회의 요약을 기본 화면으로 유지합니다.</li></ul></section><section><h2>할 일</h2><ul><li>첫 녹음 권한과 오류 복구 흐름 점검</li><li>긴 회의 처리와 저장 상태 재검증</li></ul></section></article><section class="transcript" ${transcript?'':'hidden'}><header><h2>음성 기록</h2><button class="quiet icon" data-close-transcript aria-label="음성 기록 닫기">${icon('x')}</button></header><p class="muted">전체 기록</p><div class="utterance"><small class="utterance-meta"><span>18:20</span><span>화자 1</span></small>지금 버전에서 녹음이 끊기는 이슈가 가끔 발생하는 것 같습니다.</div><div class="utterance active"><small class="utterance-meta"><span>18:42</span><span>화자 2</span></small>금요일까지 오류부터 정리하고, 다음 주에 배포할 수 있을지 다시 보죠.</div><div class="utterance"><small class="utterance-meta"><span>19:06</span><span>화자 1</span></small>그럼 요약 화면은 지금 구조를 유지하겠습니다.</div><div class="utterance"><small class="utterance-meta"><span>20:15</span><span>화자 3</span></small>추가로 긴 회의에서도 안정적으로 처리되는지 확인이 필요합니다.</div></section></div>`;return;}
const query=$('search').value.toLowerCase();const list=meetings.filter(m=>(filter==='trash'?Boolean(m.deletedAt):!m.deletedAt&&(filter==='all'||(filter==='unfiled'?!m.folder:m.folder===filter)))&&m.title.toLowerCase().includes(query));
const folders=filter==='all'&&mode==='folders'&&!query;
$('page').innerHTML=`<div class="heading"><div><h1>${escape(filter==='all'?'회의 목록':filter==='unfiled'?'미분류':filter==='trash'?'휴지통':filter)}</h1><small>${list.length}개 회의</small></div>${filter==='all'?`<div class="segmented" aria-label="회의 보기"><button data-mode="folders" aria-pressed="${mode==='folders'}">폴더별</button><button data-mode="all" aria-pressed="${mode==='all'}">전체 회의</button></div>`:''}</div>${folders?`<div class="folders">${userFolders.map(f=>`<button class="folder" data-filter="${escape(f)}">${icon('folder')}<div><strong>${escape(f)}</strong><small>${live.filter(m=>m.folder===f).length}개 회의</small></div></button>`).join('')}</div><p class="section-label">미분류</p>`:''}${list.filter(m=>!folders||!m.folder).map(m=>`<div class="row">${icon('audio-lines')}<button class="open" data-open="${m.id}"><strong title="${escape(m.title)}">${escape(m.title)}</strong><span class="list-metadata"><span>${escape(m.date)}</span><span>${escape(m.length)}</span>${!folders?'<span>'+escape(m.folder||'미분류')+'</span>':''}</span></button><button class="quiet icon" data-open="${m.id}" aria-label="${escape(m.title)} 열기">${icon('arrow-right')}</button></div>`).join('')||'<p class="muted">회의가 없습니다.</p>'}`;
}
document.addEventListener('click',e=>{const t=e.target.closest('button');if(!t)return;if(t.hasAttribute('data-home')){selected=null;filter='all';render();}if(t.closest('[data-meeting-menu]'))return;if(t.dataset.open||t.dataset.recentMeeting){selected=Number(t.dataset.open||t.dataset.recentMeeting);transcript=false;navigation.navigate();render();}if(t.dataset.filter){filter=t.dataset.filter;selected=null;navigation.navigate();render();}if(t.dataset.mode){mode=t.dataset.mode;localStorage.setItem('nero-proposal-mode',mode);render();}if(t.hasAttribute('data-cite')){transcript=true;render();}if(t.hasAttribute('data-close-transcript')){transcript=false;render();$('transcriptToggle').focus();}if(t.dataset.speed){t.closest('details').querySelector('summary').textContent=t.dataset.speed+'x';t.closest('details').open=false;}if(t.id==='play'){t.innerHTML=icon(t.getAttribute('aria-pressed')==='true'?'play':'pause');t.setAttribute('aria-pressed',t.getAttribute('aria-pressed')!=='true');}});
$('search').oninput=()=>{selected=null;filter='all';render();};$('transcriptToggle').onclick=()=>{transcript=!transcript;render();};$('width').onclick=()=>{$('shell').classList.toggle('half');navigation.resetTemporary();$('width').textContent=$('shell').classList.contains('half')?'전체 너비':'절반 너비';syncNav();};$('theme').onclick=()=>{const dark=document.documentElement.dataset.mode==='dark';document.documentElement.dataset.mode=dark?'light':'dark';$('theme').textContent=dark?'어둡게':'밝게';};document.addEventListener('keydown',e=>{if(e.defaultPrevented||document.querySelector('dialog[open]'))return;if(e.key==='Escape'){if(transcript){transcript=false;render();$('transcriptToggle').focus();}else {navigation.resetTemporary();}}});$('new').innerHTML=icon('plus')+'<span>새 회의</span>';
document.querySelector('.sidebar-utilities [data-filter="trash"]').insertAdjacentHTML('afterbegin',icon('trash-2'));
$('settings').insertAdjacentHTML('afterbegin',icon('settings'));
$('notifications-open').insertAdjacentHTML('afterbegin',icon('bell'));
function statusMarkup(){
 if(processingState!=='processing')return '';
 const job=jobFor(selected);
 return `<div class="meeting-status" role="status" data-status="processing"><span class="status-mark"></span><span>${notificationLabels[job.phase]}</span><div class="status-actions"><button class="quiet" data-cancel>취소</button></div></div>`;
}
function startSelected(){flowTarget=selected;applyFlow('start',selected);render();}
$('flow-toggle').onclick=()=>{const panel=$('flow-controls');panel.hidden=!panel.hidden;$('flow-toggle').setAttribute('aria-expanded',!panel.hidden);};
$('flow-meeting').onchange=()=>{flowTarget=Number($('flow-meeting').value);flowFeedback='대상 회의를 변경했습니다.';renderFlowControls();};
document.addEventListener('click',event=>{const button=event.target.closest('[data-flow]');if(!button)return;const action=button.dataset.flow;
 if(action==='open'){selected=flowTarget;transcript=false;navigation.navigate();}
 else if(action==='another'){selected=meetings.find(m=>!m.deletedAt&&m.id!==flowTarget)?.id||flowTarget;transcript=false;navigation.navigate();}
 else if(action==='multiple'){for(const m of meetings.filter(m=>!m.deletedAt).slice(0,2))applyFlow('start',m.id);flowFeedback='두 회의의 변환을 시작했습니다. 알림에서 각각 취소하거나 대상 회의를 바꿔 결과를 반영하세요.';}
 else if(action==='duplicate')applyFlow(jobFor(flowTarget).status);
 else if(action==='stale')applyFlow('completed',flowTarget,jobFor(flowTarget).attempt-1);
 else if(action==='reload'){flow=restoreNotificationFlow(JSON.parse(JSON.stringify(flow)));meetings.forEach(m=>{m.recording=false;});flowFeedback='새로고침 복구 예시: 진행 중인 작업을 중단 상태로 복구했습니다.';}
 else if(action==='reset'){meetings=initialMeetings.map(meeting=>({...meeting}));userFolders=[...initialFolders];flow=createNotificationFlow();selected=1;flowTarget=1;filter='all';flowFeedback='예시 상태를 초기화했습니다.';}
 else applyFlow(action);
 render();
});
const dialog=document.querySelector('.prototype-dialog');let dialogAction=null,dialogReturn=null,dialogRecent=null,dialogDraftAction=null;
function showDialog(title,content,action,{confirm='확인',destructive=false}={}){const button=dialog.querySelector('button[value="confirm"]');button.textContent=confirm;button.className=destructive?'danger':'primary';dialogReturn=document.activeElement;dialogRecent=dialogReturn?.closest('[data-recent-row]')?.dataset.recentRow;dialogDraftAction=['data-calendar','data-import'].find(attribute=>dialogReturn?.hasAttribute(attribute));dialog.returnValue='cancel';dialog.querySelector('[data-dialog-title]').textContent=title;dialog.querySelector('[data-dialog-content]').innerHTML=content;dialogAction=action;dialog.showModal();}
dialog.addEventListener('close',()=>{const action=dialogAction;dialogAction=null;if(dialog.returnValue==='confirm')action?.();const recent=[...document.querySelectorAll('[data-recent-row]')].find(row=>row.dataset.recentRow===dialogRecent);const candidate=recent?.querySelector('summary')||(dialogReturn?.isConnected?dialogReturn:null)||(dialogDraftAction?(document.querySelector(`[${dialogDraftAction}]`)||$('draft-title')):null);const target=candidate&&!candidate.closest('[inert],[hidden]')?candidate:$('toggleNav');target?.focus({preventScroll:true});dialogReturn=null;dialogRecent=null;});
function createMeeting(title='새 회의'){
 const id=Math.max(...meetings.map(m=>m.id))+1;
 const occupied=new Set(meetings.filter(m=>!m.deletedAt).map(m=>m.title));
 let unique=title,suffix=1;while(occupied.has(unique))unique=`${title} (${suffix++})`;
 meetings.unshift({id,title:unique,folder:filter!=='all'&&userFolders.includes(filter)?filter:null,date:'10. 30. 10:30',length:'',draft:true,notes:''});
 selected=id;flowTarget=id;processingState='normal';transcript=false;navigation.navigate();render();$('draft-title')?.focus();
}
function renderDraft(m){return `<header class="draft-heading"><input id="draft-title" class="text-input text-input--title" aria-label="회의 이름" value="${escape(m.title)}"><div class="metadata"><span>${escape(m.calendar?m.calendar.date.slice(0,10).replaceAll('-','. ')+' '+m.calendar.date.slice(11,16):'2026. '+m.date)}</span><button class="quiet" data-calendar>${icon('calendar-days')}${m.calendar?escape(m.calendar.title):'일정 연결'}</button></div></header>${statusMarkup()}<section class="capture-start" aria-labelledby="capture-heading"><div><h2 id="capture-heading">${m.recording?'녹음 중':m.audioName?'음성':'녹음'}</h2>${m.audioName?`<p class="attached-audio">${icon('file-text')}<span>${escape(m.audioName)}</span></p>`:''}</div><div class="capture-actions">${m.recording?'<span class="recording-time">00:00</span><button class="danger-text" data-stop-recording>녹음 종료</button>':m.audioName?'':`<button class="primary" data-record>${icon('mic')}녹음 시작</button><button data-import>${icon('upload')}음성 가져오기</button>`}</div></section><section class="draft-notes"><h2>메모</h2><textarea id="draft-notes" class="text-input" aria-label="회의 메모" placeholder="메모">${escape(m.notes)}</textarea></section>`;}
document.addEventListener('input',event=>{const m=meetings.find(m=>m.id===selected);if(!m?.draft)return;if(event.target.id==='draft-notes')m.notes=event.target.value;if(event.target.id==='draft-notes')persistDemo();if(event.target.id==='draft-title'){m.title=event.target.value;const crumb=document.querySelector('.breadcrumb-current');if(crumb){crumb.textContent=m.title;crumb.title=m.title;}const title=document.querySelector(`[data-recent-row="${m.id}"] strong`);if(title){title.textContent=m.title;title.title=m.title;}persistDemo();}});
$('new').onclick=()=>createMeeting();
$('settings').onclick=()=>showDialog('설정','<label>화면 모드<select name="appearance"><option value="dark">어둡게</option><option value="light">밝게</option></select></label>',()=>{document.documentElement.dataset.mode=dialog.querySelector('select').value;});
document.addEventListener('click',e=>{const t=e.target.closest('button');if(!t)return;
 if(t.dataset.notificationDismiss||t.dataset.notificationAcknowledge||t.dataset.jobRetry||t.dataset.jobCancel){
 const id=Number(t.dataset.notificationDismiss||t.dataset.notificationAcknowledge||t.dataset.jobRetry||t.dataset.jobCancel);
 const type=t.dataset.notificationDismiss?'dismiss':t.dataset.notificationAcknowledge?'acknowledge':t.dataset.jobRetry?'retry':'cancelled';
 applyFlow(type,id);render();const next=type==='retry'?'data-job-cancel':type==='cancelled'?'data-job-retry':type==='acknowledge'?'data-notification-dismiss':null;
 (next?document.querySelector(`[${next}="${id}"]`):document.querySelector('.notification-title')||document.querySelector('#page h1'))?.focus({preventScroll:true});
 }
 if(t.hasAttribute('data-clear-acknowledged')){applyFlow('clearAcknowledged');render();document.querySelector('#page h1')?.focus();}
 if(t.hasAttribute('data-cancel')){applyFlow('cancelled',selected);render();$('notifications-open').focus();}
 if(t.hasAttribute('data-add-folder'))showDialog('폴더 추가','<label>폴더 이름<input name="folder" placeholder="폴더 이름" required autofocus maxlength="40"></label>',()=>{const value=dialog.querySelector('input').value.trim();if(value&&!userFolders.includes(value)){userFolders.push(value);selected=null;filter=value;render();}});
 if(t.hasAttribute('data-record')){const m=meetings.find(m=>m.id===selected);if(m?.draft&&!m.audioName){m.recording=true;render();}}
 if(t.hasAttribute('data-stop-recording')){const m=meetings.find(m=>m.id===selected);if(m?.recording){m.recording=false;m.audioName='회의 녹음';startSelected();}}
 if(t.hasAttribute('data-calendar')){const m=meetings.find(m=>m.id===selected);if(!m?.draft)return;showDialog('일정 연결',`<label>일정 이름<input name="title" value="${escape(m.calendar?.title||'팀 회의')}" required autofocus></label><label>일정<input name="date" type="datetime-local" required value="${escape(m.calendar?.date||'2026-10-30T10:00')}"></label>`,()=>{const title=dialog.querySelector('[name="title"]').value.trim(),date=dialog.querySelector('[name="date"]').value;if(!title||!date)return;m.calendar={title,date};m.date=`${date.slice(5,7)}. ${date.slice(8,10)}. ${date.slice(11,16)}`;if(/^새 회의(?: \(\d+\))?$/.test(m.title))m.title=title;render();},{confirm:'연결'});}
 if(t.hasAttribute('data-import')){const m=meetings.find(m=>m.id===selected);if(!m?.draft||m.audioName||m.recording)return;showDialog('음성 가져오기','<label>음성 파일<input type="file" accept="audio/*" required></label>',()=>{const file=dialog.querySelector('input').files[0];if(file){m.audioName=file.name;startSelected();}},{confirm:'가져오기'});}

});
render();

function openRecentMenu(event) {
 const row=event.target.closest('[data-recent-row]');
 const menu=row?.querySelector('[data-meeting-menu]');
 if(!menu)return;
 event.preventDefault();menu.open=true;menus.position(menu);
 menu.querySelector('button:not(:disabled)')?.focus({preventScroll:true});
}
$('recents').addEventListener('contextmenu',openRecentMenu);
$('recents').addEventListener('keydown',event=>{
 if(event.key==='ContextMenu'||event.shiftKey&&event.key==='F10')openRecentMenu(event);
});
$('recents').addEventListener('click',event=>{
 const action=event.target.closest('[data-library-action]');if(!action)return;
 event.preventDefault();event.stopPropagation();
 const id=Number(action.dataset.libraryTarget),meeting=meetings.find(m=>m.id===id);
 if(!meeting||meeting.deletedAt)return;
 menus.close(action.closest('details'),true);
 if(action.dataset.libraryAction==='rename')showDialog('이름 변경',`<label>회의 이름<input name="title" value="${escape(meeting.title)}" required maxlength="200" autofocus></label>`,()=>{const value=dialog.querySelector('input').value.trim();if(value){meeting.title=value;render();}},{confirm:'저장'});
 if(action.dataset.libraryAction==='move')showDialog('폴더로 이동',`<div class="library-folder-choices" role="group" aria-label="대상 폴더">${['',...userFolders].map(f=>`<label class="library-folder-choice"><input type="radio" name="destination" value="${escape(f)}"${(meeting.folder||'')===f?' checked':''}><span>${icon(f?'folder':'inbox')}${escape(f||'미분류')}</span></label>`).join('')}</div>`,()=>{meeting.folder=dialog.querySelector('input[name="destination"]:checked').value||null;render();},{confirm:'이동'});
 if(action.dataset.libraryAction==='trash')showDialog('휴지통으로 이동할까요?',`<p>${escape(meeting.title)}</p>`,()=>{meeting.deletedAt=Date.now();if(selected===id){selected=null;filter='all';}render();},{confirm:'휴지통으로 이동',destructive:true});
});
