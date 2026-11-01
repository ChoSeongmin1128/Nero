import assert from 'node:assert/strict';
import test from 'node:test';
import { mountWindowChrome } from '../src/window-chrome.js';

const settled = () => new Promise(resolve => setImmediate(resolve));
const root = value => ({ dataset: { windowChrome: 'macos-overlay', ...(value === undefined ? {} : { windowFullscreen: value }) } });
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function nativeWindow() {
  const callbacks = {}, removed = [];
  let state = false, queries = 0;
  return {
    callbacks, removed,
    get queries() { return queries; },
    set fullscreen(value) { state = value; },
    async isFullscreen() { queries++; return state; },
    async onResized(callback) { callbacks.resize = callback; return () => removed.push('resize'); },
    async onFocusChanged(callback) { callbacks.focus = callback; return () => removed.push('focus'); },
  };
}

test('other documents do not query or subscribe to native window state', () => {
  const document = { dataset: { windowChrome: 'browser', windowFullscreen: 'false' } };
  const native = new Proxy({}, { get() { assert.fail('Native window must not be accessed'); } });
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  chrome.dispose();
  assert.deepEqual(document.dataset, { windowChrome: 'browser', windowFullscreen: 'false' });
});

test('initial fullscreen and resize or focus transitions use actual native state', async () => {
  const document = root(), native = nativeWindow();
  native.fullscreen = true;
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  native.fullscreen = false;
  native.callbacks.resize({ payload: { width: 1920 } });
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'false');
  native.fullscreen = true;
  native.callbacks.focus({ payload: true });
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  chrome.dispose();
  assert.deepEqual(native.removed.sort(), ['focus', 'resize']);
});

test('older queries cannot overwrite a newer resize result', async () => {
  const document = root(), native = nativeWindow();
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  const requests = [];
  native.isFullscreen = () => { const request = deferred(); requests.push(request); return request.promise; };
  native.callbacks.resize({});
  native.callbacks.resize({});
  requests[1].resolve(true);
  await settled();
  requests[0].resolve(false);
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  chrome.dispose();
});

test('events during registration and the registration gap refresh use current state', async () => {
  const document = root(), registration = deferred(), requests = [];
  let resized;
  const native = {
    isFullscreen() { const request = deferred(); requests.push(request); return request.promise; },
    onResized(callback) { resized = callback; return registration.promise; },
  };
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  resized({});
  requests[1].resolve(true);
  await settled();
  requests[0].resolve(false);
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  registration.resolve(() => {});
  await settled();
  assert.equal(requests.length, 3);
  requests[2].resolve(false);
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'false');
  chrome.dispose();
});

test('dispose ignores pending queries and late callbacks and unsubscribes once', async () => {
  const document = root(), native = nativeWindow();
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  const request = deferred();
  let queries = 0;
  native.isFullscreen = () => { queries++; return request.promise; };
  native.callbacks.resize({});
  chrome.dispose();
  chrome.dispose();
  request.resolve(true);
  native.callbacks.focus({});
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'false');
  assert.equal(queries, 1);
  assert.deepEqual(native.removed.sort(), ['focus', 'resize']);
});

test('subscriptions that finish after disposal are immediately removed', async () => {
  const document = root(), registration = deferred(), removed = [];
  const request = deferred();
  const native = { isFullscreen: () => request.promise, onResized: () => registration.promise };
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  chrome.dispose();
  registration.resolve(() => removed.push('resize'));
  request.resolve(true);
  await settled();
  assert.deepEqual(removed, ['resize']);
  assert.equal(document.dataset.windowFullscreen, undefined);
});

test('failed queries retain the last layout state and only log diagnostics', async t => {
  const warnings = t.mock.method(console, 'warn', () => {});
  const document = root('true'), native = nativeWindow();
  native.isFullscreen = async () => { throw Error('Native query unavailable'); };
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  assert.ok(warnings.mock.callCount() > 0);
  native.isFullscreen = async () => undefined;
  native.callbacks.resize({});
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  chrome.dispose();
});

test('subscription failure preserves native querying without a polling fallback', async t => {
  const warnings = t.mock.method(console, 'warn', () => {});
  const document = root(), native = nativeWindow();
  native.onResized = async () => { throw Error('Resize listener unavailable'); };
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'false');
  native.fullscreen = true;
  native.callbacks.focus({});
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'true');
  assert.ok(warnings.mock.callCount() > 0);
  chrome.dispose();
});

test('asynchronous unlisten failures are handled during disposal', async () => {
  const document = root(), native = nativeWindow();
  native.onResized = async () => async () => { throw Error('Window already closed'); };
  native.onFocusChanged = async () => async () => { throw Error('Webview already closed'); };
  const chrome = mountWindowChrome({ root: document, nativeWindow: native });
  await settled();
  chrome.dispose();
  await settled();
  assert.equal(document.dataset.windowFullscreen, 'false');
});
