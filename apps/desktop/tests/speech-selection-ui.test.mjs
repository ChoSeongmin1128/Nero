import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { markdownSource } from '../scripts/build-markdown.mjs';
const markdownBundle = await markdownSource();

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
async function fixture(t,{select,toggle,ready=true,session=null,supported=true,devices=[],speech={}}={}) {
  const window = new Window({url:'http://nero.test',settings:{enableJavaScriptEvaluation:true,suppressInsecureJavaScriptEnvironmentWarning:true,disableCSSFileLoading:true,disableJavaScriptFileLoading:true}});
  t.after(()=>window.happyDOM.abort()); window.document.write(html);
  window.eval(markdownBundle);
  for(const name of ['appearance','icons','components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`,import.meta.url),'utf8'));
  const subscriptions={}, calls=[];
  let progress={revision:1,status:'idle',supported:true,modelReady:ready,summarySupported:true,summaryReady:true,speechEngine:'nemotron',speechChoices:choices.map(choice=>({...choice,supported:choice.id==='whisperTurbo'?supported:choice.supported})),...speech};
  const stubs={
    readSnapshot:()=>({revision:1,meetings:session?[{id:'one',title:'회의',notes:'',createdAt:1,status:session.state}]:[],folders:[],session}),
    processingStatus:()=>progress,listDevices:()=>devices,setAutoRecorder(){},listTemplates:()=>[],
    readTerminologySettings:()=>({available:false}),agentConnectionInfo:()=>({available:false,clients:[]}),
    readCalendarSettings:()=>({revision:1,configured:false,connected:false,calendars:[],selectedCalendars:[]}),
    readTranscript:()=>null,readSummary:()=>null,
    requestSpeechEngine:async engine=>select?select(engine):({...progress,revision:++progress.revision,speechEngine:engine,modelReady:false}),
    cancelSpeechSwitch:async()=>({...progress,revision:++progress.revision,speechSwitch:null}),
    respondSpeechOffer:async accept=>({...progress,revision:++progress.revision,speechOffer:null}),
    setSpeakerEnabled:async enabled=>toggle?toggle(enabled):({...progress,revision:++progress.revision,speakerEnabled:enabled}),
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
    assert.match(f.get(`${place}-speech-picker`).textContent,/음성 처리 모델/);
    assert.doesNotMatch(f.get(`${place}-speech-picker`).textContent,/모델 파일|MB|GB|GPU|CPU/);
    assert.match(f.get(`${place}-speech-picker`).textContent,/속도와 정확도 균형/);
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
  assert.equal(f.calls.filter(call=>call.name==='requestSpeechEngine').length,1);
  assert.equal(f.calls.some(call=>call.name==='prepareMeetingModels'||call.name==='transcribeAudio'),false);
  f.get('setup-download').click();await wait(()=>f.calls.some(call=>call.name==='prepareMeetingModels'));
  assert.deepEqual(f.calls.find(call=>call.name==='prepareMeetingModels').args,[false]);
});

test('native selection rejection preserves the old choice in both views and displays the error',async t=>{
  const f=await fixture(t,{select:()=>{throw Error('다른 작업이 시작되었습니다.');}});
  f.radio('settings','whisperTurbo').click();
  // A working model stays in use: a model that still needs a download asks first.
  assert.equal(f.calls.some(call=>call.name==='requestSpeechEngine'),false);
  assert.equal(f.get('settings-speech-picker-confirm').closest('.speech-model-confirm').hidden,false);
  f.get('settings-speech-picker-confirm').click();
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
  assert.equal(f.calls.some(call=>call.name==='requestSpeechEngine'),false);
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


test('speaker switch keeps confirmed state in setup and settings until native success', async t => {
  let finish;
  const pending = new Promise(resolve => finish = resolve);
  const f = await fixture(t, { ready: false, toggle: () => pending, speech: { speakerEnabled: true, speakerReady: false, speakerSupported: true } });
  const toggle = place => f.get(`${place}-speech-picker-speaker-enabled`);
  toggle('setup').click();
  assert.equal(toggle('setup').checked, true);
  assert.equal(toggle('settings').checked, true);
  assert.equal(f.get('settings-speech-picker').querySelector('fieldset').disabled, true);
  finish({ ...f.progress, revision: 2, speakerEnabled: false, modelReady: true });
  await wait(() => !toggle('settings').checked && !toggle('settings').disabled);
  assert.equal(toggle('setup').checked, false);
  assert.equal(f.get('setup-download').hidden, true);
  assert.equal(f.get('settings-speech-picker-prepare').hidden, true);
  assert.equal(f.calls.filter(call => call.name === 'setSpeakerEnabled').length, 1);
  assert.equal(f.calls.some(call => ['prepareMeetingModels', 'transcribeAudio'].includes(call.name)), false);
});

test('rejected speaker preference preserves the selection without preparing models', async t => {
  const f = await fixture(t, { toggle: () => { throw Error('설정을 저장하지 못했습니다.'); }, speech: { speakerEnabled: false } });
  f.get('settings-speech-picker-speaker-enabled').click();
  await wait(() => f.get('settings-speech-picker').textContent.includes('설정을 저장하지 못했습니다.'));
  assert.equal(f.get('settings-speech-picker-speaker-enabled').checked, false);
  assert.equal(f.get('setup-speech-picker-speaker-enabled').checked, false);
  assert.equal(f.calls.some(call => call.name === 'prepareMeetingModels'), false);
});

test('Apple speech prepares system assets only after an explicit prepare action', async t => {
  const f = await fixture(t, { ready: false, speech: {
    speechEngine: 'appleSpeech', speakerEnabled: false,
    speechChoices: [...choices, { id: 'appleSpeech', label: 'macOS 기본 전사', supported: true, ready: false, modelBytes: 0 }],
  } });
  assert.equal(f.radio('setup', 'appleSpeech').checked, true);
  assert.match(f.get('setup-speech-picker').textContent, /언어 데이터 준비 필요/);
  assert.equal(f.get('setup-download').textContent, '준비');
  assert.doesNotMatch(f.get('setup-speech-picker').textContent, /MB|GB|GPU|CPU/);
  assert.equal(f.calls.some(call => call.name === 'prepareMeetingModels'), false);
  f.get('setup-download').click();
  await wait(() => f.calls.some(call => call.name === 'prepareMeetingModels'));
  assert.deepEqual(f.calls.find(call => call.name === 'prepareMeetingModels').args, [false]);
});

test('hardware recommendation shows the optional full model without changing or downloading the selected model', async t => {
  const f = await fixture(t, { speech: { speechChoices: [...choices, { id: 'whisperLargeV3', label: 'Whisper large-v3', description: '정확도 우선', modelBytes: 1081140203, ready: false, supported: true, recommended: true, recommendationReason: '이 Mac에서 정확도 우선 모델을 사용할 수 있습니다.' }] } });
  for (const place of ['setup', 'settings']) {
    assert.equal(f.radio(place, 'nemotron').checked, true);
    assert.equal(f.radio(place, 'whisperLargeV3').checked, false);
    assert.match(f.get(`${place}-speech-picker`).textContent, /Whisper large-v3 \(추천\)/);
    assert.match(f.get(`${place}-speech-picker`).textContent, /정확도 우선 모델/);
  }
  assert.equal(f.calls.some(call => ['requestSpeechEngine', 'prepareMeetingModels', 'transcribeAudio'].includes(call.name)), false);
});

test('Qwen on a low-memory Mac stays selectable but needs an explicit acknowledgement before download', async t => {
  const qwen = { id: 'qwenKo', label: 'Qwen3-ASR 한국어', modelBytes: 4_400_000_000, ready: false, supported: true,
    memoryWarning: '이 Mac 메모리 8GB 중 약 4.5GB, 절반 이상을 쓸 수 있어 처리 중 느려지거나 실패할 수 있습니다.' };
  const f = await fixture(t, { ready: false, speech: { speechEngine: 'qwenKo', speechChoices: [...choices, qwen] } });
  for (const place of ['setup', 'settings']) {
    assert.equal(f.radio(place, 'qwenKo').disabled, false);
    assert.match(f.get(`${place}-speech-picker`).textContent, /절반 이상/);
    assert.equal(f.get(`${place}-speech-picker-memory-acknowledged`).checked, false);
  }
  assert.equal(f.get('setup-download').disabled, true);
  assert.equal(f.get('settings-speech-picker-prepare').disabled, true);
  f.get('settings-speech-picker-prepare').click();
  f.get('setup-download').click();
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(f.calls.some(call => call.name === 'prepareMeetingModels'), false);
  const box = f.get('setup-speech-picker-memory-acknowledged');
  box.checked = true; box.dispatchEvent(new f.window.Event('change'));
  await wait(() => !f.get('setup-download').disabled);
  assert.equal(f.get('settings-speech-picker-memory-acknowledged').checked, true);
  assert.equal(f.get('settings-speech-picker-prepare').disabled, false);
  f.get('setup-download').click();
  await wait(() => f.calls.some(call => call.name === 'prepareMeetingModels'));
});

test('a ready or sufficiently large Mac shows no memory acknowledgement', async t => {
  const qwen = { id: 'qwenKo', label: 'Qwen3-ASR 한국어', modelBytes: 4_400_000_000, ready: false, supported: true, recommended: true, recommendationReason: '녹음이 끝난 뒤 한국어 정확도가 높은 전사를 저장합니다.' };
  const f = await fixture(t, { ready: false, speech: { speechEngine: 'qwenKo', speechChoices: [...choices, qwen] } });
  assert.equal(f.get('setup-speech-picker-memory-acknowledged').closest('.speech-memory-warning').hidden, true);
  assert.match(f.get('setup-speech-picker').textContent, /Qwen3-ASR 한국어 \(추천\)/);
});


test('a waiting switch shows download progress in Settings and cancelling it keeps the working model',async t=>{
  const qwen={id:'qwenKo',label:'Qwen3-ASR 한국어',modelBytes:4_400_000_000,ready:false,supported:true};
  const f=await fixture(t,{speech:{speechChoices:[...choices,qwen],speechSwitch:{target:'qwenKo',preparing:true},
    id:'download',kind:'setup',status:'running',message:'음성 인식 모델 다운로드 중',bytes:1_100_000_000,totalBytes:4_400_000_000,
    jobs:[{id:'download',kind:'setup',status:'running',speechEngine:'qwenKo'}]}});
  const picker=f.get('settings-speech-picker');
  assert.equal(f.radio('settings','qwenKo').checked,true);
  assert.match(picker.textContent,/다운로드 중, 끝나면 전환/);
  assert.match(picker.textContent,/전환 전까지 사용 중/);
  assert.match(picker.textContent,/25% · 1\.10 GB \/ 4\.40 GB/);
  assert.equal(picker.querySelector('fieldset').disabled,false);
  assert.equal(f.radio('settings','whisperTurbo').disabled,true);
  assert.equal(f.get('settings-speech-picker-cancel').textContent,'전환 취소');
  f.get('settings-speech-picker-cancel').click();
  await wait(()=>f.calls.some(call=>call.name==='cancelSpeechSwitch'));
  assert.equal(f.calls.some(call=>call.name==='cancelProcessing'),false);
});

test('staged low-memory downloads need the acknowledgement and confirm before the native request',async t=>{
  const qwen={id:'qwenKo',label:'Qwen3-ASR 한국어',modelBytes:4_400_000_000,ready:false,supported:true,
    memoryWarning:'이 Mac 메모리 8GB 중 약 4.5GB, 절반 이상을 쓸 수 있어 처리 중 느려지거나 실패할 수 있습니다.'};
  const f=await fixture(t,{speech:{speechChoices:[...choices,qwen]}});
  f.radio('settings','qwenKo').click();
  assert.match(f.get('settings-speech-picker').textContent,/받는 동안에는 지금 모델을 계속 사용합니다/);
  assert.equal(f.get('settings-speech-picker-confirm').disabled,true);
  const box=f.get('settings-speech-picker-memory-acknowledged');
  box.checked=true; box.dispatchEvent(new f.window.Event('change'));
  await wait(()=>!f.get('settings-speech-picker-confirm').disabled);
  f.get('settings-speech-picker-confirm').click();
  await wait(()=>f.calls.some(call=>call.name==='requestSpeechEngine'));
  assert.deepEqual(f.calls.find(call=>call.name==='requestSpeechEngine').args,['qwenKo']);
});

test('existing users get the switch offer once and only the buttons record a decision',async t=>{
  const offer={id:'qwen-ko-final-transcript-2026-11',engine:'qwenKo',title:'Qwen3-ASR 한국어 · 녹음 후 전사',description:'',downloadBytes:4_400_000_000,memoryWarning:null};
  const f=await fixture(t,{speech:{speechOffer:offer}});
  const dialog=f.get('speech-offer-dialog');
  await wait(()=>dialog.open);
  assert.match(dialog.textContent,/약 4\.40 GB/);
  assert.match(dialog.textContent,/기존 회의의 음성 기록은 바뀌지 않습니다/);
  dialog.querySelector('[data-action="accept-speech-offer"]').click();
  await wait(()=>f.calls.some(call=>call.name==='respondSpeechOffer'));
  assert.deepEqual(f.calls.find(call=>call.name==='respondSpeechOffer').args,[true]);
  await wait(()=>!dialog.open);
});
