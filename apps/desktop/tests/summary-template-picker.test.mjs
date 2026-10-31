import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountSummaryTemplatePicker } from '../src/summary-template-picker.js';

test('template selection reads current CLI edits and recovers deleted selection without silently executing another template',async t=>{
 const window=new Window();t.after(()=>window.happyDOM.abort());
 const select=window.document.createElement('select');
 let revision='v1',deleted=false;
 const bridge={listTemplates:async()=>[{id:'meeting',name:'회의',revision:'v1'}],getTemplate:async()=>{if(deleted)throw Error('삭제된 템플릿');return{id:'meeting',revision};}};
 const picker=mountSummaryTemplatePicker({node:select,bridge,storage:{getItem(){throw Error('blocked');},setItem(){throw Error('blocked');}}});
 await picker.reload();revision='v2';assert.deepEqual(await picker.selected(),{id:'meeting',revision:'v2'});
 select.dispatchEvent(new window.Event('change'));
 deleted=true;await assert.rejects(()=>picker.selected(),/삭제된 템플릿/);
 assert.equal(select.disabled,false);
});

test('deleted stored selection requires an explicit choice and never falls back to another template', async t => {
 const window=new Window();t.after(()=>window.happyDOM.abort());
 const select=window.document.createElement('select');let fetched=0,stored='deleted';const availability=[];
 const bridge={listTemplates:async()=>[{id:'meeting',name:'회의',revision:'r1'}],getTemplate:async id=>{fetched++;return{id,revision:'r1'};}};
 const picker=mountSummaryTemplatePicker({node:select,bridge,storage:{getItem:()=>stored,setItem:(_,value)=>{stored=value;}},onAvailabilityChange:value=>availability.push(value)});
 await picker.reload();assert.equal(select.value,'');assert.equal(select.disabled,false);assert.equal(picker.isAvailable(),false);
 await assert.rejects(()=>picker.selected(),/다시 선택/);assert.equal(fetched,0);assert.equal(stored,'deleted');
 select.value='meeting';select.dispatchEvent(new window.Event('change'));
 assert.equal(picker.isAvailable(),true);assert.equal(availability.at(-1),true);
 assert.deepEqual(await picker.selected(),{id:'meeting',revision:'r1'});
});

test('an empty catalog disables selection and restored defaults still require explicit re-selection', async t => {
 const window=new Window();t.after(()=>window.happyDOM.abort());
 const select=window.document.createElement('select');let templates=[],fetched=0;
 const picker=mountSummaryTemplatePicker({node:select,bridge:{listTemplates:async()=>templates,getTemplate:async id=>{fetched++;return{id,revision:'r2'};}},storage:{getItem:()=> 'meeting',setItem(){}}});
 await picker.reload();assert.equal(select.disabled,true);assert.equal(picker.isAvailable(),false);
 assert.equal(select.options[0].textContent,'템플릿 없음');
 await assert.rejects(()=>picker.selected(),/템플릿이 없습니다/);assert.equal(fetched,0);
 templates=[{id:'meeting',name:'회의',revision:'r2'}];await picker.reload();
 assert.equal(select.disabled,false);assert.equal(picker.isAvailable(),false);
 await assert.rejects(()=>picker.selected(),/다시 선택/);
 select.value='meeting';select.dispatchEvent(new window.Event('change'));
 assert.deepEqual(await picker.selected(),{id:'meeting',revision:'r2'});
});

test('deletion detected at execution refreshes the options without selecting another prompt', async t => {
 const window=new Window();t.after(()=>window.happyDOM.abort());
 const select=window.document.createElement('select');let deleted=false;const fetched=[];
 const picker=mountSummaryTemplatePicker({node:select,bridge:{listTemplates:async()=>deleted?[{id:'custom',name:'다른 템플릿'}]:[{id:'meeting',name:'회의'},{id:'custom',name:'다른 템플릿'}],getTemplate:async id=>{fetched.push(id);throw Error('삭제된 템플릿');}},storage:{getItem:()=> 'meeting',setItem(){}}});
 await picker.reload();deleted=true;
 await assert.rejects(()=>picker.selected(),/삭제된 템플릿/);
 assert.equal(picker.isAvailable(),false);assert.equal(select.value,'');assert.deepEqual(fetched,['meeting']);
});
