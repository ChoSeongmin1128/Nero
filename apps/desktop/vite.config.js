import { defineConfig, normalizePath } from 'vite';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const mainDocument = normalizePath(fileURLToPath(new URL('./src/index.html', import.meta.url)));
export default defineConfig({
  plugins: [{
    name: 'nero-native-window-chrome',
    transformIndexHtml(html, context) {
      if (normalizePath(context.filename) === mainDocument && process.env.TAURI_ENV_TARGET_TRIPLE?.endsWith('-apple-darwin')) {
        return html.replace('<html ', '<html data-window-chrome="macos-overlay" ');
      }
      return html;
    },
  }],
  resolve: {
    alias: Object.fromEntries(['react-dom', 'react', '@base-ui/react', 'react-day-picker', 'lucide-react', 'clsx', 'tailwind-merge'].map(name => [name, `${here}node_modules/${name}`])),
    dedupe: ['react', 'react-dom'],
  },
  root: `${here}src`,
  publicDir: `${here}.generated/public`,
  clearScreen: false,
  build: {
    outDir: `${here}dist`,
    emptyOutDir: true,
    target: ['es2022', 'chrome110', 'safari15'],
    rollupOptions: {
      input: {
        main: `${here}src/index.html`,
        recorder: `${here}src/recorder.html`,
        calendar: `${here}src/calendar.html`,
      },
    },
  },
});
