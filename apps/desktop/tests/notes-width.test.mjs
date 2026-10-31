import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountNotesWidth, notesWidthPreference, notesWidthBounds } from '../src/notes-width.js';

function page(t) {
  const window = new Window({url:'http://nero.test'});t.after(()=>window.happyDOM.abort());
  window.document.body.innerHTML='<section id="root"><div id="handle" tabindex="0"></div><textarea>Keep notes</textarea></section>';
  const root=window.document.getElementById('root'),handle=window.document.getElementById('handle');
  let available=1000;root.getBoundingClientRect=()=>({width:available});
  const width=mountNotesWidth({root,handle});t.after(()=>width.destroy());width.update(true);
  const pointer=(name,x)=>handle.dispatchEvent(new window.PointerEvent(name,{pointerId:1,isPrimary:true,button:0,clientX:x,bubbles:true,cancelable:true}));
  return {window,root,handle,width,pointer,resize(value){available=value;window.dispatchEvent(new window.Event('resize'));}};
}
test('notes width reserves readable body space and rejects invalid preferences',()=>{
  assert.deepEqual(notesWidthBounds(760),{min:240,max:340});
  for(const value of [null,undefined,' ',{},true,-1,Infinity])assert.equal(notesWidthPreference(value),320);
});
test('dragging the left edge grows notes, clamps against the body and saves once released',t=>{
  const p=page(t);p.pointer('pointerdown',500);p.pointer('pointermove',50);p.pointer('pointerup',50);
  assert.equal(p.handle.getAttribute('aria-valuenow'),'560');assert.equal(p.window.localStorage.getItem('nero.notes.width'),'560');
  assert.equal(p.root.querySelector('textarea').value,'Keep notes');
  p.resize(760);assert.equal(p.handle.getAttribute('aria-valuenow'),'340');assert.equal(p.window.localStorage.getItem('nero.notes.width'),'560');
  p.resize(1200);assert.equal(p.handle.getAttribute('aria-valuenow'),'560');
});
test('Escape, pointer cancellation and blur discard a partial resize without altering preference',t=>{
  const p=page(t);
  for(const cancel of [()=>p.window.document.dispatchEvent(new p.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true})),()=>p.pointer('pointercancel',400),()=>p.window.dispatchEvent(new p.window.Event('blur'))]){
    p.pointer('pointerdown',500);p.pointer('pointermove',400);cancel();
    assert.equal(p.handle.getAttribute('aria-valuenow'),'320');assert.equal(p.window.localStorage.getItem('nero.notes.width'),null);
    assert.equal(p.width.isDragging(),false);
  }
});
test('keyboard resizing follows the edge direction and closed notes ignore input',t=>{
  const p=page(t);p.handle.dispatchEvent(new p.window.KeyboardEvent('keydown',{key:'ArrowLeft',cancelable:true}));
  assert.equal(p.handle.getAttribute('aria-valuenow'),'328');
  p.width.update(false);p.pointer('pointerdown',500);p.pointer('pointerup',200);assert.equal(p.handle.hidden,true);
  assert.equal(p.window.localStorage.getItem('nero.notes.width'),'328');
  p.width.update(true);p.handle.dispatchEvent(new p.window.MouseEvent('dblclick'));assert.equal(p.handle.getAttribute('aria-valuenow'),'320');
});
