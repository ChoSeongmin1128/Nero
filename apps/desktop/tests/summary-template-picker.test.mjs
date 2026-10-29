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
