import { build } from 'esbuild';
import { mkdir, readFile, writeFile, cp } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const desktop = fileURLToPath(new URL('../', import.meta.url));
const repository = path.resolve(desktop, '../..');
const output = path.join(repository, '.work/ai-chat-preview');
await mkdir(output, { recursive: true });
await build({ entryPoints: [path.join(desktop, 'tests/fixtures/ai-chat-fixture.jsx')], bundle: true, outfile: path.join(output, 'preview.js'), format: 'iife', globalName: 'AiFixture', jsx: 'automatic', nodePaths: [path.join(desktop, 'node_modules')], define: { 'process.env.NODE_ENV': '"production"' } });
let html = await readFile(path.join(desktop, 'src/index.html'), 'utf8');
html = html.replace('<script type="module" src="/main.js"></script>', `<script src="./preview.js"></script><script>
const params = new URLSearchParams(location.search);
if (['light', 'dark'].includes(params.get('theme'))) document.addEventListener('DOMContentLoaded', () => { document.documentElement.dataset.mode = params.get('theme'); });
const preview = AiFixture.mountAiChatFixture({ document, scenario: params.get('scenario') || 'ready', delay: 600 });
document.querySelectorAll('[data-ui-icon],[data-detail-icon]').forEach(node => { node.innerHTML = NeroIcons(node.dataset.uiIcon || node.dataset.detailIcon); });
const banner = document.createElement('p'); banner.textContent = 'UI 검증용 합성 예시. 실제 계정이나 회의에 연결되지 않습니다.'; banner.style.cssText = 'position:fixed;bottom:0;left:12px;font-size:11px;color:var(--text-secondary)'; document.body.append(banner);
</script>`);
html = html.replace('<script type="module" src="/audio-player.js"></script>', '');
html = html.replaceAll('href="/', 'href="./').replaceAll('src="/', 'src="./');
await writeFile(path.join(output, 'index.html'), html);
for (const name of ['appearance.css', 'typography.css', 'components.css', 'controls.css', 'appearance.js', 'icons.js', 'components.js', 'fonts.css']) {
  await mkdir(path.join(output, 'shared'), { recursive: true });
  await cp(path.join(repository, 'design-system', name), path.join(output, 'shared', name));
}
for (const name of ['style.css', 'sidebar-views.css', 'agent-connections.css', 'ai-chat-ui.css', 'speakers.css', 'transcript-actions.css', 'document-tools.css']) {
  let content = await readFile(path.join(desktop, 'src', name), 'utf8');
  content = content.replaceAll('../../../design-system/', './shared/');
  await writeFile(path.join(output, name), content);
}
await cp(path.join(repository, 'design-system/navigation.css'), path.join(output, 'shared/navigation.css'));
await cp(path.join(repository, 'design-system/vendor'), path.join(output, 'shared/vendor'), { recursive: true });
await cp(path.join(desktop, 'brand/logo'), path.join(output, 'brand/logo'), { recursive: true });
console.log(output);
