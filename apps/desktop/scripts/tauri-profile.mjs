export function tauriProfile(args, environment = process.env.NERO_BUILD_ENV) {
  const command = args[0];
  if (!['dev', 'build'].includes(command)) return { args, environment };
  if (environment && !['development', 'production'].includes(environment)) throw Error('Invalid NERO_BUILD_ENV.');
  if (args.some(arg => arg === '--config' || arg === '-c' || arg.startsWith('--config=') || (arg.startsWith('-c') && arg.length > 2))) {
    throw Error('Custom Tauri configuration is not supported by this wrapper.');
  }
  const split = args.indexOf('--');
  const options = args.slice(1, split < 0 ? undefined : split);
  const cargoOptions = split < 0 ? [] : args.slice(split + 1);
  if (cargoOptions.some(arg => arg === '--features' || arg === '-f' || arg === '-F' || arg.startsWith('--features=') || arg.startsWith('-f') || arg.startsWith('-F') || arg === '--all-features')) {
    throw Error('Pass features as Tauri options before --.');
  }
  let requestedDevelopment = options.includes('--all-features');
  const hasDevelopment = value => value.split(/[ ,]+/).some(feature => feature === 'development' || feature.endsWith('/development'));
  for (let i = 0; i < options.length; i++) {
    const arg = options[i];
    if (arg === '--features' || arg === '-f') {
      let count = 0;
      while (options[i + 1] && !options[i + 1].startsWith('-')) {
        count++;
        if (hasDevelopment(options[++i])) requestedDevelopment = true;
      }
      if (!count) throw Error('Missing features value.');
    } else if (arg.startsWith('--features=')) {
      if (hasDevelopment(arg.slice('--features='.length))) requestedDevelopment = true;
    } else if (arg.startsWith('-f') && arg.length > 2) {
      if (hasDevelopment(arg.slice(2))) requestedDevelopment = true;
    }
  }
  const development = command === 'dev' || (options.includes('--debug') || options.includes('-d')) || requestedDevelopment;
  const selected = development ? 'development' : 'production';
  if (environment && environment !== selected) throw Error(`NERO_BUILD_ENV conflicts with the ${selected} Tauri profile.`);
  const extra = development ? ['--config', 'src-tauri/tauri.dev.conf.json', ...(requestedDevelopment ? [] : ['--features', 'development'])] : [];
  return { args: split < 0 ? [...args, ...extra] : [...args.slice(0, split), ...extra, ...args.slice(split)], environment: selected };
}
