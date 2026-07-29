#!/usr/bin/env node

/**
 * Scan every object in the local Git object database, including unreachable
 * loose and packed objects. Output is deliberately limited to a rule ID and an
 * object ID; matched content, commit messages, refs, and paths are never shown.
 */

import { Buffer } from 'node:buffer';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

import {
  MAX_REPOSITORY_FILE_BYTES,
  findRepositoryRoot,
  scanBufferContent,
  scanRepositoryPath,
} from './audit-repo.mjs';

const MAX_OBJECT_LIST_BYTES = 128 * 1024 * 1024;
const OBJECT_ID_PATTERN = /^[0-9a-f]{40,64}$/;

function git(root, args, options = {}) {
  const result = spawnSync('git', args, {
    cwd: root,
    encoding: options.encoding ?? null,
    maxBuffer: options.maxBuffer ?? MAX_REPOSITORY_FILE_BYTES + 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  if (result.error || result.status !== 0) {
    if (options.allowFailure) return options.encoding ? '' : Buffer.alloc(0);
    const error = new Error('Git command failed');
    error.code = 'GIT_COMMAND_FAILED';
    throw error;
  }

  return result.stdout;
}

function addFinding(findings, ruleId, objectId) {
  const safeObjectId = OBJECT_ID_PATTERN.test(objectId) ? objectId : 'UNKNOWN_OBJECT';
  findings.add(`${ruleId}\u0000${safeObjectId}`);
}

function enumerateObjects(root) {
  const output = git(
    root,
    [
      'cat-file',
      '--batch-all-objects',
      '--batch-check=%(objectname) %(objecttype) %(objectsize)',
    ],
    { encoding: 'utf8', maxBuffer: MAX_OBJECT_LIST_BYTES },
  );

  const objects = [];
  for (const line of output.split('\n')) {
    if (!line) continue;
    const [oid, type, sizeText] = line.split(' ');
    const size = Number(sizeText);
    if (
      !OBJECT_ID_PATTERN.test(oid) ||
      !['blob', 'commit', 'tag', 'tree'].includes(type) ||
      !Number.isSafeInteger(size) ||
      size < 0
    ) {
      const error = new Error('Invalid Git object metadata');
      error.code = 'INVALID_GIT_OBJECT_METADATA';
      throw error;
    }
    objects.push({ oid, type, size });
  }
  return objects;
}

function scanObjectContent(root, object, findings) {
  if (object.size > MAX_REPOSITORY_FILE_BYTES) {
    addFinding(findings, 'GIT_OBJECT_TOO_LARGE', object.oid);
    return;
  }

  if (object.type === 'tree') {
    const tree = git(root, ['ls-tree', '-z', object.oid], {
      maxBuffer: MAX_REPOSITORY_FILE_BYTES + 1024 * 1024,
    });
    for (const entry of tree.toString('utf8').split('\u0000')) {
      if (!entry) continue;
      const tab = entry.indexOf('\t');
      if (tab === -1) {
        addFinding(findings, 'GIT_TREE_INVALID_ENTRY', object.oid);
        continue;
      }
      const metadata = entry.slice(0, tab).split(' ');
      const name = entry.slice(tab + 1);
      const isDirectory = metadata[1] === 'tree' || metadata[1] === 'commit';
      for (const ruleId of scanRepositoryPath(name, 'source', { isDirectory })) {
        addFinding(findings, ruleId, object.oid);
      }
      for (const ruleId of scanBufferContent(Buffer.from(name, 'utf8'))) {
        addFinding(findings, ruleId, object.oid);
      }
    }
    return;
  }

  const content = git(root, ['cat-file', object.type, object.oid], {
    maxBuffer: MAX_REPOSITORY_FILE_BYTES + 1024 * 1024,
  });
  for (const ruleId of scanBufferContent(content)) {
    addFinding(findings, ruleId, object.oid);
  }
}

function scanRefs(root, findings) {
  const output = git(root, ['for-each-ref', '--format=%(objectname)%00%(refname)'], {
    encoding: 'utf8',
    maxBuffer: MAX_OBJECT_LIST_BYTES,
  });

  for (const line of output.split('\n')) {
    if (!line) continue;
    const separator = line.indexOf('\u0000');
    if (separator === -1) continue;
    const oid = line.slice(0, separator);
    const ref = line.slice(separator + 1);
    for (const ruleId of scanBufferContent(Buffer.from(ref, 'utf8'))) {
      addFinding(findings, ruleId, oid);
    }
    for (const ruleId of scanRepositoryPath(ref, 'source')) {
      addFinding(findings, ruleId, oid);
    }
  }
}

function scanReflogs(root, findings) {
  const output = git(root, ['reflog', 'show', '--all', '--format=%H%x00%gs'], {
    encoding: 'utf8',
    maxBuffer: MAX_OBJECT_LIST_BYTES,
    allowFailure: true,
  });

  for (const line of output.split('\n')) {
    if (!line) continue;
    const separator = line.indexOf('\u0000');
    if (separator === -1) continue;
    const oid = line.slice(0, separator);
    const subject = line.slice(separator + 1);
    for (const ruleId of scanBufferContent(Buffer.from(subject, 'utf8'))) {
      addFinding(findings, ruleId, oid);
    }
  }
}

function printFindings(findings) {
  const ordered = [...findings]
    .map((entry) => {
      const separator = entry.indexOf('\u0000');
      return {
        ruleId: entry.slice(0, separator),
        objectId: entry.slice(separator + 1),
      };
    })
    .sort(
      (left, right) =>
        left.ruleId.localeCompare(right.ruleId) || left.objectId.localeCompare(right.objectId),
    );

  if (ordered.length === 0) {
    process.stdout.write('GIT_OBJECT_PRIVACY_AUDIT_PASS\n');
    return 0;
  }

  for (const finding of ordered) {
    process.stdout.write(`${finding.ruleId} ${finding.objectId}\n`);
  }
  process.stdout.write(`GIT_OBJECT_PRIVACY_AUDIT_FAIL ${ordered.length}\n`);
  return 1;
}

export function runGitObjectAudit({ cwd = process.cwd() } = {}) {
  const root = findRepositoryRoot(cwd);
  const findings = new Set();
  for (const object of enumerateObjects(root)) {
    scanObjectContent(root, object, findings);
  }
  scanRefs(root, findings);
  scanReflogs(root, findings);
  return findings;
}

function printHelp() {
  process.stdout.write(
    [
      'Usage: node scripts/privacy/audit-git-objects.mjs',
      '',
      'Scans all local Git objects, refs, and reflog subjects.',
      '',
    ].join('\n'),
  );
}

function main() {
  if (process.argv.includes('--help') || process.argv.includes('-h')) {
    printHelp();
    return;
  }
  if (process.argv.length > 2) {
    process.stderr.write('GIT_OBJECT_PRIVACY_AUDIT_ERROR INVALID_ARGUMENT\n');
    process.exitCode = 2;
    return;
  }

  try {
    const findings = runGitObjectAudit();
    process.exitCode = printFindings(findings);
  } catch (error) {
    const code =
      typeof error?.code === 'string' ? error.code : 'GIT_OBJECT_AUDIT_INTERNAL_ERROR';
    process.stderr.write(`GIT_OBJECT_PRIVACY_AUDIT_ERROR ${code}\n`);
    process.exitCode = 2;
  }
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : '';
if (import.meta.url === invokedPath) {
  main();
}
