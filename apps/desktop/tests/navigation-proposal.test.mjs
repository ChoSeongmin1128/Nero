import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';

const prototype = new URL('../../../design-system/prototype/', import.meta.url);
const bundle = await build({entryPoints:[fileURLToPath(new URL('navigation-review.js',prototype))],bundle:true,write:false,format:'iife'});

async function setup(t) {
  const window = new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true,disableCSSFileLoading:true,disableJavaScriptFileLoading:true}});
  t.after(()=>window.happyDOM.abort());
  window.document.write(await readFile(new URL('navigation-review.html',prototype),'utf8'));
  window.GroveIcons=name=>`<svg data-icon="${name}"></svg>`;
  window.eval(bundle.outputFiles[0].text);
  return window;
}

test('hover reveals the sidebar without duplicate controls; the same button pins and collapses',async t=>{
  const window=await setup(t), doc=window.document;
  const trigger=doc.getElementById('toggleNav'), shell=doc.getElementById('shell');
  assert.equal(doc.querySelectorAll('#toggleNav').length,1);
  assert.equal(doc.getElementById('closeNav'),null);
  doc.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
  assert.ok(shell.classList.contains('nav-peek'));
  assert.equal(trigger.parentElement.className,'navigation-rail-head');
  assert.equal(trigger.getAttribute('aria-label'),'사이드바 고정');
  assert.equal(doc.querySelector('main').inert,false);
  trigger.click();
  assert.ok(shell.classList.contains('nav-pinned'));
  assert.equal(window.localStorage.getItem('nero-proposal-sidebar-pinned'),'true');
  trigger.click();
  assert.ok(shell.classList.contains('collapsed'));
  assert.equal(trigger.parentElement.className,'navigation-rail-head');
  doc.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'touch'}));
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
  const edge=doc.querySelector('.navigation-rail'),aside=doc.querySelector('aside');
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

 test('prototype feedback belongs to the selected meeting and supports retry and dismissal', async t => {
  const window = await setup(t), doc = window.document;
  doc.querySelector('[data-state="cancelled"]').click();
  assert.match(doc.querySelector('.meeting-status').textContent, /음성 기록 변환을 취소/);
  doc.querySelector('[data-retry]').click();
  assert.equal(doc.querySelector('.meeting-status').dataset.status, 'processing');
  doc.querySelector('[data-cancel]').click();
  assert.equal(doc.querySelector('.meeting-status').dataset.status, 'cancelled');
  doc.querySelector('[data-recent-meeting="2"]').click();
  assert.equal(doc.querySelector('.meeting-status'), null);
  doc.querySelector('[data-recent-meeting="1"]').click();
  assert.equal(doc.querySelector('.meeting-status').dataset.status, 'cancelled');
  doc.querySelector('[data-dismiss]').click();
  assert.equal(doc.querySelector('.meeting-status'), null);
});


test('recent right click operates on the targeted meeting without navigating or changing another meeting', async t => {
  const window=await setup(t),doc=window.document;
  doc.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
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
  doc.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter',{pointerType:'mouse'}));
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
