import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const main = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const bridges = await Promise.all(['bridge.js', 'calendar-bridge.js', 'people-bridge.js'].map(name => readFile(new URL(`../src/${name}`, import.meta.url), 'utf8')));
const names = [...new Set(bridges.flatMap(source => [...source.matchAll(/^export (?:async )?(?:const|function) (\w+)/gm)].map(match => match[1])))];
const bundle = await build({ entryPoints: [root+'src/main.js'], bundle: true, write: false, format: 'iife', jsx: 'automatic', nodePaths: [root+'node_modules'], loader: {'.css':'empty'}, define: {'process.env.NODE_ENV':'"production"'},
  plugins: [{name:'speech-native-fixture', setup(builder) {
    builder.onResolve({filter:/^\.\/bridge\.js$/},()=>({path:'bridge',namespace:'fixture'}));
    builder.onLoad({filter:/.*/,namespace:'fixture'},()=>({contents:names.map(name=>`export const ${name}=(...args)=>globalThis.testBridge.${name}(...args);`).join('\n')}));
    builder.onLoad({filter:/[\\/]src[\\/]main\.js$/},()=>({contents:`${main.match(/^import .*;$/gm).join('\n')}\nglobalThis.neroBoot=(async()=>{${main.replace(/^import .*;\r?\n/gm,'')}})();`,loader:'js'}));
  }}],
});
const choices = [
  {id:'nemotron',label:'Nemotron 3.5',description:'녹음 중 빠른 처리를 위한 선택지',modelBytes:612_000_000,ready:true,supported:true},
  {id:'whisperTurbo',label:'Whisper turbo',description:'구간별 처리로 녹음 종료 후 추가 시간이 필요할 수 있습니다.',modelBytes:574_041_195,ready:false,supported:true},
];
const wait = async predicate => { for(let i=0;i<100;i++){if(predicate())return;await new Promise(resolve=>setTimeout(resolve,5));} assert.fail('Speech UI did not settle'); };
async function fixture(t,{select,ready=true,session=null,supported=true,devices=[]}={}) {
  const window = new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true,disableCSSFileLoading:true,disableJavaScriptFileLoading:true}});
  t.after(()=>window.happyDOM.abort()); window.document.write(html);
  for(const name of ['appearance','icons','components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`,import.meta.url),'utf8'));
  const subscriptions={}, calls=[];
  let progress={revision:1,status:'idle',supported:true,modelReady:ready,summarySupported:true,summaryReady:true,speechEngine:'nemotron',speechChoices:choices.map(choice=>({...choice,supported:choice.id==='whisperTurbo'?supported:choice.supported}))};
  const stubs={
    readSnapshot:()=>({revision:1,meetings:session?[{id:'one',title:'회의',notes:'',createdAt:1,status:session.state}]:[],folders:[],session}),
    processingStatus:()=>progress,listDevices:()=>devices,setAutoRecorder(){},listTemplates:()=>[],
    readTerminologySettings:()=>({available:false}),agentConnectionInfo:()=>({available:false,clients:[]}),
    readCalendarSettings:()=>({revision:1,configured:false,connected:false,calendars:[],selectedCalendars:[]}),
    readTranscript:()=>null,readSummary:()=>null,
    setSpeechEngine:async engine=>select?select(engine):({...progress,revision:++progress.revision,speechEngine:engine,modelReady:false}),
    prepareMeetingModels:()=>({...progress,revision:++progress.revision,status:'running',kind:'setup',id:'download',bytes:10,totalBytes:100}),
  };
  window.testBridge=Object.fromEntries(names.map(name=>[name,async(...args)=>{
    if(name==='appUpdateStatus')return{phase:'idle',currentVersion:'0.0.1',currentBuild:'43'};
    if(name.startsWith('subscribe')){subscriptions[name]=args[0];return()=>{};}
    calls.push({name,args});assert.ok(Object.hasOwn(stubs,name),`Unexpected native call ${name}`);return stubs[name](...args);
  }]));
  window.eval(bundle.outputFiles[0].text);await window.neroBoot;
  const get=id=>window.document.getElementById(id);
  const radio=(place,id)=>get(`${place}-speech-picker`).querySelector(`input[value="${id}"]`);
  return {window,get,radio,calls,subscriptions,progress};
}

test('onboarding and settings share both choices and wait for native selection before enabling download',async t=>{
  let finish;const pending=new Promise(resolve=>finish=resolve);
  const f=await fixture(t,{ready:false,select:()=>pending});
  for(const place of ['setup','settings']){
    assert.equal(f.get(`${place}-speech-picker`).querySelectorAll('input[type=radio]').length,2);
    assert.match(f.get(`${place}-speech-picker`).textContent,/모델 파일 574 MB/);
    assert.match(f.get(`${place}-speech-picker`).textContent,/추가 시간/);
  }
  f.radio('setup','whisperTurbo').focus();
  f.radio('setup','whisperTurbo').click();
  f.window.document.body.focus();
  assert.equal(f.radio('setup','nemotron').checked,true);
  assert.equal(f.radio('settings','nemotron').checked,true);
  assert.equal(f.get('setup-download').disabled,true);
  assert.equal(f.get('settings-speech-picker').querySelector('fieldset').disabled,true);
  finish({...f.progress,revision:2,speechEngine:'whisperTurbo',modelReady:false});
  await wait(()=>f.radio('settings','whisperTurbo').checked&&!f.get('setup-download').disabled);
  assert.equal(f.window.document.activeElement.id,'setup-speech-picker-engine-whisperTurbo');
  assert.equal(f.calls.filter(call=>call.name==='setSpeechEngine').length,1);
  assert.equal(f.calls.some(call=>call.name==='prepareMeetingModels'||call.name==='transcribeAudio'),false);
  f.get('setup-download').click();await wait(()=>f.calls.some(call=>call.name==='prepareMeetingModels'));
  assert.deepEqual(f.calls.find(call=>call.name==='prepareMeetingModels').args,[false]);
});

test('native selection rejection preserves the old choice in both views and displays the error',async t=>{
  const f=await fixture(t,{select:()=>{throw Error('다른 작업이 시작되었습니다.');}});
  f.radio('settings','whisperTurbo').click();
  await wait(()=>f.get('settings-speech-picker').textContent.includes('다른 작업이 시작되었습니다.'));
  for(const place of ['setup','settings']){
    assert.equal(f.radio(place,'nemotron').checked,true);
    assert.equal(f.radio(place,'whisperTurbo').checked,false);
  }
  assert.equal(f.calls.some(call=>call.name==='prepareMeetingModels'||call.name==='transcribeAudio'),false);
});

test('unsupported choices and recording lock selection without invoking a change',async t=>{
  const f=await fixture(t,{supported:false,devices:[{id:"input",name:"마이크",isDefault:true}]});
  assert.equal(f.radio('settings','whisperTurbo').disabled,true);
  f.radio('settings','whisperTurbo').click();
  assert.equal(f.calls.some(call=>call.name==='setSpeechEngine'),false);
  f.subscriptions.subscribeSnapshots({revision:2,meetings:[{id:'one',title:'회의',notes:'',createdAt:1,status:'recording'}],folders:[],session:{id:'capture',meetingId:'one',state:'recording',elapsedMs:1000}});
  await wait(()=>f.get('settings-speech-picker').querySelector('fieldset').disabled);
  assert.match(f.get('settings-speech-picker').textContent,/녹음을 마친 뒤/);
  assert.equal(f.get('settings-microphone-hint').hidden,false);
  assert.equal(f.get('settings-message').hidden,true);
  assert.equal(f.get('auto-recorder').disabled,false);
  const trigger=()=>f.window.document.querySelector('[role=combobox][data-field="settings-microphone"]');
  await wait(()=>trigger()?.getAttribute('aria-describedby')==='settings-microphone-hint');
  assert.equal(trigger().disabled,true);
  f.subscriptions.subscribeSnapshots({revision:3,meetings:[],folders:[],session:null});
  await wait(()=>f.get('settings-microphone-hint').hidden&&!trigger()?.hasAttribute('aria-describedby'));
  assert.equal(trigger().disabled,false);
});


test('model choices keep visible native radio indicators while appearance uses its custom indicator',async t=>{
  const f=await fixture(t);
  const style=f.window.document.createElement('style');
  style.textContent=await readFile(new URL('../../../design-system/components.css',import.meta.url),'utf8')+'\n'+await readFile(new URL('../src/speech-model-picker.css',import.meta.url),'utf8');
  f.window.document.head.append(style);
  const native=f.window.getComputedStyle(f.radio('settings','nemotron'));
  assert.notEqual(native.position,'absolute');
  assert.notEqual(native.clipPath,'inset(50%)');
  const appearance=f.window.getComputedStyle(f.window.document.querySelector('.brightness-choice input'));
  assert.equal(appearance.position,'absolute');
  assert.equal(appearance.clipPath,'inset(50%)');
});
