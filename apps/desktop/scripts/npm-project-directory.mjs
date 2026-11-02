import { readFile, realpath } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

const dependencyFields = ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies', 'peerDependenciesMeta'];
const ordered = value => Array.isArray(value) ? value.map(ordered) : value && typeof value === 'object'
  ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, ordered(item)])) : value;
const normalize = value => JSON.stringify(ordered(value || {}));

export async function npmProjectDirectory(app) {
  const modules = await realpath(resolve(app, 'node_modules'));
  const directory = dirname(modules);
  if (await realpath(resolve(directory, 'node_modules')) !== modules) throw new Error('Shared node_modules has no matching npm project directory.');
  if (directory === await realpath(app)) return directory;
  const [requested, installed] = await Promise.all([app, directory].map(async root => JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'))));
  if (dependencyFields.some(field => normalize(requested[field]) !== normalize(installed[field]))) {
    throw new Error('Shared node_modules does not match the requested runtime dependencies.');
  }
  const locks = await Promise.all([app, directory].map(async root => {
    const lock = JSON.parse(await readFile(resolve(root, 'package-lock.json'), 'utf8'));
    if (!lock.packages) throw new Error('Missing installed dependency identities in package-lock.json.');
    return Object.fromEntries(Object.entries(lock.packages).filter(([path]) => path !== ''));
  }));
  if (normalize(locks[0]) !== normalize(locks[1])) throw new Error('Shared node_modules does not match package-lock.json.');
  return directory;
}
