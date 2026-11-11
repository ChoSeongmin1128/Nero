import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import {build} from 'esbuild';
import {Window} from 'happy-dom';
import {iconRegistrySource,buildIconRegistry} from '../../../scripts/build-icon-registry.mjs';

const root=fileURLToPath(new URL('../../../',import.meta.url));
const registry=await readFile(new URL('../../../design-system/icons.js',import.meta.url),'utf8');
const context=vm.createContext({});
vm.runInContext(registry,context);
const icon=context.NeroIcons;

test('every selected SVG and brand source matches the registry packaged by both renderers',async()=>{
  assert.equal(await iconRegistrySource(),registry);
  await buildIconRegistry({check:true});
});

// These are selected shapes, not a snapshot of every exploratory icon candidate.
test('the shared renderer preserves the chosen core geometry and normalized outline weight',()=>{
  assert.equal(icon.names.length,85);
  assert.match(icon('arrow-up'),/M12 18V6M7\.5 10\.5 12 6l4\.5 4\.5/);
  assert.match(icon('notebook-tabs'),/M8 9\.5v5M12 7v10M16 9\.5v5/);
  assert.match(icon('mic'),/M8\.4 7\.4/);
  assert.match(icon('trash-2'),/M19 6v14/);
  for(const id of icon.names){
    const definition=icon.definition(id);
    assert.ok(definition.body,id);
    if(definition.kind==='stroke'){
      const height=Number(definition.viewBox.split(/\s+/)[3]);
      assert.ok(Math.abs(Number(definition.strokeWidth)*24/height-1.25)<0.00001,id);
    }
  }
  assert.equal(icon.definition('meeting'),icon.definition('notebook-tabs'));
  assert.equal(icon.definition('does-not-exist'),icon.definition('file-text'));
});

test('the AI mark uses the unchanged approved cat while panel states remain distinguishable',async()=>{
  const brand=await readFile(new URL('../brand/logo/nero-symbol-light.svg',import.meta.url),'utf8');
  const [,viewBox]=brand.match(/viewBox="([^"]+)"/);
  const body=brand.match(/<svg\b[^>]*>([\s\S]*)<\/svg>/)[1].trim();
  assert.equal(icon.definition('sparkles').viewBox,viewBox);
  assert.equal(icon.definition('sparkles').body,body);
  assert.equal(icon.definition('sparkles').stroke,'none');
  assert.match(icon('sparkles'),/nero-icon-brand/);
  assert.match(icon('sparkles'),/style="fill:none;stroke:none;stroke-width:0"/);
  const states=['panel-left','panel-left-peek','panel-left-close'].map(id=>icon.definition(id).body);
  assert.equal(new Set(states).size,3);
  assert.ok(states.every(value=>!value.includes('m16 15-3-3')));
});

test('React icons retain native SVG, ref, size, accessible names and the shared geometry',async t=>{
  const entry=`import React from 'react';import {createRoot} from 'react-dom/client';import {ArrowUp,ActivitySpinner} from ${JSON.stringify(root+'design-system/icons-react.jsx')};const ref=React.createRef();window.iconRef=ref;createRoot(document.getElementById('root')).render(<><ArrowUp ref={ref} size={20} className="custom" aria-label="질문 전송"><title>전송</title></ArrowUp><ActivitySpinner size={14} strokeWidth={1.25}/><ArrowUp data-testid="explicit-style" strokeWidth={1.75} style={{strokeWidth:2}}/></>);`;
  const bundle=await build({stdin:{contents:entry,resolveDir:root+'apps/desktop',loader:'jsx'},bundle:true,write:false,format:'iife',jsx:'automatic',nodePaths:[root+'apps/desktop/node_modules'],define:{'process.env.NODE_ENV':'"production"'}});
  const window=new Window({settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true}});
  t.after(()=>window.happyDOM.abort());
  window.document.head.innerHTML='<style>svg {stroke-width:1.6;stroke:red;fill:none}</style>';
  window.document.body.innerHTML='<div id="root"></div>';
  window.eval(bundle.outputFiles[0].text);
  for(let attempt=0;attempt<40&&!window.iconRef?.current;attempt++)await new Promise(resolve=>setTimeout(resolve,5));
  const svg=window.iconRef.current;
  assert.equal(svg.tagName.toLowerCase(),'svg');
  assert.equal(svg.getAttribute('width'),'20');
  assert.equal(svg.getAttribute('height'),'20');
  assert.equal(svg.getAttribute('stroke-width'),'1.25');
  assert.equal(svg.style.strokeWidth,'1.25');
  assert.equal(svg.style.stroke,'currentColor');
  assert.equal(window.getComputedStyle(svg).strokeWidth,'1.25');
  assert.equal(svg.getAttribute('aria-label'),'질문 전송');
  assert.equal(svg.getAttribute('aria-hidden'),null);
  assert.equal(svg.classList.contains('custom'),true);
  assert.equal(svg.querySelector('title').textContent,'전송');
  assert.equal(svg.querySelector('path').getAttribute('d'),'M12 18V6M7.5 10.5 12 6l4.5 4.5');
  const spinner=window.document.querySelector('[data-nero-icon="activity-spinner"]');
  assert.equal(spinner.getAttribute('viewBox'),'0 0 14 14');
  assert.equal(spinner.getAttribute('stroke-width'),String(1.25*14/24));
  assert.equal(spinner.getAttribute('aria-hidden'),'true');
  const explicit=window.document.querySelector('[data-testid="explicit-style"]');
  assert.equal(explicit.getAttribute('stroke-width'),'1.75');
  assert.equal(explicit.style.strokeWidth,'2');
});
