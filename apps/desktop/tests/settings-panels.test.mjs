import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { createSettingsPanels } from '../src/settings-panels.js';
import { createNewMeetingFolder } from '../src/new-meeting-folder.js';

function dialogWith(names) {
  const window = new Window();
  const document = window.document;
  document.body.innerHTML = `<dialog id="settings"><nav class="app-settings-tabs">${names.map(name => `<button role="tab" id="settings-${name}-tab" data-settings-panel="${name}"></button>`).join('')}</nav>${names.map(name => `<section id="settings-${name}-panel"></section>`).join('')}</dialog>`;
  return { window, dialog: document.getElementById('settings'), byId: id => document.getElementById(id) };
}

test('only the shown panel is active, its message is returned and closing turns every part off', t => {
  const { window, dialog, byId } = dialogWith(['appearance', 'recording', 'agent']);
  t.after(() => window.happyDOM.abort());
  const calls = [];
  const part = name => ({ setActive: active => calls.push([name, active]) });
  const panels = createSettingsPanels({ dialog, byId, panels: { recording: [part('storage'), { ...part('recording'), message: () => '마이크 없음' }], agent: [part('agent'), part('calendar')] } });
  dialog.showModal();
  assert.equal(panels.render('recording'), '마이크 없음');
  assert.deepEqual(calls.filter(([, active]) => active).map(([name]) => name), ['storage', 'recording']);
  assert.equal(byId('settings-recording-panel').hidden, false);
  assert.equal(byId('settings-agent-panel').hidden, true);
  assert.equal(byId('settings-recording-tab').getAttribute('aria-selected'), 'true');
  assert.equal(byId('settings-agent-tab').tabIndex, -1);
  assert.equal(panels.render('appearance'), '', 'a panel without a message shows none');
  calls.length = 0;
  panels.deactivate();
  assert.deepEqual(calls.map(([, active]) => active), [false, false, false, false]);
  dialog.close();
  calls.length = 0;
  panels.render('recording');
  assert.equal(calls.some(([, active]) => active), false, 'a closed dialog activates nothing');
});

test('arrow keys move between tabs in a loop and other keys do nothing', t => {
  const { window, dialog, byId } = dialogWith(['appearance', 'recording', 'agent']);
  t.after(() => window.happyDOM.abort());
  const panels = createSettingsPanels({ dialog, byId });
  assert.equal(panels.neighbor('settings-agent-tab', 'ArrowDown'), 'appearance');
  assert.equal(panels.neighbor('settings-appearance-tab', 'ArrowUp'), 'agent');
  assert.equal(panels.neighbor('settings-recording-tab', 'Home'), 'appearance');
  assert.equal(panels.neighbor('settings-recording-tab', 'End'), 'agent');
  assert.equal(panels.neighbor('settings-recording-tab', 'Enter'), null);
  assert.equal(panels.neighbor('other', 'ArrowDown'), null);
});

test('a failed new-meeting folder change keeps the previous folder and reports busy while saving', async () => {
  let release, changes = 0;
  const folder = createNewMeetingFolder({ onChange: () => { changes += 1; }, bridge: {
    readNewMeetingFolder: async () => 'project',
    setNewMeetingFolder: () => new Promise((resolve, reject) => { release = { resolve, reject }; }),
  } });
  await folder.reload();
  assert.equal(folder.value(), 'project');
  const saving = folder.set('');
  assert.equal(folder.busy(), true);
  release.reject(new Error('disk'));
  await assert.rejects(saving, /disk/);
  assert.equal(folder.value(), 'project');
  assert.equal(folder.busy(), false);
  const saved = folder.set('other');
  release.resolve('other');
  await saved;
  assert.equal(folder.value(), 'other');
  assert.ok(changes >= 5);
});

test('a folder that cannot be read leaves the current value and redraws nothing', async () => {
  let changes = 0;
  const folder = createNewMeetingFolder({ onChange: () => { changes += 1; }, bridge: { readNewMeetingFolder: async () => { throw new Error('offline'); } } });
  await folder.reload();
  assert.equal(folder.value(), null);
  assert.equal(changes, 0);
});
