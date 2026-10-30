import assert from 'node:assert/strict';
import test from 'node:test';
import { requestAudioPlayback } from '../src/audio-playback.js';
const pending=()=>{let reject;return{promise:new Promise((_,no)=>reject=no),reject:e=>reject(e)}};
test('old source and old meeting playback rejections never report into the new screen',async()=>{
 for(const replacement of ['source','meeting']){
  const wait=pending(),errors=[];let source='a.wav',current=true;
  const audio={getAttribute:()=>source,play:()=>wait.promise};
  const request=requestAudioPlayback(audio,{isCurrent:()=>current,onError:e=>errors.push(e)});
  if(replacement==='source')source='b.wav';else current=false;
  wait.reject(Error('old'));assert.equal(await request,false);assert.equal(errors.length,0);
 }
});
test('intentional playback abort stays silent but a current decoder failure remains actionable',async()=>{
 const errors=[],audio={getAttribute:()=> 'a.wav',play:async()=>{throw Object.assign(Error('pause'),{name:'AbortError'})}};
 await requestAudioPlayback(audio,{onError:e=>errors.push(e)});assert.equal(errors.length,0);
 audio.play=async()=>{throw Error('decode')};
 await requestAudioPlayback(audio,{onError:e=>errors.push(e)});assert.equal(errors.length,1);
 audio.play=async()=>{};assert.equal(await requestAudioPlayback(audio),true);
});
