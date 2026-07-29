#!/usr/bin/env node

/**
 * Privacy-scan the unpacked macOS application payload.
 *
 * Output contains aggregate counts or rule IDs with opaque file digests only.
 */

import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { lstat, open, readdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

import { scanBufferContent, scanRepositoryPath } from './audit-repo.mjs';

const MAX_PACKAGE_FILE_BYTES = 512 * 1024 * 1024;
const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, '..', '..');
const applicationRoot = path.join(
  repositoryRoot,
  'src-tauri',
  'target',
  'release',
  'bundle',
  'macos',
  'ChatGPT History Browser.app',
);

function opaquePath(relative) {
  return createHash('sha256')
    .update('package-path-v1\0')
    .update(relative)
    .digest('hex')
    .slice(0, 16);
}

async function readRegularFileNoFollow(absolute) {
  const flags =
    fsConstants.O_RDONLY |
    (typeof fsConstants.O_NOFOLLOW === 'number' ? fsConstants.O_NOFOLLOW : 0);
  const handle = await open(absolute, flags);
  try {
    const metadata = await handle.stat();
    if (!metadata.isFile() || metadata.nlink > 1 || metadata.size > MAX_PACKAGE_FILE_BYTES) {
      const error = new Error('Unsafe package file');
      error.code = 'UNSAFE_PACKAGE_FILE';
      throw error;
    }
    return { bytes: await handle.readFile(), size: metadata.size };
  } finally {
    await handle.close();
  }
}

async function walk(directory, findings, totals) {
  const entries = await readdir(directory, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));

  for (const entry of entries) {
    const absolute = path.join(directory, entry.name);
    const relative = path.relative(applicationRoot, absolute).split(path.sep).join('/');
    const metadata = await lstat(absolute);
    const digest = opaquePath(relative);

    if (metadata.isSymbolicLink() || (!metadata.isDirectory() && !metadata.isFile())) {
      findings.add(`PACKAGE_FILESYSTEM_ENTRY\u0000${digest}`);
      continue;
    }
    if (metadata.isDirectory()) {
      await walk(absolute, findings, totals);
      continue;
    }

    for (const ruleId of scanRepositoryPath(relative, 'build')) {
      findings.add(`${ruleId}\u0000${digest}`);
    }
    try {
      const { bytes, size } = await readRegularFileNoFollow(absolute);
      totals.files += 1;
      totals.bytes += size;
      for (const ruleId of scanBufferContent(bytes)) {
        findings.add(`${ruleId}\u0000${digest}`);
      }
    } catch {
      findings.add(`PACKAGE_FILE_REJECTED\u0000${digest}`);
    }
  }
}

async function main() {
  if (process.argv.length > 2) {
    process.stderr.write('PACKAGE_PRIVACY_AUDIT_ERROR INVALID_ARGUMENT\n');
    process.exitCode = 2;
    return;
  }

  try {
    const metadata = await lstat(applicationRoot);
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      throw new Error('Invalid application bundle');
    }
    const findings = new Set();
    const totals = { files: 0, bytes: 0 };
    await walk(applicationRoot, findings, totals);
    if (findings.size === 0) {
      process.stdout.write(
        `PACKAGE_PRIVACY_AUDIT_PASS files=${totals.files} bytes=${totals.bytes}\n`,
      );
      return;
    }
    for (const finding of [...findings].sort()) {
      const [ruleId, digest] = finding.split('\u0000');
      process.stdout.write(`${ruleId} path:${digest}\n`);
    }
    process.stdout.write(`PACKAGE_PRIVACY_AUDIT_FAIL ${findings.size}\n`);
    process.exitCode = 1;
  } catch {
    process.stderr.write('PACKAGE_PRIVACY_AUDIT_ERROR PACKAGE_UNAVAILABLE\n');
    process.exitCode = 2;
  }
}

await main();
