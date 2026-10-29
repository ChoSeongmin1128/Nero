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
