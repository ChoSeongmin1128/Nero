import test from 'node:test';
import assert from 'node:assert/strict';
import { Window } from 'happy-dom';
import { returnFocusTarget } from '../src/focus-return.js';
test('a closing dialog returns to accessible navigation when its sidebar origin became inert or hidden',async t=>{
 const window=new Window();t.after(()=>window.happyDOM.abort());
 window.document.body.innerHTML='<aside><button id="origin">설정</button></aside><button id="nav">탐색</button>';
 const origin=window.document.getElementById('origin'),nav=window.document.getElementById('nav'),aside=origin.parentElement;
 assert.equal(returnFocusTarget(origin,nav),origin);
 aside.inert=true;assert.equal(returnFocusTarget(origin,nav),nav);
 aside.inert=false;aside.style.visibility='hidden';assert.equal(returnFocusTarget(origin,nav),nav);
 aside.style.visibility='visible';assert.equal(returnFocusTarget(origin,nav),origin);
});
