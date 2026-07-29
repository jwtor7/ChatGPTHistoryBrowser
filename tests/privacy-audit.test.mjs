import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { scanBufferContent, scanRepositoryPath } from '../scripts/privacy/audit-repo.mjs';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('detects common private values without returning matched bytes', () => {
  const syntheticEmail = ['person', ['company', 'example', 'biz'].join('.')].join('@');
  const syntheticHomePath = ['', 'Users', 'private-user', 'Documents', 'item'].join('/');
  const findings = scanBufferContent(
    Buffer.from(
      [
        `contact=${syntheticEmail}`,
        `path=${syntheticHomePath}`,
        'credential=sk-' + 'A'.repeat(24),
      ].join('\n'),
    ),
  );

  assert.deepEqual(
    new Set(findings),
    new Set(['PII_NON_RESERVED_EMAIL', 'PII_ABSOLUTE_HOME_PATH', 'SECRET_OPENAI_KEY']),
  );
  assert.ok(findings.every((finding) => /^[A-Z0-9_]+$/.test(finding)));
});

test('accepts reserved-domain synthetic addresses', () => {
  assert.deepEqual(scanBufferContent(Buffer.from('fictional.person@example.invalid')), []);
});

test('accepts GitHub platform no-reply and bot addresses', () => {
  assert.deepEqual(
    scanBufferContent(Buffer.from('49699333+dependabot[bot]@users.noreply.github.com')),
    [],
  );
  assert.deepEqual(scanBufferContent(Buffer.from('noreply@github.com')), []);
  assert.deepEqual(scanBufferContent(Buffer.from('support@github.com')), []);
});

test('accepts conventional density-scaled image filenames', () => {
  assert.deepEqual(scanBufferContent(Buffer.from('AppIcon-20x20@3x.png')), []);
  assert.deepEqual(scanBufferContent(Buffer.from('AppIcon-512@2x.png')), []);
  assert.deepEqual(scanRepositoryPath('src-tauri/icons/128x128@2x.png'), []);
});

test('does not mistake a long minified token for an email address', () => {
  const minifiedToken =
    'lnnfmwSltfqfgjmojmf8slufqwz`kbnafqOjujmd#ulovnfpBmwklmzoldjm!#' +
    'QfobwfgF`lmlnzqfb`kfp`vwwjmddqbujwzojef#jm@kbswfq.pkbgltMlwbaof';
  assert.deepEqual(scanBufferContent(Buffer.from(minifiedToken)), []);
});

test('rejects export-shaped and production source-map paths', () => {
  assert.ok(
    scanRepositoryPath('private/conversations-000.json').includes('PROHIBITED_EXPORT_ARTIFACT'),
  );
  assert.ok(
    scanRepositoryPath('dist/assets/app.js.map', 'build').includes('PRODUCTION_SOURCE_MAP'),
  );
});

test('CLI findings never echo a private filename canary', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'history-browser-privacy-audit-'));
  const canary = ['PRIVATE', 'FILENAME', 'CANARY'].join('_');
  const filename = ['conversations', canary].join('-') + '.json';

  try {
    const initialized = spawnSync('git', ['init', '--quiet'], {
      cwd: directory,
      encoding: 'utf8',
    });
    assert.equal(initialized.status, 0);
    await writeFile(path.join(directory, filename), '[]', {
      encoding: 'utf8',
      flag: 'wx',
    });

    const result = spawnSync(
      process.execPath,
      [path.join(repositoryRoot, 'scripts/privacy/audit-repo.mjs'), 'worktree'],
      {
        cwd: directory,
        encoding: 'utf8',
      },
    );

    assert.equal(result.status, 1);
    assert.match(result.stdout, /^PROHIBITED_EXPORT_ARTIFACT path:[0-9a-f]{16}$/m);
    assert.doesNotMatch(result.stdout, new RegExp(canary));
    assert.doesNotMatch(result.stderr, new RegExp(canary));
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});

test('local detailed security reports are ignored', () => {
  const result = spawnSync('git', ['check-ignore', '--quiet', 'SecurityReport.md'], {
    cwd: repositoryRoot,
  });
  assert.equal(result.status, 0);
});

test('staged filename findings are opaque and include path privacy rules', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'history-browser-staged-audit-'));
  const canary = ['synthetic-person', 'private.example.biz'].join('@');

  try {
    assert.equal(
      spawnSync('git', ['init', '--quiet'], { cwd: directory, encoding: 'utf8' }).status,
      0,
    );
    await writeFile(path.join(directory, `${canary}.txt`), 'synthetic', {
      encoding: 'utf8',
      flag: 'wx',
    });
    assert.equal(
      spawnSync('git', ['add', '--', `${canary}.txt`], {
        cwd: directory,
        encoding: 'utf8',
      }).status,
      0,
    );

    const result = spawnSync(
      process.execPath,
      [path.join(repositoryRoot, 'scripts/privacy/audit-repo.mjs'), 'staged'],
      { cwd: directory, encoding: 'utf8' },
    );

    assert.equal(result.status, 1);
    assert.match(result.stdout, /^PATH_PII_NON_RESERVED_EMAIL path:[0-9a-f]{16}$/m);
    assert.doesNotMatch(result.stdout, new RegExp(canary));
    assert.doesNotMatch(result.stderr, new RegExp(canary));
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});
