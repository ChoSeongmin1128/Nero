import { mountNavigationSidebar } from './navigation-sidebar.js';
import { recentMeetingRow } from '../navigation.mjs';

const $=id=>document.getElementById(id), icon=GroveIcons;
const meetings=[{id:1,title:'제품 검토 및 다음 배포 일정 논의',folder:'제품팀',date:'10. 29. 10:30',length:'52:17'},{id:2,title:'고객 인터뷰 - 사용 경험과 개선 요청',folder:'인터뷰',date:'10. 28. 14:00',length:'38:12'},{id:3,title:'운영 개선 사항 검토',folder:'제품팀',date:'10. 27. 11:00',length:'24:08'},{id:4,title:'새로운 아이디어 메모',folder:null,date:'10. 26. 16:00',length:'12:40'}];
let mode=localStorage.getItem('nero-proposal-mode')||'folders',filter='all',selected=1,transcript=false,processingState='normal';
$('toggleNav').innerHTML=icon('panel-left-close'); $('rail-new').innerHTML=icon('plus'); $('rail-home').innerHTML=icon('notebook-pen'); $('rail-settings').innerHTML=icon('settings');
const userFolders=['제품팀','인터뷰'];
const meetingStates=new Map();
const escape=value=>String(value??'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const navigation=mountNavigationSidebar({shell:$('shell'),aside:document.querySelector('aside'),content:document.querySelector('main'),trigger:$('toggleNav'),backdrop:document.querySelector('.backdrop')});
const syncNav=()=>navigation.render();
addEventListener('resize',syncNav);
document.querySelector('.recent-chevron').innerHTML=icon('chevron-down');
const recent=document.querySelector('.recent');recent.open=localStorage.getItem('nero-proposal-recents')!=='false';recent.ontoggle=()=>localStorage.setItem('nero-proposal-recents',recent.open);

function render(){ syncNav();
processingState=meetingStates.get(selected)||'normal';
document.querySelectorAll('[data-state]').forEach(b=>b.setAttribute('aria-pressed',b.dataset.state===processingState));
$('recents').innerHTML=meetings.map(m=>recentMeetingRow({id:m.id,title:m.title,metadata:[m.folder||'미분류',m.date,m.length],active:selected===m.id,escape})).join('');
const meeting=meetings.find(item=>item.id===selected);
const folder=meeting ? (meeting.folder||'unfiled') : filter;
const folderName=folder==='unfiled'?'미분류':folder==='trash'?'휴지통':folder;
$('breadcrumbs').innerHTML=`<button class="quiet" data-home>회의 목록</button>${folder!=='all'?`<span aria-hidden="true">/</span><button class="quiet" data-filter="${folder}" ${selected?'':'aria-current="page"'}>${folderName}</button>`:''}${meeting?`<span aria-hidden="true">/</span><span class="breadcrumb-current" aria-current="page" title="${meeting.title}">${meeting.title}</span>`:''}`;
const navItem=([id,name,i,count])=>`<button data-filter="${id}" ${!selected&&filter===id?'aria-current="page"':''}>${icon(i)}${name}<span class="count">${count}</span></button>`;
$('nav').innerHTML=`<div class="library-root">${navItem(['all','회의 목록','book-open',meetings.length])}<button class="quiet add-folder" data-add-folder aria-label="새 폴더" title="새 폴더">${icon('plus')}</button></div><div class="library-children">${navItem(['unfiled','미분류','inbox',meetings.filter(m=>!m.folder).length])}${userFolders.map(f=>navItem([escape(f),escape(f),'folder',meetings.filter(m=>m.folder===f).length])).join('')}</div>`;$('transcriptToggle').hidden=!selected;$('transcriptToggle').setAttribute('aria-expanded',transcript);
if(selected){const m=meetings.find(m=>m.id===selected);$('page').innerHTML=`<h1>${m.title}</h1><p class="metadata">2026. ${m.date} / ${m.folder||'미분류'}</p>${statusMarkup()}<div class="player"><button class="quiet icon" id="play" aria-label="예시 재생 상태">${icon('play')}</button><span>0:00</span><input type="range" aria-label="재생 위치" min="0" max="3137" value="0"><span>${m.length}</span><details class="speed"><summary>1x</summary><div class="speed-menu">${[.75,1,1.25,1.5,2].map(n=>`<button data-speed="${n}">${n}x${n===1?' 선택':''}</button>`).join('')}</div></details></div><div class="detail-body"><article class="summary"><h2>요약</h2><section><h2>핵심</h2><p>녹음 안정성과 오류 복구를 우선 개선하고, 배포 전 긴 회의에서 다시 확인합니다.</p></section><section><h2>결정 사항</h2><ul><li>금요일까지 오류를 수정한 뒤 다음 주 배포 여부를 결정합니다. <button class="citation" data-cite>18:42</button></li><li>회의 요약을 기본 화면으로 유지합니다.</li></ul></section><section><h2>할 일</h2><ul><li>첫 녹음 권한과 오류 복구 흐름 점검</li><li>긴 회의 처리와 저장 상태 재검증</li></ul></section></article><section class="transcript" ${transcript?'':'hidden'}><header><h2>음성 기록</h2><button class="quiet icon" data-close-transcript aria-label="음성 기록 닫기">${icon('x')}</button></header><p class="muted">전체 기록</p><div class="utterance"><small>18:20 / 화자 1</small>지금 버전에서 녹음이 끊기는 이슈가 가끔 발생하는 것 같습니다.</div><div class="utterance active"><small>18:42 / 화자 2</small>금요일까지 오류부터 정리하고, 다음 주에 배포할 수 있을지 다시 보죠.</div><div class="utterance"><small>19:06 / 화자 1</small>그럼 요약 화면은 지금 구조를 유지하겠습니다.</div><div class="utterance"><small>20:15 / 화자 3</small>추가로 긴 회의에서도 안정적으로 처리되는지 확인이 필요합니다.</div></section></div>`;return;}
const query=$('search').value.toLowerCase();const list=meetings.filter(m=>(filter==='all'||(filter==='unfiled'?!m.folder:m.folder===filter))&&m.title.toLowerCase().includes(query));
const folders=filter==='all'&&mode==='folders'&&!query;
$('page').innerHTML=`<div class="heading"><div><h1>${filter==='all'?'회의 목록':filter==='unfiled'?'미분류':filter==='trash'?'휴지통':filter}</h1><small>${list.length}개 회의</small></div>${filter==='all'?`<div class="segmented" aria-label="회의 보기"><button data-mode="folders" aria-pressed="${mode==='folders'}">폴더별</button><button data-mode="all" aria-pressed="${mode==='all'}">전체 회의</button></div>`:''}</div>${folders?`<div class="folders">${userFolders.map(f=>`<button class="folder" data-filter="${f}">${icon('folder')}<div><strong>${f}</strong><small>${meetings.filter(m=>m.folder===f).length}개 회의</small></div></button>`).join('')}</div><p class="section-label">미분류</p>`:''}${list.filter(m=>!folders||!m.folder).map(m=>`<div class="row">${icon('audio-lines')}<button class="open" data-open="${m.id}"><strong title="${m.title}">${m.title}</strong><small>${m.date} / ${m.length}${!folders?' / '+(m.folder||'미분류'):''}</small></button><button class="quiet icon" data-open="${m.id}" aria-label="${m.title} 열기">${icon('arrow-right')}</button></div>`).join('')||'<p class="muted">회의가 없습니다.</p>'}`;
}
document.addEventListener('click',e=>{const t=e.target.closest('button');if(!t)return;if(t.hasAttribute('data-home')){selected=null;filter='all';render();}if(t.dataset.open||t.dataset.recentMeeting){selected=Number(t.dataset.open||t.dataset.recentMeeting);transcript=false;navigation.navigate();render();}if(t.dataset.filter){filter=t.dataset.filter;selected=null;navigation.navigate();render();}if(t.dataset.mode){mode=t.dataset.mode;localStorage.setItem('nero-proposal-mode',mode);render();}if(t.hasAttribute('data-cite')){transcript=true;render();}if(t.hasAttribute('data-close-transcript')){transcript=false;render();$('transcriptToggle').focus();}if(t.dataset.speed){t.closest('details').querySelector('summary').textContent=t.dataset.speed+'x';t.closest('details').open=false;}if(t.id==='play'){t.innerHTML=icon(t.getAttribute('aria-pressed')==='true'?'play':'pause');t.setAttribute('aria-pressed',t.getAttribute('aria-pressed')!=='true');}});
$('search').oninput=()=>{selected=null;filter='all';render();};$('transcriptToggle').onclick=()=>{transcript=!transcript;render();};$('width').onclick=()=>{$('shell').classList.toggle('half');navigation.resetTemporary();$('width').textContent=$('shell').classList.contains('half')?'전체 너비':'절반 너비';syncNav();};$('theme').onclick=()=>{const dark=document.documentElement.dataset.mode==='dark';document.documentElement.dataset.mode=dark?'light':'dark';$('theme').textContent=dark?'어둡게':'밝게';};document.addEventListener('keydown',e=>{if(e.key==='Escape'){if(transcript){transcript=false;render();$('transcriptToggle').focus();}else {navigation.resetTemporary();}}});$('new').innerHTML=icon('plus')+'<span>새 회의</span>';
document.querySelector('[data-chevron]').innerHTML=icon('chevron-down');
document.querySelector('[data-import]').insertAdjacentHTML('afterbegin',icon('upload'));
document.querySelector('.sidebar-utilities [data-filter]').insertAdjacentHTML('afterbegin',icon('trash-2'));
$('settings').insertAdjacentHTML('afterbegin',icon('settings'));
function statusMarkup(){
 if(processingState==='normal')return '';
 const copy={processing:'음성을 텍스트로 변환하고 있습니다.',cancelled:'음성 기록 변환을 취소했습니다.',failed:'텍스트로 변환하지 못했습니다.'}[processingState];
 return `<div class="meeting-status" role="status" data-status="${processingState}"><span class="status-mark"></span><span>${copy}</span><div class="status-actions">${processingState==='processing'?'<button class="quiet" data-cancel>취소</button>':'<button class="quiet" data-retry>다시 시도</button><button class="quiet icon" data-dismiss aria-label="알림 닫기">'+icon('x')+'</button>'}</div></div>`;
}
function setState(state){processingState=state;if(!selected)selected=1;meetingStates.set(selected,state);document.querySelectorAll('[data-state]').forEach(b=>b.setAttribute('aria-pressed',b.dataset.state===state));render();}
const dialog=document.querySelector('.prototype-dialog');let dialogAction=null;
function showDialog(title,content,action){dialog.querySelector('[data-dialog-title]').textContent=title;dialog.querySelector('[data-dialog-content]').innerHTML=content;dialogAction=action;dialog.showModal();}
dialog.addEventListener('close',()=>{if(dialog.returnValue==='confirm')dialogAction?.();});
function createMeeting(title='새 회의'){const id=Math.max(...meetings.map(m=>m.id))+1;meetings.unshift({id,title,folder:null,date:'10. 29. 15:00',length:'0:00'});selected=id;processingState='normal';transcript=false;render();}
$('new').onclick=()=>createMeeting();$('rail-new').onclick=()=>createMeeting();
$('settings').onclick=$('rail-settings').onclick=()=>showDialog('설정','<label>화면 모드<select name="appearance"><option value="dark">어둡게</option><option value="light">밝게</option></select></label>',()=>{document.documentElement.dataset.mode=dialog.querySelector('select').value;});
document.addEventListener('click',e=>{const t=e.target.closest('button');if(!t)return;
 if(t.dataset.state)setState(t.dataset.state);
 if(t.hasAttribute('data-retry'))setState('processing');
 if(t.hasAttribute('data-cancel'))setState('cancelled');
 if(t.hasAttribute('data-dismiss'))setState('normal');
 if(t.hasAttribute('data-add-folder'))showDialog('폴더 추가','<label>폴더 이름<input name="folder" placeholder="폴더 이름" required autofocus maxlength="40"></label>',()=>{const value=dialog.querySelector('input').value.trim();if(value&&!userFolders.includes(value)){userFolders.push(value);selected=null;filter=value;render();}});
 if(t.hasAttribute('data-calendar')){t.closest('details').open=false;showDialog('일정으로 만들기','<label>회의 이름<input name="title" value="팀 회의" required autofocus></label><label>일정<input type="datetime-local" value="2026-10-30T10:00"></label>',()=>createMeeting(dialog.querySelector('input').value.trim()||'팀 회의'));}
 if(t.hasAttribute('data-import'))showDialog('오디오 가져오기','<label>오디오 파일<input type="file" accept="audio/*"></label>',()=>{const file=dialog.querySelector('input').files[0];if(file){createMeeting(file.name);setState('processing');}});
});
render();
