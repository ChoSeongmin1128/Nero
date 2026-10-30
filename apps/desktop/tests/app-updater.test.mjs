import test from 'node:test';
import assert from 'node:assert/strict';
import { createAppUpdater, appUpdateAction } from '../src/app-updater.js';

test('automatic checks download an update but never install it', async () => {
  const calls = [], scheduled = [];
  const updater = createAppUpdater({ invoke: async command => { calls.push(command); return { phase: command === 'app_update_check' ? 'available' : command === 'app_update_download' ? 'ready' : 'idle' }; }, listen: async () => () => {}, schedule: (fn, delay) => { scheduled.push({ fn, delay }); return scheduled.length; } });
  await updater.start();
  assert.equal(scheduled[0].delay, 45_000);
  await scheduled[0].fn();
  assert.deepEqual(calls, ['app_update_status', 'app_update_check', 'app_update_download']);
  assert.equal(updater.getSnapshot().phase, 'ready');
  assert.equal(scheduled[1].delay, 6 * 60 * 60 * 1000);
  updater.dispose();
});

test('install waits for editor saves and serializes duplicate clicks', async () => {
  let release;
  const saved = new Promise(resolve => { release = resolve; });
  const calls = [], locks = [];
  const updater = createAppUpdater({ invoke: async command => { calls.push(command); return { phase: 'waiting' }; }, listen: async () => () => {}, saveEdits: () => saved, setInstalling: value => locks.push(value) });
  const installation = updater.install();
  assert.equal(locks[0], true);
  assert.equal(await updater.install(), false);
  assert.equal(locks.at(-1), true);
  assert.deepEqual(calls, []);
  release(true);
  await installation;
  assert.deepEqual(calls, ['app_update_install']);
  assert.equal(locks.at(-1), false);
});

test('failed or declined saves never invoke the native installer and unlock editing', async () => {
  for (const saveEdits of [async () => false, async () => { throw Error('저장 실패'); }, undefined]) {
    const calls = [], locks = [];
    const updater = createAppUpdater({ invoke: async command => calls.push(command), listen: async () => () => {}, saveEdits, setInstalling: value => locks.push(value) });
    assert.equal(await updater.install(), false);
    assert.deepEqual(calls, []);
    assert.equal(locks.at(-1), false);
  }
});

test('dispose removes listeners and prevents delayed actions', async () => {
  let release, stopped = 0;
  const calls = [];
  const updater = createAppUpdater({ invoke: async command => calls.push(command), listen: () => new Promise(resolve => { release = resolve; }) });
  const started = updater.start();
  updater.dispose();
  release(() => stopped++);
  await started;
  assert.equal(stopped, 1);
  assert.equal(await updater.check(), false);
  assert.deepEqual(calls, []);
});

test('status actions expose user-controlled install and progress', () => {
  assert.equal(appUpdateAction({ phase: 'ready' }).action, 'install');
  assert.equal(appUpdateAction({ phase: 'waiting' }).action, 'install');
  assert.equal(appUpdateAction({ phase: 'downloading' }).disabled, true);
  assert.equal(appUpdateAction({ phase: 'error' }).action, 'check');
});

test('failed app restoration keeps editing locked and permits only recovery quit', async () => {
  const calls = [], locks = [];
  const recovery = { phase: 'recoveryRequired', recoveryRequired: true, previousAppPath: '/synthetic/이전 앱.app', recoveryReceiptSaved: true };
  const updater = createAppUpdater({ invoke: async command => { calls.push(command); return command === 'app_update_install' ? recovery : undefined; }, listen: async () => () => {}, saveEdits: async () => true, setInstalling: value => locks.push(value) });
  await updater.install();
  assert.equal(locks.at(-1), true);
  assert.equal(updater.getSnapshot().previousAppPath, recovery.previousAppPath);
  assert.equal(appUpdateAction(updater.getSnapshot()).action, 'quitForRecovery');
  assert.equal(await updater.check(), false);
  assert.equal(await updater.download(), false);
  assert.equal(await updater.install(), false);
  assert.deepEqual(calls, ['app_update_install']);
  await updater.quitForRecovery();
  assert.deepEqual(calls, ['app_update_install', 'app_update_recovery_quit']);
  assert.equal(locks.at(-1), true);
});

test('recovery exit does not wait for a pending installation IPC response', async () => {
  let eventHandler, finishInstall;
  const calls = [], locks = [];
  const recovery = { phase: 'recoveryRequired', recoveryRequired: true, previousAppPath: '/synthetic/previous.app' };
  const updater = createAppUpdater({ invoke: async command => {
    calls.push(command);
    if (command === 'app_update_status') return { phase: 'ready' };
    if (command === 'app_update_install') return new Promise(resolve => { finishInstall = resolve; });
  }, listen: async (_event, handler) => { eventHandler = handler; return () => {}; }, saveEdits: async () => true, setInstalling: value => locks.push(value), schedule: () => 0, unschedule: () => {} });
  await updater.start();
  const installing = updater.install();
  await new Promise(resolve => setImmediate(resolve));
  eventHandler({ payload: recovery });
  eventHandler({ payload: { phase: 'ready', recoveryRequired: false } });
  assert.equal(updater.getSnapshot().previousAppPath, recovery.previousAppPath);
  assert.equal(updater.getSnapshot().recoveryRequired, true);
  assert.equal(await updater.quitForRecovery(), true);
  assert.equal(calls.at(-1), 'app_update_recovery_quit');
  assert.equal(locks.at(-1), true);
  finishInstall(recovery);
  await installing;
  updater.dispose();
});
