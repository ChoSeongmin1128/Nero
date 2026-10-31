import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountAppUpdateView, appUpdateNotices } from '../src/app-update-view.js';
import { mountNotificationCenter } from '../src/notification-center.js';

const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const status = (phase, extra = {}) => ({ phase, currentVersion: '0.0.1', currentBuild: '40', availableVersion: '0.0.1', availableBuild: '41', message: phase, ...extra });

function setup(t, overrides = {}) {
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  window.document.write(html);
  window.document.getElementById('sidebar-app-update')?.remove();
  let sidebar;
  if (overrides.sidebar) {
    sidebar = window.document.createElement('button');
    sidebar.id = 'sidebar-app-update';
    sidebar.innerHTML = '<span data-detail-icon="download"></span><span data-app-update-label></span><progress data-app-update-progress></progress>';
    window.document.querySelector('.workspace').append(sidebar);
  }
  t.after(() => window.happyDOM.abort());
  const calls = [], notices = [], locks = [], scheduled = [];
  let emit;
  const bridge = {
    appUpdateStatus: async () => status('idle'),
    checkAppUpdate: async () => { calls.push('check'); return status('available'); },
    downloadAppUpdate: async () => { calls.push('download'); return status('ready', { downloadReady: true }); },
    installAppUpdate: async () => { calls.push('install'); return status('waiting'); },
    subscribeAppUpdate: async callback => { emit = callback; return () => {}; }, ...overrides.bridge,
  };
  const view = mountAppUpdateView({ container: window.document.getElementById('app-update'), bridge,
    saveEdits: overrides.saveEdits || (async () => true), onRecovery: overrides.onRecovery, onInstalling: value => locks.push(value), onChange: value => notices.push(value),
    schedule: (fn, delay) => { scheduled.push({ fn, delay }); return scheduled.length; }, unschedule() {},
  });
  t.after(() => view.destroy());
  return { window, view, sidebar, calls, notices, locks, scheduled, emit: value => emit(value), get: id => window.document.getElementById(`app-update-${id}`) };
}

test('settings row starts status/listening and auto-downloads without installing', async t => {
  const app = setup(t);
  await app.view.start();
  assert.equal(app.get('version').textContent, '0.0.1 (40)');
  assert.equal(app.scheduled[0].delay, 45_000);
  await app.scheduled[0].fn();
  assert.deepEqual(app.calls, ['check', 'download']);
  assert.equal(app.get('action').textContent, '다시 시작해 업데이트');
  assert.deepEqual(app.notices.at(-1), []);
  assert.equal(app.scheduled[1].delay, 6 * 60 * 60 * 1000);
  app.emit(status('downloading', { totalBytes: 100, downloadedBytes: 40 }));
  assert.equal(app.get('progress').value, 40);
  assert.equal(app.get('action').disabled, true);
  assert.deepEqual(app.notices.at(-1), []);
});

test('settings keep routine update states quiet while retaining results and failure guidance', async t => {
  const app = setup(t);
  await app.view.start();
  assert.equal(app.get('message').hidden, true);
  assert.equal(app.get('recovery').hidden, true);
  for (const phase of ['checking', 'downloading', 'ready', 'installing']) {
    app.emit(status(phase, { message: '버튼과 중복되는 안내' }));
    assert.equal(app.get('message').hidden, true);
  }
  app.emit(status('current'));
  assert.equal(app.get('message').hidden, true);
  assert.equal(app.get('message').textContent, '');
  for (const phase of ['waiting', 'error']) {
    app.emit(status(phase, { message: '녹음이 끝난 뒤 다시 시도해 주세요.' }));
    assert.equal(app.get('message').hidden, false);
    assert.equal(app.get('message').textContent, '녹음이 끝난 뒤 다시 시도해 주세요.');
  }
  app.emit(status('error', { message: '앱 복원에 실패했습니다.', recoveryMessage: '앱 복원에 실패했습니다.' }));
  assert.equal(app.get('message').hidden, true);
  assert.equal(app.get('recovery').hidden, false);
  app.emit(status('idle'));
  assert.equal(app.get('message').hidden, true);
  assert.equal(app.get('message').textContent, '');
  assert.equal(app.get('recovery').hidden, true);
});

test('install keeps dirty confirmation interactive, then owns and releases the installation lock', async t => {
  let decide, finish;
  const app = setup(t, { saveEdits: () => new Promise(resolve => { decide = resolve; }), bridge: {
    installAppUpdate: () => new Promise(resolve => { finish = resolve; }),
  } });
  await app.view.start(); app.emit(status('ready', { downloadReady: true }));
  const workspace = app.window.document.querySelector('.workspace');
  const dialog = app.window.document.getElementById('settings-dialog'); dialog.showModal();
  app.get('action').click(); await tick();
  assert.equal(workspace.inert, false); assert.equal(dialog.inert, false);
  assert.equal(app.get('action').disabled, true);
  decide(true); await tick();
  assert.equal(workspace.inert, true); assert.equal(dialog.inert, true);
  assert.deepEqual(app.locks, [true]);
  finish(status('waiting', { message: '녹음이 끝난 뒤 다시 시도해 주세요.' })); await tick();
  assert.equal(workspace.inert, false); assert.equal(dialog.inert, false);
  assert.deepEqual(app.locks, [true, false]);
  assert.equal(app.get('action').disabled, false);
});

test('declined changes never install or acquire the lock', async t => {
  const app = setup(t, { saveEdits: async () => false });
  await app.view.start(); app.emit(status('ready'));
  await app.view.perform();
  assert.deepEqual(app.calls, []); assert.deepEqual(app.locks, []);
  assert.equal(app.get('action').disabled, false);
});

test('update failures expose a user action without routine completion notices', async t => {
  const app = setup(t);
  await app.view.start(); app.emit(status('error', { downloadReady: true, message: '설치를 다시 시도해 주세요.' }));
  const center = mountNotificationCenter({ container: app.window.document.getElementById('notification-center'), onNoticeAction: service => { assert.equal(service, 'app-update'); return app.view.perform(); } });
  t.after(() => center.destroy());
  center.update({ connected: true, notices: appUpdateNotices(app.view.getSnapshot()) });
  const container = app.window.document.getElementById('notification-center');
  assert.doesNotMatch(container.textContent, /연동 설정/);
  container.querySelector('[data-notification-action="notice"]').click(); await tick();
  assert.deepEqual(app.calls, ['install']);
  assert.equal(appUpdateNotices(status('ready')).length, 0);
  assert.equal(appUpdateNotices(status('current')).length, 0);
  assert.equal(appUpdateNotices(status('error', { message: '' })).length, 0);
  assert.equal(appUpdateNotices(status('idle', { recoveryMessage: '업데이트 복구 확인' }))[0].service, 'app-update');
});

test('isolated recovery status retains the app lock while excluding the recovery host', async t => {
  const recoveryStates = [];
  const app = setup(t, { onRecovery: value => recoveryStates.push(value), bridge: { installAppUpdate: async () => status('recoveryRequired', { recoveryRequired: true, previousAppPath: '/recovery/Previous.app', recoveryReceiptSaved: true }) } });
  const host = app.window.document.createElement('dialog'); host.dataset.appUpdateRecovery = ''; app.window.document.body.append(host);
  const nested = app.window.document.createElement('div'); nested.setAttribute('role', 'alertdialog'); host.append(nested);
  await app.view.start(); app.emit(status('ready'));
  await app.view.perform();
  assert.equal(app.window.document.querySelector('.workspace').inert, true);
  assert.equal(app.window.document.getElementById('settings-dialog').inert, true);
  assert.equal(host.inert, false); assert.equal(nested.inert, false);
  assert.match(app.get('action').textContent, /종료/);
  assert.equal(recoveryStates.at(-1).previousAppPath, '/recovery/Previous.app');
});


test('sidebar exposes download progress and requires a click after download completes', async t => {
  const app = setup(t, { sidebar: true });
  await app.view.start();
  assert.equal(app.sidebar.hidden, true);
  app.get('action').click(); await tick();
  assert.deepEqual(app.calls, ['check', 'download']);
  assert.equal(app.sidebar.hidden, false);
  assert.equal(app.sidebar.getAttribute('aria-label'), '업데이트');
  assert.equal(app.sidebar.title, '업데이트');
  assert.equal(app.sidebar.querySelector('[data-app-update-label]').textContent, '업데이트');
  assert.equal(app.get('action').textContent, '업데이트 확인');
  assert.equal(app.get('action').disabled, true);
  app.emit(status('downloading', { totalBytes: 100, downloadedBytes: 40 }));
  assert.equal(app.sidebar.disabled, true);
  assert.equal(app.sidebar.title, '업데이트 다운로드 40%');
  assert.equal(app.sidebar.querySelector('progress').value, 40);
  app.sidebar.click(); await tick();
  assert.deepEqual(app.calls, ['check', 'download']);
  app.emit(status('downloading'));
  assert.equal(app.sidebar.querySelector('progress').hasAttribute('value'), false);
  assert.equal(app.sidebar.title, '업데이트 다운로드');
  app.emit(status('ready', { downloadReady: true }));
  assert.equal(app.sidebar.querySelector('progress').hidden, true);
  assert.equal(app.sidebar.disabled, false);
  assert.deepEqual(app.calls, ['check', 'download']);
  app.sidebar.click(); await tick();
  assert.deepEqual(app.calls, ['check', 'download', 'install']);
});

test('sidebar waits for saves and native waiting keeps the update available', async t => {
  let resolveSave;
  const app = setup(t, { sidebar: true, saveEdits: () => new Promise(resolve => { resolveSave = resolve; }) });
  await app.view.start();
  app.emit(status('ready', { downloadReady: true }));
  app.sidebar.click(); await tick();
  assert.deepEqual(app.calls, []);
  assert.equal(app.sidebar.disabled, true);
  assert.equal(app.window.document.querySelector('.workspace').inert, false);
  resolveSave(false); await tick();
  assert.deepEqual(app.calls, []);
  assert.equal(app.sidebar.disabled, false);
  app.sidebar.click(); await tick();
  resolveSave(true); await tick();
  assert.deepEqual(app.calls, ['install']);
  assert.equal(app.sidebar.disabled, false);
  assert.equal(app.sidebar.hidden, false);
  assert.equal(app.window.document.querySelector('.workspace').inert, false);
});

test('sidebar failure remains actionable and destroyed view removes its listener', async t => {
  const app = setup(t, { sidebar: true });
  await app.view.start();
  app.emit(status('error', { message: '네트워크를 확인해 주세요.' }));
  assert.equal(app.sidebar.hidden, false);
  assert.equal(app.sidebar.disabled, false);
  assert.equal(app.get('action').disabled, false);
  assert.equal(app.notices.at(-1)[0].message, '네트워크를 확인해 주세요.');
  app.view.destroy();
  app.sidebar.click(); await tick();
  assert.deepEqual(app.calls, []);
});
