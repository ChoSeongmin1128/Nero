import assert from 'node:assert/strict';
import test from 'node:test';
import { createViewHistory } from '../src/view-history.js';
const library=(filter='all',query='')=>({kind:'library',filter,view:'folders',query});
const meeting=id=>({kind:'meeting',id});
test('back and forward follow visited views without creating entries for polling or search typing',async()=>{
 const calls=[];const h=createViewHistory({navigate:async route=>{calls.push(route);h.visit(route);return true;}});
 h.visit(library());h.visit(library('all','lookup'));h.visit(meeting('a'));h.visit(meeting('a'));
 assert.equal(await h.go(-1),true);assert.equal(calls[0].query,'lookup');assert.equal(h.status().canBack,false);
 assert.equal(await h.go(1),true);assert.equal(calls[1].id,'a');
});
test('failed saves retain history and a new visit discards only the forward branch',async()=>{
 let blocked=true;const h=createViewHistory({navigate:async()=>{if(blocked)throw Error('save failed');return true;}});
 h.visit(library());h.visit(meeting('a'));await assert.rejects(h.go(-1));assert.equal(h.status().canForward,false);
 blocked=false;await h.go(-1);h.visit(meeting('b'));assert.equal(h.status().canForward,false);
 await h.go(-1);assert.equal(h.status().canBack,false);
});
test('deleted meetings and folders are skipped while a newer direct navigation wins pending restoration',async()=>{
 const missing=new Set();let resolve;const calls=[];let slow=false;
 const h=createViewHistory({canVisit:r=>!missing.has(r.id||r.filter),navigate:async r=>{calls.push(r);if(slow)await new Promise(y=>resolve=y);return true;}});
 h.visit(library());h.visit(meeting('deleted'));h.visit(meeting('last'));missing.add('deleted');await h.go(-1);
 assert.equal(calls[0].kind,'library');
 slow=true;const pending=h.go(1);assert.equal(h.status().busy,true);assert.equal(await h.go(1),false);
 h.visit(meeting('newer'));resolve();assert.equal(await pending,false);assert.equal(h.status().busy,false);
});

test('a cancelled restoration cannot change a newer direct view after a slow save',async()=>{
 let release;let screen='initial';const h=createViewHistory({navigate:async(route,{isCurrent})=>{await new Promise(resolve=>release=resolve);if(!isCurrent())return false;screen=route.id;return true;}});
 h.visit(meeting('a'));h.visit(meeting('b'));const pending=h.go(-1);screen='newer';h.visit(meeting('newer'));release();assert.equal(await pending,false);assert.equal(screen,'newer');
});

test('a new explicit search cancels pending history even when it opens the same target route',async()=>{
 let release;const h=createViewHistory({navigate:async(r,{isCurrent})=>{await new Promise(resolve=>release=resolve);return isCurrent();}});
 h.visit(library());h.visit(meeting('m'));const back=h.go(-1);h.cancel();h.visit(library('all','new query'));release();assert.equal(await back,false);assert.equal(h.status().busy,false);assert.equal(h.status().canBack,true);assert.equal(h.status().canForward,false);
});
