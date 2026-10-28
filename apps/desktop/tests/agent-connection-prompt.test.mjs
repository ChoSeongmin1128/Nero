import assert from 'node:assert/strict';
import test from 'node:test';
import { agentConnectionPrompt } from '../src/agent-connection-prompt.js';

function configuration(prompt) {
  return JSON.parse(prompt.slice(prompt.indexOf('{'), prompt.lastIndexOf('}') + 1));
}

for (const [platform, executablePath, dataDir] of [
  ['macOS', "/Applications/User's Nero.app/Contents/Resources/agent/nero", "/Users/Example's Space/Library/Application Support/nero"],
  ['Windows', "C:\\Users\\Example's Space\\Nero\\agent\\nero.exe", "C:\\Users\\Example's Space\\AppData\\Roaming\\nero"],
]) {
  test(`${platform} prompt carries literal paths once without shell interpolation`, () => {
    const prompt = agentConnectionPrompt({ available: true, executablePath, dataDir, token: 'do-not-copy', config: { env: { SECRET: 'do-not-copy' } }, clients: [{ detail: 'do-not-copy' }] });
    assert.deepEqual(configuration(prompt), { mcpServers: { nero: { command: executablePath, args: ['--data-dir', dataDir, 'mcp'] } } });
    assert.equal(prompt.split(JSON.stringify(executablePath)).length - 1, 1);
    assert.equal(prompt.split(JSON.stringify(dataDir)).length - 1, 1);
    assert.doesNotMatch(prompt, /do-not-copy|curl|sudo|powershell -enc/);
    assert.match(prompt, /doctor --json/);
    assert.match(prompt, /기존 설정을 보존/);
    assert.match(prompt, /읽기 전용/);
  });
}

test('missing information produces a discovery request without inventing a data path', () => {
  for (const info of [null, {}, { available: false, executablePath: '/not-installed/nero', dataDir: '/not-current' }, { available: true, executablePath: '/nero' }]) {
    const prompt = agentConnectionPrompt(info);
    assert.match(prompt, /설치된 Nero 앱/);
    assert.match(prompt, /파일이 없으면 임의로 설치하지 말고/);
    assert.doesNotMatch(prompt, /not-installed|not-current|mcpServers/);
  }
});
