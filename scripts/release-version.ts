import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const packagePath = 'package.json';
const configPath = 'src-tauri/tauri.conf.json';
const cargoPath = 'src-tauri/Cargo.toml';
const pkg = JSON.parse(readFileSync(packagePath, 'utf8'));
const config = JSON.parse(readFileSync(configPath, 'utf8'));
const cargo = readFileSync(cargoPath, 'utf8');
const cargoVersion = cargo.match(/^version = "([^"]+)"/m)?.[1];
const requested = process.argv[2];
const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

if (requested && requested !== '--check') {
  if (!stableVersion.test(requested))
    throw new Error('Use a stable version such as 0.1.4.');
  pkg.version = config.version = requested;
  writeFileSync(packagePath, JSON.stringify(pkg, null, 2) + '\n');
  writeFileSync(configPath, JSON.stringify(config, null, 2) + '\n');
  writeFileSync(
    cargoPath,
    cargo.replace(/^version = "[^"]+"/m, `version = "${requested}"`),
  );
  execFileSync(
    'cargo',
    ['update', '--workspace', '--manifest-path', cargoPath],
    { stdio: 'ignore' },
  );
  console.log(
    `Version set to ${requested}. Commit the changes, then push tag v${requested}.`,
  );
} else {
  if (
    !stableVersion.test(pkg.version) ||
    pkg.version !== config.version ||
    pkg.version !== cargoVersion
  ) {
    throw new Error(
      'Versions in package.json, Cargo.toml, and tauri.conf.json must match.',
    );
  }
  const tag = `v${pkg.version}`;
  if (
    process.env.GITHUB_REF_TYPE === 'tag' &&
    process.env.GITHUB_REF_NAME !== tag
  ) {
    throw new Error(`Release tag must be ${tag}.`);
  }
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(
      process.env.GITHUB_OUTPUT,
      `version=${pkg.version}\ntag=${tag}\n`,
    );
  console.log(`Version ${pkg.version}`);
}
