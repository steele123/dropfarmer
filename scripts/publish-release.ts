import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';

const root = process.cwd();
const packagePath = join(root, 'package.json');
const configPath = join(root, 'src-tauri', 'tauri.conf.json');
const cargoPath = join(root, 'src-tauri', 'Cargo.toml');
const versionScript = join(root, 'scripts', 'release-version.ts');
const packageData = JSON.parse(readFileSync(packagePath, 'utf8')) as {
  version: string;
};
const configData = JSON.parse(readFileSync(configPath, 'utf8')) as {
  version: string;
};
const requestedVersion = process.argv
  .slice(2)
  .find((arg) => !arg.startsWith('--'));
const notesArg = process.argv
  .slice(2)
  .find((arg) => arg.startsWith('--notes-file='));
const notesIndex = process.argv.indexOf('--notes-file');
const notesFile =
  notesArg?.slice('--notes-file='.length) ??
  (notesIndex >= 0 ? process.argv[notesIndex + 1] : undefined);
const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function fail(message: string): never {
  throw new Error(message);
}

function run(command: string, args: string[], env = process.env): void {
  execFileSync(command, args, { cwd: root, stdio: 'inherit', env });
}

function capture(command: string, args: string[]): string {
  return execFileSync(command, args, { cwd: root, encoding: 'utf8' }).trim();
}

function hasRelease(tag: string): { isDraft: boolean } | null {
  try {
    return JSON.parse(
      capture('gh', ['release', 'view', tag, '--json', 'isDraft']),
    ) as {
      isDraft: boolean;
    };
  } catch {
    return null;
  }
}

function filesUnder(directory: string): string[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? filesUnder(path) : [path];
  });
}

function readNotes(version: string): string {
  if (notesFile) {
    const path = resolve(root, notesFile);
    if (!existsSync(path))
      fail(`Release notes file does not exist: ${notesFile}`);
    return readFileSync(path, 'utf8').trim();
  }
  return [
    `Windows x64 release for Dropfarmer ${version}.`,
    '',
    'Existing installations can update from Settings → App updates.',
  ].join('\n');
}

function main() {
  if (process.argv.includes('--help')) {
    console.log('Usage: bun run release:publish <version> [--notes-file path]');
    console.log(
      'Builds, signs, uploads, and publishes a Windows release from this PC.',
    );
    return;
  }
  const version = requestedVersion ?? fail('Pass a version such as 0.1.6.');
  if (!stableVersion.test(version)) fail('Use a stable version such as 0.1.6.');
  const tag = `v${version}`;
  if (capture('git', ['status', '--porcelain'])) {
    fail('Commit or stash working changes before publishing a release.');
  }
  if (capture('git', ['branch', '--show-current']) !== 'main') {
    fail('Publish releases from the main branch.');
  }
  const existingRelease = hasRelease(tag);
  if (existingRelease && !existingRelease.isDraft) {
    fail(`${tag} is already published. Choose a new version.`);
  }

  if (packageData.version !== version) {
    run('bun', [versionScript, version]);
  } else if (configData.version !== version) {
    fail('package.json and tauri.conf.json do not have matching versions.');
  }
  run('bun', [versionScript, '--check']);
  run('bun', ['run', 'check']);
  run('bun', ['run', 'test']);
  run('cargo', ['test', '--release', '--locked', '--manifest-path', cargoPath]);

  if (capture('git', ['status', '--porcelain'])) {
    run('git', [
      'add',
      'package.json',
      'src-tauri/Cargo.toml',
      'src-tauri/Cargo.lock',
      'src-tauri/tauri.conf.json',
    ]);
    run('git', ['commit', '-m', `Release ${version}`]);
  }

  const signingKey = process.env.TAURI_SIGNING_PRIVATE_KEY;
  if (!signingKey) {
    fail(
      'Set TAURI_SIGNING_PRIVATE_KEY to the signing key path or key contents.',
    );
  }
  const buildEnv = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: signingKey };
  run(
    'bun',
    ['run', 'tauri', 'build', '--bundles', 'nsis', '--', '--locked'],
    buildEnv,
  );

  const files = filesUnder(
    join(root, 'src-tauri', 'target', 'release', 'bundle'),
  );
  const installer = files.find(
    (path) => /-setup\.exe$/i.test(path) && path.includes(`_${version}_`),
  );
  if (!installer) fail('The signed NSIS installer was not produced.');
  const signature = `${installer}.sig`;
  if (!existsSync(signature))
    fail(`Installer signature was not produced: ${relative(root, signature)}`);
  const signatureText = readFileSync(signature, 'utf8').trim();
  if (!signatureText) fail('The installer signature is empty.');

  const repo = capture('gh', [
    'repo',
    'view',
    '--json',
    'nameWithOwner',
    '--jq',
    '.nameWithOwner',
  ]);
  const releaseNotes = readNotes(version);
  const staging = mkdtempSync(join(tmpdir(), 'dropfarmer-release-'));
  const latestPath = join(staging, 'latest.json');
  const installerName = installer.split(/[\\/]/).pop()!;
  const installerUrl = `https://github.com/${repo}/releases/download/${tag}/${installerName}`;
  writeFileSync(
    latestPath,
    `${JSON.stringify(
      {
        version,
        notes: releaseNotes,
        pub_date: new Date().toISOString(),
        platforms: {
          'windows-x86_64': { signature: signatureText, url: installerUrl },
          'windows-x86_64-nsis': {
            signature: signatureText,
            url: installerUrl,
          },
        },
      },
      null,
      2,
    )}\n`,
  );

  try {
    const head = capture('git', ['rev-parse', 'HEAD']);
    const localTag = (() => {
      try {
        return capture('git', ['rev-parse', `${tag}^{commit}`]);
      } catch {
        return '';
      }
    })();
    if (localTag && localTag !== head)
      fail(`${tag} already points to a different commit.`);
    if (!localTag) run('git', ['tag', tag]);
    run('git', ['push', 'origin', 'main']);
    const remoteTag = (() => {
      try {
        return (
          capture('git', ['ls-remote', 'origin', `refs/tags/${tag}`]).split(
            /\s+/,
          )[0] ?? ''
        );
      } catch {
        return '';
      }
    })();
    if (remoteTag && remoteTag !== head)
      fail(`Remote ${tag} already points to a different commit.`);
    if (!remoteTag) run('git', ['push', 'origin', tag]);

    if (!hasRelease(tag)) {
      const notesPath = join(staging, 'release-notes.md');
      writeFileSync(notesPath, releaseNotes + '\n');
      run('gh', [
        'release',
        'create',
        tag,
        '--verify-tag',
        '--draft',
        '--title',
        `Dropfarmer ${version}`,
        '--notes-file',
        notesPath,
      ]);
    }
    run('gh', [
      'release',
      'upload',
      tag,
      installer,
      signature,
      latestPath,
      '--clobber',
    ]);
    run('gh', [
      'release',
      'edit',
      tag,
      '--draft=false',
      '--latest',
      '--verify-tag',
    ]);

    const release = JSON.parse(
      capture('gh', ['release', 'view', tag, '--json', 'url,isDraft,assets']),
    ) as {
      url: string;
      isDraft: boolean;
      assets: { name: string }[];
    };
    if (
      release.isDraft ||
      ![installerName, `${installerName}.sig`, 'latest.json'].every((name) =>
        release.assets.some((asset) => asset.name === name),
      )
    ) {
      fail('GitHub did not publish all release assets.');
    }
    console.log(`Published ${tag}: ${release.url}`);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
