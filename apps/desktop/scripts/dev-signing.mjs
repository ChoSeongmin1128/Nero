import { execFileSync } from 'node:child_process';
import { lstatSync, realpathSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const bundleId = 'io.github.choseongmin1128.nero.desktop.dev';
const valueOf = (args, name, short) => {
  const joined = args.find(arg => arg.startsWith(`${name}=`));
  if (joined) return joined.slice(name.length + 1);
  const index = args.findIndex(arg => arg === name || arg === short);
  return index < 0 ? null : args[index + 1];
};

// Only local .app development builds use this path. Distribution signing owns
// production bundles, notarization and worker receipts separately.
export function devSigningPlan(profile, args, { platform = process.platform, env = process.env, run = execFileSync, warn = console.warn } = {}) {
  if (platform !== 'darwin' || profile.environment !== 'development' || args[0] !== 'build') return null;
  if (args.includes('--no-sign')) return null;
  if (valueOf(args, '--bundles', '-b') !== 'app' || args.includes('--no-bundle')) {
    warn('For stable development Keychain access, build Nero Dev with --bundles app.');
    return null;
  }
  const configured = env.NERO_DEV_SIGN_IDENTITY || env.APPLE_SIGNING_IDENTITY;
  if (configured === '-') { warn('Nero Dev uses an ad-hoc identity; Keychain approval may repeat after a rebuild.'); return null; }
  const output = run('/usr/bin/security', ['find-identity', '-v', '-p', 'codesigning'], { encoding: 'utf8' });
  const identities = [...output.matchAll(/^\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"([^"]+)"\s*$/gm)].map(([, hash, name]) => ({ hash, name }));
  const matches = configured ? identities.filter(item => item.hash.toLowerCase() === configured.toLowerCase() || item.name === configured) : identities.filter(item => item.name.startsWith('Apple Development:'));
  if (matches.length !== 1) {
    if (configured || matches.length > 1) throw Error('Set NERO_DEV_SIGN_IDENTITY to one valid local certificate fingerprint.');
    warn('No Apple Development identity is available. Nero Dev remains ad-hoc; use NERO_DEV_SIGN_IDENTITY for persistent Keychain access.');
    return null;
  }
  const target = valueOf(args, '--target', '-t');
  const profileName = valueOf(args, '--profile') || (args.includes('--debug') || args.includes('-d') ? 'debug' : 'release');
  if ((target && !/^[\w-]+-apple-darwin$/.test(target)) || !/^[\w-]+$/.test(profileName)) throw Error('Unsupported development signing output.');
  const targetRoot = env.CARGO_TARGET_DIR ? path.resolve(root, env.CARGO_TARGET_DIR) : path.join(root, '.work/rust-target');
  return { app: path.join(targetRoot, ...(target ? [target] : []), profileName, 'bundle/macos/Nero Dev.app'), identity: matches[0].hash };
}

export function signDevBundle(plan, { run = execFileSync, stat = lstatSync, realpath = realpathSync } = {}) {
  if (!plan) return;
  const app = realpath(plan.app);
  if (app !== path.resolve(plan.app) || stat(app).isSymbolicLink()) throw Error('Development signing requires an owned app directory.');
  const plist = path.join(app, 'Contents/Info.plist');
  const id = run('/usr/bin/plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', plist], { encoding: 'utf8' }).trim();
  const executable = run('/usr/bin/plutil', ['-extract', 'CFBundleExecutable', 'raw', '-o', '-', plist], { encoding: 'utf8' }).trim();
  if (id !== bundleId || executable !== 'nero-desktop') throw Error('Refusing to sign a non-development Nero app.');
  const binary = stat(path.join(app, 'Contents/MacOS', executable));
  if (!binary.isFile() || binary.isSymbolicLink() || binary.nlink !== 1) throw Error('Development signing must not modify a shared executable.');
  // Seal only the app/main executable. Packaged workers and their hash receipts
  // remain byte-for-byte unchanged. No broad Keychain ACL or trust changes.
  run('/usr/bin/codesign', ['--force', '--sign', plan.identity, '--identifier', bundleId, '--timestamp=none', app], { stdio: 'inherit' });
  run('/usr/bin/codesign', ['--verify', '--strict', app], { stdio: 'inherit' });
  console.log('Nero Dev signed with a stable local certificate identity.');
}
