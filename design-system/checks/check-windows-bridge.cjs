const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const M = require('../prototype/model.js');
const Desktop = require('../prototype/desktop-preview-model.js');
const messages = [], listeners = new Map();
const host = { postMessage: (value, origin) => messages.push({ value, origin }) };
const add = (name, handler) => listeners.set(name, handler);
const context = {
  URLSearchParams, Date, Object, console,
  location: { search: '?platform=windows&session=windows-design', origin: 'https://nero.test' },
  parent: host, NeroDesktopModel: Desktop,
  document: { body: { classList: { add() {} } }, documentElement: { dataset: {} }, querySelectorAll: () => [], addEventListener: add },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
  addEventListener: add, setInterval: () => 1, clearInterval() {},
};
context.window = context;
vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../windows-preview/bridge.js'), 'utf8'), context);
let store;
const controller = context.NeroDesktopController;
controller.attach({ model: M, begin: (value) => { store = value; }, getState: () => store,
  setState: (value) => { store = value; }, clearModal() {}, render: () => controller.sync() });
const snapshot = () => messages.filter(({ value }) => value.type === 'windows-nero:state').at(-1).value;
const command = (action, extra = {}, source = host, origin = context.location.origin) => listeners.get('message')({ source, origin, data: { type: 'windows-nero:command', action, ...extra } });
assert.equal(snapshot().phase, 'ready');
command('start', {}, {}); assert.equal(snapshot().phase, 'ready');
command('start', {}, host, 'https://other.test'); assert.equal(snapshot().phase, 'ready');
command('start'); assert.equal(snapshot().phase, 'recording');
const owner = snapshot().ownerId;
assert.ok(owner);
command('pause'); assert.equal(snapshot().phase, 'paused');
command('resume'); assert.equal(snapshot().phase, 'recording');
command('stop', { ownerId: 'stale-owner' }); assert.equal(snapshot().phase, 'recording');
controller.action('stop');
assert.equal(messages.at(-1).value.type, 'windows-nero:request-stop');
assert.equal(messages.at(-1).value.ownerId, owner);
controller.sync(); assert.equal(snapshot().phase, 'recording');
command('stop', { ownerId: owner }); assert.equal(snapshot().phase, 'finished');
const previous = snapshot().phase;
command('scenario', { value: '__proto__' }); assert.equal(snapshot().phase, previous);
assert.ok(messages.every(({ origin }) => origin === 'https://nero.test'));
console.log('Windows bridge: origin/source guards, shared recording state, request/confirm and owner checks passed.');
