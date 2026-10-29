import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';

const prototype = new URL('../../../design-system/prototype/', import.meta.url);
const bundle = await build({entryPoints:[fileURLToPath(new URL('navigation-review.js',prototype))],bundle:true,write:false,format:'iife'});

async function setup(t, snapshot) {
  const window = new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true,disableCSSFileLoading:true,disableJavaScriptFileLoading:true}});
  t.after(()=>window.happyDOM.abort());
  window.document.write(await readFile(new URL('navigation-review.html',prototype),'utf8'));
  window.NeroIcons=name=>`<svg data-icon="${name}"></svg>`;
  if(snapshot)window.sessionStorage.setItem('nero-notification-demo-v1',snapshot);
  window.eval(bundle.outputFiles[0].text);
  return window;
}

test('hover reveals the sidebar without duplicate controls; the same button pins and collapses',async t=>{
  const window=await setup(t), doc=window.document;
  const trigger=doc.getElementById('toggleNav'), shell=doc.getElementById('shell');
  assert.equal(doc.querySelectorAll('#toggleNav').length,1);
  assert.equal(doc.getElementById('closeNav'),null);
  doc.querySelector('.navigation-edge').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  assert.ok(shell.classList.contains('nav-peek'));
  assert.equal(trigger.parentElement.className,'brand');
  assert.equal(trigger.getAttribute('aria-label'),'사이드바 펼쳐두기');
  assert.equal(doc.querySelector('main').inert,false);
  trigger.click();
  assert.ok(shell.classList.contains('nav-pinned'));
  assert.equal(window.localStorage.getItem('nero-proposal-sidebar-pinned'),'true');
  trigger.click();
  assert.ok(shell.classList.contains('collapsed'));
  assert.equal(trigger.parentElement.className,'navigation-toggle-slot');
  doc.querySelector('.navigation-edge').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'touch'}));
  assert.equal(shell.classList.contains('nav-peek'),false);
});

test('folder and meeting breadcrumbs navigate to the actual parent and preserve root view mode',async t=>{
  const window=await setup(t),doc=window.document;
  doc.querySelector('#breadcrumbs [data-home]').click();
  doc.querySelector('.folder[data-filter="제품팀"]').click();
  assert.equal(doc.querySelector('h1').textContent,'제품팀');
  assert.equal(doc.querySelector('#breadcrumbs [aria-current="page"]').textContent,'제품팀');
  doc.querySelector('.open[data-open="1"]').click();
  assert.match(doc.getElementById('breadcrumbs').textContent,/회의 목록.*제품팀.*제품 검토/);
  doc.querySelector('#breadcrumbs [data-filter="제품팀"]').click();
  assert.equal(doc.querySelector('h1').textContent,'제품팀');
  doc.querySelector('#breadcrumbs [data-home]').click();
  assert.equal(doc.querySelector('h1').textContent,'회의 목록');
  assert.equal(doc.querySelector('[data-mode="folders"]').getAttribute('aria-pressed'),'true');
  assert.equal(doc.querySelector('.recent summary').querySelector('svg').dataset.icon,'chevron-down');
});

test('temporary sidebar closes after pointer exit but stays while entering a search',async t=>{
  const window=await setup(t),doc=window.document,shell=doc.getElementById('shell');
  const edge=doc.querySelector('.navigation-edge'),aside=doc.querySelector('aside');
  edge.dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  aside.dispatchEvent(new window.PointerEvent('pointerleave',{pointerType:'mouse'}));
  await new Promise(resolve=>setTimeout(resolve,220));
  assert.equal(shell.classList.contains('nav-peek'),false);
  edge.dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  doc.getElementById('search').focus();
  aside.dispatchEvent(new window.PointerEvent('pointerleave',{pointerType:'mouse'}));
  await new Promise(resolve=>setTimeout(resolve,220));
  assert.equal(shell.classList.contains('nav-peek'),true);
});

test('notifications retain retry, acknowledgement and cleanup outside the document', async t => {
 const window=await setup(t),doc=window.document;
 const action=name=>doc.querySelector(`[data-flow="${name}"]`).click();
 action('start');action('cancelled');
 assert.equal(doc.querySelector('.meeting-status'),null);
 assert.equal(doc.getElementById('notification-count').textContent,'1');
 doc.getElementById('notifications-open').click();
 assert.match(doc.querySelector('.notification-item').textContent,/취소됨/);
 doc.querySelector('[data-job-retry="1"]').click();
 assert.equal(doc.activeElement.dataset.jobCancel,'1');
 assert.match(doc.querySelector('.notification-item').textContent,/준비 중/);
 doc.querySelector('[data-job-cancel="1"]').click();
 doc.querySelector('[data-notification-acknowledge="1"]').click();
 assert.equal(doc.getElementById('notification-count').hidden,true);
 assert.equal(doc.activeElement.dataset.notificationDismiss,'1');
 doc.querySelector('[data-notification-dismiss="1"]').click();
 assert.equal(doc.querySelector('.notification-item'),null);
 assert.equal(doc.activeElement,doc.querySelector('#page h1'));
 doc.querySelector('[data-recent-meeting="1"]').click();
 assert.equal(doc.querySelector('.meeting-status'),null);
 action('retry');
 assert.equal(doc.querySelector('.meeting-status').dataset.status,'processing');
 action('another');
 assert.equal(doc.querySelector('.meeting-status'),null);
 doc.getElementById('notifications-open').click();
 assert.match(doc.querySelector('.notification-item').textContent,/제품 검토/);
});

test('separate meetings, completion, duplicate and old attempts project one current notification each',async t=>{
 const window=await setup(t),doc=window.document,action=name=>doc.querySelector(`[data-flow="${name}"]`).click();
 action('multiple');doc.getElementById('notifications-open').click();
 assert.equal(doc.querySelectorAll('.notification-item').length,2);
 action('failed');action('retry');action('stale');
 assert.equal(doc.querySelector('[data-notification="1"]').dataset.jobStatus,'processing');
 assert.match(doc.getElementById('flow-status').textContent,/현재 시도와 다른/);
 action('advance');action('advance');action('completed');action('duplicate');
 assert.equal(doc.querySelectorAll('.notification-item').length,2);
 assert.equal(doc.querySelector('[data-notification="1"]').dataset.jobStatus,'completed');
 assert.equal(doc.querySelector('[data-notification="2"]').dataset.jobStatus,'processing');
 assert.equal(doc.querySelector('[data-job-retry="1"]'),null);
 doc.querySelector('[data-notification-acknowledge="1"]').click();
 doc.querySelector('[data-clear-acknowledged]').click();
 assert.equal(doc.querySelector('[data-notification="1"]'),null);
 assert.ok(doc.querySelector('[data-notification="2"]'));
 action('reload');
 assert.equal(doc.querySelector('[data-notification="2"]').dataset.jobStatus,'interrupted');
 assert.ok(doc.querySelector('[data-job-retry="2"]'));
});

test('reload preserves draft title and notes without inventing generated content',async t=>{
 const window=await setup(t),doc=window.document;
 doc.getElementById('new').click();
 for(const [id,value] of [['draft-title','보존할 제목'],['draft-notes','보존할 메모']]){doc.getElementById(id).value=value;doc.getElementById(id).dispatchEvent(new window.Event('input',{bubbles:true}));}
 doc.querySelector('[data-flow="start"]').click();
 const saved=window.sessionStorage.getItem('nero-notification-demo-v1');
 const restored=new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true,disableCSSFileLoading:true,disableJavaScriptFileLoading:true}});
 t.after(()=>restored.happyDOM.abort());
 restored.document.write(await readFile(new URL('navigation-review.html',prototype),'utf8'));
 restored.NeroIcons=name=>`<svg data-icon="${name}"></svg>`;
 restored.sessionStorage.setItem('nero-notification-demo-v1',saved);restored.eval(bundle.outputFiles[0].text);
 restored.document.querySelector('[data-recent-meeting="5"]').click();
 assert.equal(restored.document.getElementById('draft-title').value,'보존할 제목');
 assert.equal(restored.document.getElementById('draft-notes').value,'보존할 메모');
 assert.equal(restored.document.querySelector('.summary'),null);
 restored.document.getElementById('notifications-open').click();
 assert.equal(restored.document.querySelector('[data-notification="5"]').dataset.jobStatus,'interrupted');
});


test('recent right click operates on the targeted meeting without navigating or changing another meeting', async t => {
  const window=await setup(t),doc=window.document;
  doc.querySelector('.navigation-edge').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  doc.getElementById('toggleNav').click();
  const row=()=>doc.querySelector('[data-recent-row="2"]');
  const open=()=>{const event=new window.MouseEvent('contextmenu',{bubbles:true,cancelable:true});row().dispatchEvent(event);assert.equal(event.defaultPrevented,true);};
  const original=doc.querySelector('h1').textContent;
  open();
  assert.equal(row().querySelector('details').open,true);
  assert.equal(doc.activeElement.textContent,'이름 변경');
  assert.equal(doc.querySelector('h1').textContent,original);
  row().querySelector('[data-library-action="rename"]').click();
  const dialog=doc.querySelector('dialog');
  assert.equal(dialog.open,true);
  dialog.querySelector('input').value='수정한 인터뷰';
  dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
  assert.match(row().textContent,/수정한 인터뷰/);
  assert.equal(doc.activeElement,row().querySelector('summary'));
  assert.equal(doc.querySelector('h1').textContent,original);
  open();row().querySelector('[data-library-action="move"]').click();
  dialog.querySelector('input[name="destination"][value="제품팀"]').checked=true;
  dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
  assert.match(row().querySelector('.recent-meeting-meta').textContent,/제품팀/);
  open();row().querySelector('[data-library-action="trash"]').click();
  dialog.close('cancel');await new Promise(resolve=>setImmediate(resolve));
  assert.ok(row());
  open();row().querySelector('[data-library-action="trash"]').click();
  dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
  assert.equal(row(),null);
  assert.equal(doc.querySelector('h1').textContent,original);
  assert.equal(doc.querySelectorAll('[data-recent-row]').length,3);
});

test('recent context menu supports keyboard navigation, Escape and outside dismissal',async t=>{
  const window=await setup(t),doc=window.document;
  doc.querySelector('.navigation-edge').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  doc.getElementById('toggleNav').click();
  const row=doc.querySelector('[data-recent-row="1"]'),trigger=row.querySelector('summary');
  const recent=row.querySelector('[data-recent-meeting]');recent.focus();
  recent.dispatchEvent(new window.KeyboardEvent('keydown',{key:'F10',shiftKey:true,bubbles:true,cancelable:true}));
  assert.equal(doc.activeElement.textContent,'이름 변경');
  doc.activeElement.dispatchEvent(new window.KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}));
  assert.equal(doc.activeElement.textContent,'폴더로 이동');
  doc.activeElement.dispatchEvent(new window.KeyboardEvent('keydown',{key:'End',bubbles:true,cancelable:true}));
  assert.equal(doc.activeElement.textContent,'휴지통으로 이동');
  doc.activeElement.dispatchEvent(new window.KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
  assert.equal(row.querySelector('details').open,false);assert.equal(doc.activeElement,trigger);
  recent.dispatchEvent(new window.MouseEvent('contextmenu',{bubbles:true,cancelable:true}));
  doc.querySelector('main').dispatchEvent(new window.PointerEvent('pointerdown',{bubbles:true}));
  assert.equal(row.querySelector('details').open,false);
});


test('recent metadata and breadcrumb paths use layout and chevrons instead of slash separators',async t=>{
 const window=await setup(t),doc=window.document;
 for(const metadata of doc.querySelectorAll('.recent-meeting-meta'))assert.doesNotMatch(metadata.textContent,/\//);
 assert.doesNotMatch(doc.querySelector('#breadcrumbs').textContent,/\//);
 assert.equal(doc.querySelectorAll('#breadcrumbs .breadcrumb-separator svg[data-icon="chevron-right"]').length,2);
 assert.equal(doc.querySelector('.recent-meeting-meta').querySelectorAll('span').length,2);
});


test('collapsed navigation has no separate action rail or pin icon',async t=>{
 const window=await setup(t),doc=window.document;
 assert.equal(doc.querySelector('.navigation-rail'),null);
 assert.equal(doc.querySelectorAll('#new').length,1);
 assert.ok(doc.querySelector('[data-filter="trash"] svg[data-icon="trash-2"]'));
 assert.equal(doc.querySelector('#notifications-open svg[data-icon="trash-2"]'),null);
 doc.querySelector('.navigation-edge').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
 assert.equal(doc.querySelector('svg[data-icon="pin"]'),null);
 assert.equal(doc.querySelectorAll('#toggleNav').length,1);
});


test('new meeting owns import and notes without prefilled summary or duplicate creation',async t=>{
 const window=await setup(t),doc=window.document;
 assert.equal(doc.querySelector('.create-options'),null);
 assert.equal(doc.querySelector('.sidebar-utilities [data-import]'),null);
 doc.getElementById('new').click();
 assert.equal(doc.querySelectorAll('[data-recent-row]').length,5);
 const id=doc.querySelector('[data-recent-meeting][aria-current="page"]').dataset.recentMeeting;
 assert.equal(doc.querySelector('.summary'),null);
 assert.equal(doc.getElementById('transcriptToggle').hidden,true);
 const title=doc.getElementById('draft-title');title.value='직접 정한 제목';title.dispatchEvent(new window.Event('input',{bubbles:true}));
 const notes=doc.getElementById('draft-notes');notes.value='유지할 메모';notes.dispatchEvent(new window.Event('input',{bubbles:true}));
 doc.querySelector('[data-import]').click();
 const dialog=doc.querySelector('dialog');
 assert.equal(dialog.querySelector('button[value="cancel"]').formNoValidate,true);
 dialog.close('cancel');await new Promise(resolve=>setImmediate(resolve));
 assert.equal(doc.querySelector('[data-import]').isConnected,true);
 doc.querySelector('[data-import]').click();
 Object.defineProperty(dialog.querySelector('input[type="file"]'),'files',{value:[new window.File(['sample'],'검토 음성.wav',{type:'audio/wav'})]});
 dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
 assert.equal(doc.querySelectorAll('[data-recent-row]').length,5);
 assert.equal(doc.querySelector('[data-recent-meeting][aria-current="page"]').dataset.recentMeeting,id);
 assert.equal(doc.getElementById('draft-title').value,'직접 정한 제목');
 assert.equal(doc.getElementById('draft-notes').value,'유지할 메모');
 assert.match(doc.querySelector('.attached-audio').textContent,/검토 음성.wav/);
 assert.equal(doc.querySelector('[data-import]'),null);
 assert.equal(doc.querySelector('.summary'),null);
});

test('calendar attaches to the same meeting and cancel leaves the draft unchanged',async t=>{
 const window=await setup(t),doc=window.document;
 doc.getElementById('new').click();
 const id=doc.querySelector('[data-recent-meeting][aria-current="page"]').dataset.recentMeeting;
 doc.querySelector('[data-calendar]').click();const dialog=doc.querySelector('dialog');
 dialog.querySelector('[name="title"]').value='지난 회의';dialog.querySelector('[name="date"]').value='2026-10-28T09:00';
 dialog.close('cancel');await new Promise(resolve=>setImmediate(resolve));
 assert.equal(doc.getElementById('draft-title').value,'새 회의');
 doc.querySelector('[data-calendar]').click();dialog.querySelector('[name="title"]').value='미리 준비하는 회의';dialog.querySelector('[name="date"]').value='2027-01-05T09:00';
 dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
 assert.equal(doc.querySelectorAll('[data-recent-row]').length,5);
 assert.equal(doc.querySelector('[data-recent-meeting][aria-current="page"]').dataset.recentMeeting,id);
 assert.equal(doc.getElementById('draft-title').value,'미리 준비하는 회의');
 assert.match(doc.querySelector('.draft-heading .metadata').textContent,/2027/);
 doc.getElementById('new').click();doc.getElementById('new').click();
 assert.equal(doc.getElementById('draft-title').value,'새 회의 (1)');
});


test('custom folders survive reload and reset restores only initial folders', async t => {
 const window=await setup(t),doc=window.document;
 for(const title of ['보존 폴더','빈 폴더']){
  doc.querySelector('[data-add-folder]').click();
  const dialog=doc.querySelector('dialog');dialog.querySelector('input').value=title;
  dialog.close('confirm');await new Promise(resolve=>setImmediate(resolve));
 }
 doc.querySelector('#nav [data-filter="보존 폴더"]').click();
 doc.getElementById('new').click();
 const restored=await setup(t,window.sessionStorage.getItem('nero-notification-demo-v1'));
 const page=restored.document;
 for(const title of ['보존 폴더','빈 폴더'])assert.ok(page.querySelector(`#nav [data-filter="${title}"]`));
 page.querySelector('#nav [data-filter="보존 폴더"]').click();
 assert.ok(page.querySelector('.open[data-open="5"]'));
 page.querySelector('[data-flow="reset"]').click();
 assert.equal(page.querySelector('#nav [data-filter="보존 폴더"]'),null);
 assert.equal(page.querySelector('#nav [data-filter="빈 폴더"]'),null);
 assert.ok(page.querySelector('#nav [data-filter="제품팀"]'));
 const saved=JSON.parse(restored.sessionStorage.getItem('nero-notification-demo-v1'));
 assert.deepEqual(saved.userFolders,['제품팀','인터뷰']);
 assert.equal(saved.meetings.length,4);
});
