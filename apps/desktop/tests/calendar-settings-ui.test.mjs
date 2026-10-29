import assert from 'node:assert/strict';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';

test('calendar settings renders pending progress, reopens the same browser request and offers retry on failure', async t => {
  const app = fileURLToPath(new URL('../', import.meta.url));
  const bundle = await build({entryPoints:[app+'src/calendar-settings.jsx'],bundle:true,write:false,format:'iife',globalName:'CalendarUI',loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"'}});
  const window = new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true}});
  t.after(()=>window.happyDOM.abort());
  window.document.body.innerHTML='<div id="calendar"></div>';
  window.eval(bundle.outputFiles[0].text);
  const snapshot={revision:1,configured:true,connected:false,connecting:false,restoring:false,calendars:[],selectedCalendars:[],error:null};
  let rejectFinish; const opened=[];
  const panel=window.CalendarUI.mountCalendarSettings({panel:window.document.getElementById('calendar'),bridge:{readCalendarSettings:async()=>snapshot,subscribeCalendarSettings:async()=>()=>{},beginCalendarConnect:async()=>({attemptId:'one'}),finishCalendarConnect:()=>new Promise((_,reject)=>{rejectFinish=reject;}),reopenCalendarConnect:async id=>opened.push(id),cancelCalendarConnect:async()=>({...snapshot,revision:2})}});
  t.after(()=>panel.dispose());
  const wait=async predicate=>{for(let i=0;i<100;i++){if(predicate())return;await new Promise(resolve=>setTimeout(resolve,5));}assert.fail('Expected Calendar UI state');};
  const button=text=>[...window.document.querySelectorAll('button')].find(node=>node.textContent===text);
  panel.setActive(true); await wait(()=>button('Google 연결')&&!button('Google 연결').disabled);
  button('Google 연결').click();await wait(()=>button('브라우저 다시 열기')&&!button('브라우저 다시 열기').disabled);
  assert.match(window.document.body.textContent,/브라우저에서 승인 중/);
  assert.ok(window.document.querySelector('.activity-spinner'));
  button('브라우저 다시 열기').click();await wait(()=>opened.length===1);assert.deepEqual(opened,['one']);
  rejectFinish(Error('승인 시간이 지났습니다.'));await wait(()=>button('다시 시도'));
  assert.equal(button('취소'),undefined);
  assert.match(window.document.querySelector('[role="alert"]').textContent,/승인 시간이/);
});
