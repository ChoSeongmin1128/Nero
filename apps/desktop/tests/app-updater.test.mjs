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

test('a failed automatic check retries and downloads without a user click or automatic installation', async () => {
  const calls = [], scheduled = [];
  let checks = 0;
  const updater = createAppUpdater({ invoke: async command => {
    calls.push(command);
    if (command === 'app_update_check') {
      if (++checks === 1) throw Error('temporary network failure');
      return { phase: 'available' };
    }
    return command === 'app_update_download' ? { phase: 'ready', downloadReady: true } : { phase: 'idle' };
  }, listen: async () => () => {}, schedule: (fn, delay) => { scheduled.push({ fn, delay }); return scheduled.length; }, unschedule() {} });
  await updater.start();
  await scheduled[0].fn();
  assert.equal(updater.getSnapshot().phase, 'error');
  assert.equal(scheduled[1].delay, 5 * 60 * 1000);
  await scheduled[1].fn();
  assert.deepEqual(calls, ['app_update_status', 'app_update_check', 'app_update_check', 'app_update_download']);
  assert.equal(updater.getSnapshot().downloadReady, true);
  assert.equal(scheduled[2].delay, 6 * 60 * 60 * 1000);
  await scheduled[2].fn();
  assert.equal(calls.length, 4, 'a ready download stays ready for the single restart action');
  updater.dispose();
});

test('automatic checks do not interrupt an active download', async () => {
  const calls = [], scheduled = [];
  const updater = createAppUpdater({ invoke: async command => { calls.push(command); return { phase: 'downloading' }; },
    listen: async () => () => {}, schedule: (fn, delay) => { scheduled.push({ fn, delay }); return scheduled.length; }, unschedule() {} });
  await updater.start();
  await scheduled[0].fn();
  assert.deepEqual(calls, ['app_update_status']);
  assert.equal(updater.getSnapshot().phase, 'downloading');
  assert.equal(scheduled[1].delay, 5 * 60 * 1000);
  updater.dispose();
});

for (const failure of ['check', 'download']) {
  test(`manual ${failure} failure replaces the existing six-hour timer with one five-minute retry`, async () => {
    const calls = [], pending = new Map();
    let mode = 'current', nextId = 0, emit;
    const fireNext = async () => {
      assert.equal(pending.size, 1);
      const [id, task] = [...pending][0];
      pending.delete(id);
      await task.fn();
    };
    const updater = createAppUpdater({ invoke: async command => {
      calls.push(command);
      if (command === 'app_update_check') {
        if (mode === 'fail' && failure === 'check') throw Error('manual check failed');
        return { phase: mode === 'current' ? 'current' : 'available' };
      }
      if (command === 'app_update_download') {
        if (mode === 'fail') throw Error('manual download failed');
        return { phase: 'ready', downloadReady: true };
      }
      return { phase: 'idle' };
    }, listen: async (_, callback) => { emit = callback; return () => {}; },
    schedule: (fn, delay) => { pending.set(++nextId, { fn, delay }); return nextId; },
    unschedule: id => pending.delete(id) });
    await updater.start();
    await fireNext();
    assert.equal([...pending.values()][0].delay, 6 * 60 * 60 * 1000);
    mode = 'fail';
    assert.equal(await updater.check(), false);
    assert.equal(updater.getSnapshot().phase, 'error');
    assert.equal(pending.size, 1);
    assert.equal([...pending.values()][0].delay, 5 * 60 * 1000);
    mode = 'recover';
    await fireNext();
    assert.equal(updater.getSnapshot().downloadReady, true);
    assert.equal(calls.includes('app_update_install'), false);
    assert.equal(pending.size, 1);
    assert.equal([...pending.values()][0].delay, 6 * 60 * 60 * 1000);
    emit({ payload: { phase: 'recoveryRequired', recoveryRequired: true } });
    assert.equal(pending.size, 0, 'recovery removes even the new scheduled timer');
    updater.dispose();
    assert.equal(pending.size, 0);
  });
}

test('install waits for editor saves and serializes duplicate clicks', async () => {
  let release;
  const saved = new Promise(resolve => { release = resolve; });
  const calls = [], locks = [];
  const updater = createAppUpdater({ invoke: async command => { calls.push(command); return { phase: 'waiting' }; }, listen: async () => () => {}, saveEdits: () => saved, setInstalling: value => locks.push(value) });
  const installation = updater.install();
  assert.equal(locks.length, 0);
  assert.equal(await updater.install(), false);
  assert.equal(locks.length, 0);
  assert.deepEqual(calls, []);
  release(true);
  await installation;
  assert.deepEqual(calls, ['app_update_install']);
  assert.equal(locks.includes(true), true);
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

test('disabled updater never schedules, checks, downloads, installs or saves', async () => {
  const calls = [], scheduled = [];
  let saves = 0;
  const updater = createAppUpdater({
    invoke: async command => { calls.push(command); return { enabled: false, phase: 'disabled' }; },
    listen: async () => () => {}, saveEdits: async () => { saves++; },
    schedule: (...args) => { scheduled.push(args); },
  });
  await updater.start();
  assert.equal(await updater.check(), false);
  assert.equal(await updater.download(), false);
  assert.equal(await updater.install(), false);
  assert.deepEqual(calls, ['app_update_status']);
  assert.equal(saves, 0);
  assert.deepEqual(scheduled, []);
  assert.equal(appUpdateAction(updater.getSnapshot()).action, null);
  updater.dispose();
});
