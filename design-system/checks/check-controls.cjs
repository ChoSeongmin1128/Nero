const path = require('node:path');
const { spawnSync } = require('node:child_process');

const result = spawnSync(process.execPath, ['--test', path.resolve(__dirname, '../../apps/desktop/tests/shared-controls.test.mjs')], { stdio: 'inherit' });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
