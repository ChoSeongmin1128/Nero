import assert from 'node:assert/strict';
import test from 'node:test';
import { createNotificationFlow, notificationTransition as apply, notificationProjection as project, restoreNotificationFlow as restore } from '../../../design-system/notification-flow.mjs';
const meetings=[{id:1,title:'회의 A'},{id:2,title:'회의 B'}];
const event=(type,meetingId=1,attempt=1)=>({type,meetingId,attempt});
test('duplicate starts and terminal deliveries cannot replace a newer attempt',()=>{
 let state=apply(createNotificationFlow(),event('start'));
 assert.equal(apply(state,event('start')),state);
 state=apply(state,event('failed'));state=apply(state,event('retry'));
 assert.equal(state.jobs[0].attempt,2);
 assert.equal(apply(state,event('completed')),state);
 state=apply(state,event('completed',1,2));
 assert.equal(apply(state,event('completed',1,2)),state);
 assert.equal(apply(state,event('failed',1,2)),state);
 assert.equal(apply(state,event('retry',1,2)),state);
 assert.equal(project(state,meetings).length,1);
});
test('acknowledged terminal cleanup does not cancel another meeting or lose retry identity',()=>{
 let state=apply(createNotificationFlow(),event('start'));
 state=apply(state,event('start',2));state=apply(state,event('cancelled'));
 assert.equal(apply(state,event('dismiss')),state);
 state=apply(state,event('acknowledge'));state=apply(state,event('clearAcknowledged'));
 assert.deepEqual(project(state,meetings).map(job=>job.meetingId),[2]);
 state=apply(state,event('retry'));
 assert.equal(state.jobs.find(job=>job.meetingId===1).attempt,2);
 assert.equal(project(state,meetings).length,2);
 assert.equal(project(state,[meetings[0],{...meetings[1],deletedAt:1}]).length,1);
});
test('reload interrupts unfinished work, preserves terminal acknowledgements and rejects corrupt state',()=>{
 let state=apply(createNotificationFlow(),event('start'));
 state=apply(state,event('start',2));state=apply(state,event('completed',2));state=apply(state,event('acknowledge',2));
 const restored=restore(JSON.parse(JSON.stringify(state)));
 assert.equal(restored.jobs[0].status,'interrupted');
 assert.equal(restored.jobs[1].status,'completed');
 assert.equal(restored.jobs[1].acknowledged,true);
 assert.equal(restore({version:2,jobs:[]}).jobs.length,0);
 assert.equal(restore({...state,jobs:[null,{},...state.jobs]}).jobs.length,2);
 assert.equal(apply(restored,event('completed')),restored);
});
