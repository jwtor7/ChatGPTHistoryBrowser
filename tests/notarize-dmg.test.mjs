import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  assertNotarizationAccepted,
  createNotarytoolSubmitCommand,
  createStaplerStapleCommand,
  DEFAULT_RELEASE_DMG_ARCHITECTURE,
  DEFAULT_RELEASE_DMG_PRODUCT_NAME,
  notarizeAndStapleDmg,
  parseNotarytoolWaitOutput,
  readReleaseVersion,
  REQUIRED_NOTARIZATION_ENV,
  requireNotarizationCredentials,
  resolveReleaseDmgPath,
} from '../scripts/release/notarize-dmg.mjs';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const releaseWorkflowPath = path.join(repositoryRoot, '.github/workflows/release.yml');

function workflowOrder(workflow, left, right) {
  const leftIndex = workflow.indexOf(left);
  const rightIndex = workflow.indexOf(right);
  assert.notEqual(leftIndex, -1, `missing ${left}`);
  assert.notEqual(rightIndex, -1, `missing ${right}`);
  assert.ok(leftIndex < rightIndex, `${left} must appear before ${right}`);
}

test('reads the release version from tauri.conf.json', async () => {
  const tauri = JSON.parse(
    await readFile(path.join(repositoryRoot, 'src-tauri/tauri.conf.json'), 'utf8'),
  );
  assert.equal(await readReleaseVersion(repositoryRoot), tauri.version);
});

test('resolves the existing aarch64 GitHub Release DMG path', () => {
  const dmgPath = resolveReleaseDmgPath(repositoryRoot, '0.2.1');
  assert.equal(
    dmgPath,
    path.join(
      repositoryRoot,
      'src-tauri',
      'target',
      'release',
      'bundle',
      'dmg',
      `${DEFAULT_RELEASE_DMG_PRODUCT_NAME}_0.2.1_${DEFAULT_RELEASE_DMG_ARCHITECTURE}.dmg`,
    ),
  );
});

test('keeps an explicit workflow DMG path unchanged after resolution', () => {
  const relative =
    'src-tauri/target/release/bundle/dmg/ChatGPT History Browser_0.2.1_aarch64.dmg';
  assert.equal(
    resolveReleaseDmgPath(repositoryRoot, '0.2.1', relative),
    path.join(repositoryRoot, relative),
  );
});

test('requires the existing App Store Connect API key environment', () => {
  assert.deepEqual(REQUIRED_NOTARIZATION_ENV, [
    'APPLE_API_ISSUER',
    'APPLE_API_KEY',
    'APPLE_API_KEY_PATH',
  ]);
  assert.throws(
    () =>
      requireNotarizationCredentials({
        APPLE_API_ISSUER: 'issuer',
        APPLE_API_KEY: 'key',
      }),
    { code: 'MISSING_NOTARIZATION_CREDENTIALS' },
  );
});

test('submits the DMG with notarytool before stapling that same file', () => {
  const dmgPath = '/tmp/ChatGPT History Browser_0.2.1_aarch64.dmg';
  const credentials = {
    apiIssuer: 'test-issuer',
    apiKeyId: 'TESTKEYID1',
    apiKeyPath: '/tmp/AuthKey_TESTKEYID1.p8',
  };
  const submit = createNotarytoolSubmitCommand(dmgPath, credentials);
  const staple = createStaplerStapleCommand(dmgPath);

  assert.deepEqual(submit, [
    'xcrun',
    'notarytool',
    'submit',
    dmgPath,
    '--key',
    credentials.apiKeyPath,
    '--key-id',
    credentials.apiKeyId,
    '--issuer',
    credentials.apiIssuer,
    '--wait',
    '--output-format',
    'json',
  ]);
  assert.deepEqual(staple, ['xcrun', 'stapler', 'staple', dmgPath]);
  assert.equal(submit[3], staple[3]);
  assert.equal(submit.includes('staple'), false);
});

test('refuses to staple unless notarytool reports Accepted', () => {
  assert.doesNotThrow(() => assertNotarizationAccepted({ status: 'Accepted' }));
  assert.throws(() => assertNotarizationAccepted({ status: 'Invalid' }), {
    code: 'NOTARIZATION_NOT_ACCEPTED',
  });
  assert.throws(
    () => assertNotarizationAccepted(parseNotarytoolWaitOutput('{"status":"In Progress"}')),
    { code: 'NOTARIZATION_NOT_ACCEPTED' },
  );

  const dmgPath = '/tmp/ChatGPT History Browser_0.2.1_aarch64.dmg';
  const commands = [];
  assert.throws(
    () =>
      notarizeAndStapleDmg({
        dmgPath,
        env: {
          APPLE_API_ISSUER: 'test-issuer',
          APPLE_API_KEY: 'TESTKEYID1',
          APPLE_API_KEY_PATH: '/tmp/AuthKey_TESTKEYID1.p8',
        },
        runCommand: (command) => {
          commands.push(command);
          return { status: 0, stdout: '{"status":"Invalid"}', stderr: '' };
        },
      }),
    { code: 'NOTARIZATION_NOT_ACCEPTED' },
  );
  assert.equal(commands.length, 1);
  assert.equal(commands[0][1], 'notarytool');
});

test('does not staple-only when credentials or submit are missing', () => {
  const dmgPath = '/tmp/ChatGPT History Browser_0.2.1_aarch64.dmg';
  const commands = [];
  assert.throws(
    () =>
      notarizeAndStapleDmg({
        dmgPath,
        env: {},
        runCommand: (command) => {
          commands.push(command);
          return { status: 0, stdout: '', stderr: '' };
        },
      }),
    { code: 'MISSING_NOTARIZATION_CREDENTIALS' },
  );
  assert.deepEqual(commands, []);
});

test('staples only after Accepted and only the submitted DMG', () => {
  const dmgPath = '/tmp/ChatGPT History Browser_0.2.1_aarch64.dmg';
  const commands = [];
  const result = notarizeAndStapleDmg({
    dmgPath,
    env: {
      APPLE_API_ISSUER: 'test-issuer',
      APPLE_API_KEY: 'TESTKEYID1',
      APPLE_API_KEY_PATH: '/tmp/AuthKey_TESTKEYID1.p8',
    },
    runCommand: (command) => {
      commands.push(command);
      if (command[1] === 'notarytool') {
        return { status: 0, stdout: '{"status":"Accepted"}', stderr: '' };
      }
      return { status: 0, stdout: '', stderr: '' };
    },
  });

  assert.equal(result.status, 'Accepted');
  assert.equal(result.dmgPath, dmgPath);
  assert.equal(commands.length, 2);
  assert.equal(commands[0][1], 'notarytool');
  assert.equal(commands[0][2], 'submit');
  assert.equal(commands[1][1], 'stapler');
  assert.equal(commands[1][2], 'staple');
  assert.equal(commands[0][3], dmgPath);
  assert.equal(commands[1][3], dmgPath);
});

test('release workflow submits the DMG after tauri build and still validates tickets', async () => {
  const workflow = await readFile(releaseWorkflowPath, 'utf8');
  const packageJson = JSON.parse(
    await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'),
  );

  assert.equal(
    packageJson.scripts['tauri:build:release'],
    'RUSTFLAGS="--remap-path-prefix=${HOME}=/_local_build" tauri build',
  );
  assert.match(workflow, /run: npm run tauri:build:release/);
  assert.match(workflow, /node scripts\/release\/notarize-dmg\.mjs "\$dmg_path"/);
  assert.match(
    workflow,
    /dmg_path="src-tauri\/target\/release\/bundle\/dmg\/ChatGPT History Browser_\$\{GITHUB_REF_NAME#v\}_aarch64\.dmg"/,
  );
  assert.match(workflow, /xcrun stapler validate "\$app_path"/);
  assert.match(workflow, /xcrun stapler validate "\$dmg_path"/);
  assert.doesNotMatch(workflow, /tauri-action/);
  assert.doesNotMatch(workflow, /skip-stapling/);
  assert.doesNotMatch(workflow, /stapler validate.*\|\| true/);

  workflowOrder(
    workflow,
    'run: npm run tauri:build:release',
    'scripts/release/notarize-dmg.mjs',
  );
  workflowOrder(
    workflow,
    'scripts/release/notarize-dmg.mjs',
    'xcrun stapler validate "$app_path"',
  );
  workflowOrder(
    workflow,
    'xcrun stapler validate "$app_path"',
    'xcrun stapler validate "$dmg_path"',
  );
  workflowOrder(workflow, 'xcrun stapler validate "$dmg_path"', 'gh release create');
});
