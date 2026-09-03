#!/usr/bin/env node

/**
 * Submit the signed GitHub Release DMG to Apple notarization and staple it.
 *
 * Tauri 2.11.5 notarizes and staples the .app, then wraps that app in a signed
 * DMG that is never submitted. Notarization tickets are per-cdhash, so the
 * stapled .app ticket cannot be reused on the DMG.
 */

import { spawnSync } from 'node:child_process';
import { lstat, readFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

export const REQUIRED_NOTARIZATION_ENV = [
  'APPLE_API_ISSUER',
  'APPLE_API_KEY',
  'APPLE_API_KEY_PATH',
];

export const DEFAULT_RELEASE_DMG_ARCHITECTURE = 'aarch64';
export const DEFAULT_RELEASE_DMG_PRODUCT_NAME = 'ChatGPT History Browser';

export function createError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

export function resolveReleaseDmgPath(repositoryRoot, version, explicitPath) {
  if (explicitPath) {
    return path.isAbsolute(explicitPath)
      ? explicitPath
      : path.resolve(repositoryRoot, explicitPath);
  }

  return path.join(
    repositoryRoot,
    'src-tauri',
    'target',
    'release',
    'bundle',
    'dmg',
    `${DEFAULT_RELEASE_DMG_PRODUCT_NAME}_${version}_${DEFAULT_RELEASE_DMG_ARCHITECTURE}.dmg`,
  );
}

export function requireNotarizationCredentials(env) {
  const missing = REQUIRED_NOTARIZATION_ENV.filter((name) => !String(env[name] ?? '').trim());
  if (missing.length > 0) {
    throw createError(
      'MISSING_NOTARIZATION_CREDENTIALS',
      `Missing required environment variables: ${missing.join(', ')}`,
    );
  }

  return {
    apiIssuer: env.APPLE_API_ISSUER,
    apiKeyId: env.APPLE_API_KEY,
    apiKeyPath: env.APPLE_API_KEY_PATH,
  };
}

export function createNotarytoolSubmitCommand(dmgPath, credentials) {
  return [
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
  ];
}

export function createStaplerStapleCommand(dmgPath) {
  return ['xcrun', 'stapler', 'staple', dmgPath];
}

export function parseNotarytoolWaitOutput(stdout) {
  const text = String(stdout ?? '').trim();
  try {
    return JSON.parse(text);
  } catch {
    const start = text.lastIndexOf('{');
    const end = text.lastIndexOf('}');
    if (start >= 0 && end > start) {
      return JSON.parse(text.slice(start, end + 1));
    }
    throw createError('NOTARYTOOL_OUTPUT_UNPARSEABLE', 'Unable to parse notarytool output');
  }
}

export function assertNotarizationAccepted(payload) {
  const status = typeof payload?.status === 'string' ? payload.status : '';
  if (status !== 'Accepted') {
    throw createError(
      'NOTARIZATION_NOT_ACCEPTED',
      `Notarization status was ${status || 'UNKNOWN'}, expected Accepted`,
    );
  }
}

export async function readReleaseVersion(repositoryRoot) {
  try {
    const tauri = JSON.parse(
      await readFile(path.join(repositoryRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
    );
    if (typeof tauri.version !== 'string' || !tauri.version.trim()) {
      throw createError('RELEASE_VERSION_UNAVAILABLE', 'tauri.conf.json version is missing');
    }
    return tauri.version;
  } catch (error) {
    if (error?.code === 'RELEASE_VERSION_UNAVAILABLE') {
      throw error;
    }
    throw createError('RELEASE_VERSION_UNAVAILABLE', 'Unable to read tauri.conf.json version');
  }
}

export async function assertRegularDmgFile(dmgPath) {
  try {
    const metadata = await lstat(dmgPath);
    if (!metadata.isFile()) {
      throw createError('RELEASE_DMG_UNAVAILABLE', 'Release DMG is not a regular file');
    }
  } catch (error) {
    if (error?.code === 'RELEASE_DMG_UNAVAILABLE') {
      throw error;
    }
    throw createError('RELEASE_DMG_UNAVAILABLE', 'Release DMG is not available');
  }
}

export function notarizeAndStapleDmg({ dmgPath, env, runCommand }) {
  const credentials = requireNotarizationCredentials(env);
  const submitCommand = createNotarytoolSubmitCommand(dmgPath, credentials);
  const submitResult = runCommand(submitCommand);
  if (submitResult.status !== 0) {
    throw createError('NOTARYTOOL_SUBMIT_FAILED', 'notarytool submit failed');
  }

  const payload = parseNotarytoolWaitOutput(submitResult.stdout);
  assertNotarizationAccepted(payload);

  const stapleCommand = createStaplerStapleCommand(dmgPath);
  if (submitCommand[3] !== dmgPath || stapleCommand[3] !== dmgPath) {
    throw createError('RELEASE_DMG_PATH_MISMATCH', 'Submit and staple must use the same DMG');
  }

  const stapleResult = runCommand(stapleCommand);
  if (stapleResult.status !== 0) {
    throw createError('STAPLE_FAILED', 'stapler staple failed');
  }

  return { dmgPath, status: payload.status };
}

function defaultRunCommand(command) {
  return spawnSync(command[0], command.slice(1), {
    encoding: 'utf8',
  });
}

function writeToolOutput(result) {
  if (result.stdout) {
    process.stdout.write(result.stdout);
  }
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
}

async function main() {
  const repositoryRoot = process.cwd();
  const explicitPath = process.argv[2];
  const version = explicitPath ? undefined : await readReleaseVersion(repositoryRoot);
  const dmgPath = resolveReleaseDmgPath(repositoryRoot, version, explicitPath);

  await assertRegularDmgFile(dmgPath);
  process.stderr.write(`Submitting ${dmgPath} to Apple notarization\n`);

  notarizeAndStapleDmg({
    dmgPath,
    env: process.env,
    runCommand: (command) => {
      const result = defaultRunCommand(command);
      writeToolOutput(result);
      return result;
    },
  });

  process.stderr.write(`Stapled notarization ticket onto ${dmgPath}\n`);
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : '';
if (import.meta.url === invokedPath) {
  try {
    await main();
  } catch (error) {
    const code = typeof error?.code === 'string' ? error.code : 'NOTARIZATION_INTERNAL_ERROR';
    process.stderr.write(`RELEASE_DMG_NOTARIZATION_ERROR ${code}\n`);
    process.exitCode = 1;
  }
}
