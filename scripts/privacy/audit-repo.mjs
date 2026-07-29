#!/usr/bin/env node

/**
 * Dependency-free privacy audit for repository files and Git index content.
 *
 * Safety property: findings contain only a rule ID and an opaque digest of the
 * repository-relative path. Paths and matched bytes never reach stdout/stderr.
 */

import { spawnSync } from 'node:child_process';
import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { lstat, open, readdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

export const MAX_REPOSITORY_FILE_BYTES = 5 * 1024 * 1024;

const VALID_SCOPES = new Set(['worktree', 'staged', 'tracked', 'build', 'all']);
const BUILD_DIRECTORIES = ['dist', 'build', 'out', '.next', '.output'];
const DEPENDENCY_DIRECTORIES = new Set(['node_modules', '.pnpm-store', '.yarn', '.npm']);
const GENERATED_WORKTREE_DIRECTORIES = new Set([
  '.code-review-graph',
  'dist',
  'target',
  'src-tauri/gen',
  'src-tauri/target',
]);

const SECRET_PATTERNS = [
  {
    id: 'SECRET_PRIVATE_KEY',
    regex: /-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----/,
  },
  { id: 'SECRET_AWS_ACCESS_KEY', regex: /\bAKIA[0-9A-Z]{16}\b/ },
  { id: 'SECRET_GITHUB_TOKEN', regex: /\bgh[pousr]_[A-Za-z0-9]{30,255}\b/ },
  {
    id: 'SECRET_GITHUB_FINE_GRAINED_TOKEN',
    regex: /\bgithub_pat_[A-Za-z0-9_]{40,255}\b/,
  },
  { id: 'SECRET_GITLAB_TOKEN', regex: /\bglpat-[A-Za-z0-9_-]{20,255}\b/ },
  { id: 'SECRET_NPM_TOKEN', regex: /\bnpm_[A-Za-z0-9]{36,255}\b/ },
  {
    id: 'SECRET_HUGGING_FACE_TOKEN',
    regex: /\bhf_[A-Za-z0-9]{30,255}\b/,
  },
  {
    id: 'SECRET_GOOGLE_API_KEY',
    regex: /\bAIza[0-9A-Za-z_-]{35}\b/,
  },
  { id: 'SECRET_OPENAI_KEY', regex: /\bsk-[A-Za-z0-9_-]{20,255}\b/ },
  {
    id: 'SECRET_ANTHROPIC_KEY',
    regex: /\bsk-ant-[A-Za-z0-9_-]{20,255}\b/,
  },
  {
    id: 'SECRET_SLACK_TOKEN',
    regex: /\bxox[baprs]-[A-Za-z0-9-]{10,255}\b/,
  },
  { id: 'SECRET_STRIPE_LIVE_KEY', regex: /\bsk_live_[A-Za-z0-9]{16,255}\b/ },
  {
    id: 'SECRET_JWT',
    regex: /\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\b/,
  },
  {
    id: 'SECRET_BEARER_TOKEN',
    regex: /\bBearer\s+[A-Za-z0-9._~+/=-]{20,255}\b/i,
  },
  {
    id: 'SECRET_ASSIGNED_CREDENTIAL',
    regex:
      /\b(?:api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|password|passwd)\b\s*[:=]\s*["'][^"'\r\n]{12,}["']/i,
  },
  {
    id: 'SECRET_CREDENTIAL_URL',
    regex: /\b[a-z][a-z0-9+.-]*:\/\/[^/\s:@]{1,128}:[^/\s@]{8,128}@/i,
  },
];

const EMAIL_PATTERN =
  /(?<![A-Z0-9.!#$%&'*+/?^_`{|}~-])[A-Z0-9][A-Z0-9._%+-]{0,63}@[A-Z0-9](?:[A-Z0-9-]{0,61}[A-Z0-9])?(?:\.[A-Z0-9](?:[A-Z0-9-]{0,61}[A-Z0-9])?)+\b/gi;
const PHONE_PATTERN = /\b(?:\+[1-9]\d{0,2}[\s.-]?)?(?:\(\d{3}\)|\d{3})[\s.-]\d{3}[\s.-]\d{4}\b/;
const GOVERNMENT_ID_PATTERN = /\b\d{3}-\d{2}-\d{4}\b/;

function git(root, args, options = {}) {
  const result = spawnSync('git', args, {
    cwd: root,
    encoding: options.encoding ?? null,
    maxBuffer: options.maxBuffer ?? MAX_REPOSITORY_FILE_BYTES + 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  if (result.error || result.status !== 0) {
    const error = new Error('Git command failed');
    error.code = 'GIT_COMMAND_FAILED';
    throw error;
  }

  return result.stdout;
}

export function findRepositoryRoot(cwd = process.cwd()) {
  const result = spawnSync('git', ['rev-parse', '--show-toplevel'], {
    cwd,
    encoding: 'utf8',
    maxBuffer: 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  if (result.error || result.status !== 0) {
    const error = new Error('Not a Git repository');
    error.code = 'NOT_A_GIT_REPOSITORY';
    throw error;
  }

  return result.stdout.trim();
}

function normalizeRelativePath(value) {
  return value.split(path.sep).join('/');
}

function relativePath(root, absolutePath) {
  const relative = path.relative(root, absolutePath);
  if (
    relative === '' ||
    relative === '..' ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  ) {
    const error = new Error('Path is outside repository');
    error.code = 'PATH_OUTSIDE_REPOSITORY';
    throw error;
  }
  return normalizeRelativePath(relative);
}

function addFinding(findings, ruleId, target) {
  const normalizedTarget = normalizeRelativePath(String(target));
  const targetDigest = createHash('sha256')
    .update('repository-path-v1\0')
    .update(normalizedTarget)
    .digest('hex')
    .slice(0, 16);
  const key = `${ruleId}\u0000${targetDigest}`;
  if (!findings.has(key)) {
    findings.set(key, { ruleId, targetDigest });
  }
}

function containsControlCharacter(value) {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint <= 0x1f || codePoint === 0x7f) return true;
  }
  return false;
}

function isReservedEmail(email) {
  const normalizedEmail = email.toLowerCase();
  const separator = email.lastIndexOf('@');
  if (separator === -1) return false;
  const domain = email.slice(separator + 1).toLowerCase();
  return (
    domain === 'users.noreply.github.com' ||
    normalizedEmail === 'noreply@github.com' ||
    normalizedEmail === 'support@github.com' ||
    domain === 'example.com' ||
    domain.endsWith('.example.com') ||
    domain === 'example.org' ||
    domain.endsWith('.example.org') ||
    domain === 'example.net' ||
    domain.endsWith('.example.net') ||
    domain === 'example' ||
    domain.endsWith('.example') ||
    domain === 'test' ||
    domain.endsWith('.test') ||
    domain === 'invalid' ||
    domain.endsWith('.invalid') ||
    domain === 'localhost' ||
    domain.endsWith('.localhost')
  );
}

function isDensityAssetFilename(emailLikeToken) {
  return /^(?:[a-z][a-z0-9_-]*-)?\d+(?:\.\d+)?(?:x\d+(?:\.\d+)?)?@[1-9]\d*x(?:-\d+)?\.(?:gif|jpe?g|png|webp)$/i.test(
    emailLikeToken,
  );
}

function homePathPatterns() {
  const slash = '/';
  const macPrefix = [slash, 'Users', slash].join('');
  const linuxPrefix = [slash, 'home', slash].join('');
  const rootPrefix = [slash, 'root', slash].join('');

  return [
    new RegExp(`${macPrefix.replaceAll('/', '\\/')}[^/\\s"'<>]+${slash.replace('/', '\\/')}`),
    new RegExp(`${linuxPrefix.replaceAll('/', '\\/')}[^/\\s"'<>]+${slash.replace('/', '\\/')}`),
    new RegExp(rootPrefix.replaceAll('/', '\\/')),
    /[A-Za-z]:[\\/]+Users[\\/]+[^\\/\s"'<>]+[\\/]/i,
  ];
}

export function scanBufferContent(buffer) {
  const ruleIds = new Set();
  const text = buffer.toString('utf8');

  for (const pattern of homePathPatterns()) {
    if (pattern.test(text)) ruleIds.add('PII_ABSOLUTE_HOME_PATH');
  }

  EMAIL_PATTERN.lastIndex = 0;
  for (const match of text.matchAll(EMAIL_PATTERN)) {
    if (!isReservedEmail(match[0]) && !isDensityAssetFilename(match[0])) {
      ruleIds.add('PII_NON_RESERVED_EMAIL');
      break;
    }
  }

  if (PHONE_PATTERN.test(text)) ruleIds.add('PII_PHONE_LIKE');
  if (GOVERNMENT_ID_PATTERN.test(text)) ruleIds.add('PII_GOVERNMENT_ID_LIKE');

  for (const { id, regex } of SECRET_PATTERNS) {
    if (regex.test(text)) ruleIds.add(id);
  }

  return [...ruleIds];
}

export function scanRepositoryPath(
  repositoryRelativePath,
  context = 'source',
  { isDirectory = false } = {},
) {
  const ruleIds = new Set();
  const normalized = normalizeRelativePath(repositoryRelativePath);
  const segments = normalized.split('/');
  const directorySegments = isDirectory ? segments : segments.slice(0, -1);
  const basename = segments.at(-1) ?? '';
  const lowerBasename = basename.toLowerCase();

  for (const ruleId of scanBufferContent(Buffer.from(normalized, 'utf8'))) {
    ruleIds.add(`PATH_${ruleId}`);
  }

  if (normalized.includes('\uFFFD')) ruleIds.add('PATH_INVALID_ENCODING');
  if (containsControlCharacter(normalized)) {
    ruleIds.add('PATH_CONTROL_CHARACTER');
  }

  if (
    lowerBasename === 'chat.html' ||
    /^conversations(?:-[^/]+)?\.json$/i.test(basename) ||
    /\.dat$/i.test(basename)
  ) {
    ruleIds.add('PROHIBITED_EXPORT_ARTIFACT');
  }

  if (/\.(?:zip|tar|tar\.gz|tgz|7z|rar)$/i.test(basename)) {
    ruleIds.add('PROHIBITED_ARCHIVE');
  }

  if (
    /\.(?:sqlite|sqlite3|db)(?:-(?:wal|shm))?$/i.test(basename) ||
    /\.(?:wal|shm)$/i.test(basename)
  ) {
    ruleIds.add('PROHIBITED_DATABASE');
  }

  if (/\.log$/i.test(basename)) ruleIds.add('PROHIBITED_LOG');
  if (lowerBasename === '.ds_store') ruleIds.add('PROHIBITED_OS_METADATA');

  if (
    (lowerBasename === '.env' || lowerBasename.startsWith('.env.')) &&
    lowerBasename !== '.env.example'
  ) {
    ruleIds.add('PROHIBITED_ENV_FILE');
  }

  if (
    directorySegments.some((segment) =>
      /^(?:(?:openai|[a-z]*gpt)[ _-]*(?:data[ _-]*)?export(?:s)?(?:[ _-].*)?|[a-z]*gpt[ _-]*(?:backup|data)(?:[ _-].*)?|exports?)$/i.test(
        segment,
      ),
    )
  ) {
    ruleIds.add('PROHIBITED_EXPORT_DIRECTORY');
  }

  if (
    directorySegments.some((segment) =>
      /^(?:\.?cache|caches|logs|\.?local-data|\.?indexes?|indices|\.chatgpt-history-browser)$/i.test(
        segment,
      ),
    )
  ) {
    ruleIds.add('PROHIBITED_PRIVATE_DATA_DIRECTORY');
  }

  if (context === 'build' && /\.map$/i.test(basename)) {
    ruleIds.add('PRODUCTION_SOURCE_MAP');
  }

  return [...ruleIds];
}

async function readRegularFileNoFollow(absolutePath) {
  const flags =
    fsConstants.O_RDONLY |
    (typeof fsConstants.O_NOFOLLOW === 'number' ? fsConstants.O_NOFOLLOW : 0);
  const handle = await open(absolutePath, flags);
  try {
    const stats = await handle.stat();
    if (!stats.isFile()) {
      const error = new Error('Not a regular file');
      error.code = 'NOT_REGULAR_FILE';
      throw error;
    }
    if (stats.nlink > 1) {
      const error = new Error('Hard-linked file');
      error.code = 'HARD_LINK';
      throw error;
    }
    if (stats.size > MAX_REPOSITORY_FILE_BYTES) {
      const error = new Error('File exceeds size limit');
      error.code = 'LARGE_FILE';
      throw error;
    }
    return await handle.readFile();
  } finally {
    await handle.close();
  }
}

async function scanDiskFile(root, absolutePath, relative, context, findings) {
  for (const ruleId of scanRepositoryPath(relative, context, { isDirectory: false })) {
    addFinding(findings, ruleId, relative);
  }

  let buffer;
  try {
    buffer = await readRegularFileNoFollow(absolutePath);
  } catch (error) {
    const ruleId =
      error.code === 'HARD_LINK'
        ? 'FILESYSTEM_HARD_LINK'
        : error.code === 'LARGE_FILE'
          ? 'FILE_TOO_LARGE'
          : error.code === 'ELOOP'
            ? 'FILESYSTEM_SYMLINK'
            : 'FILE_READ_ERROR';
    addFinding(findings, ruleId, relative);
    return;
  }

  for (const ruleId of scanBufferContent(buffer)) {
    addFinding(findings, ruleId, relative);
  }
}

async function walkDisk(root, absoluteDirectory, context, findings, options = {}) {
  let entries;
  try {
    entries = await readdir(absoluteDirectory, { withFileTypes: true });
  } catch {
    const relative = absoluteDirectory === root ? '.' : relativePath(root, absoluteDirectory);
    addFinding(findings, 'DIRECTORY_READ_ERROR', relative);
    return;
  }

  entries.sort((left, right) => left.name.localeCompare(right.name));

  for (const entry of entries) {
    if (absoluteDirectory === root && entry.name === '.git') continue;

    const absolute = path.join(absoluteDirectory, entry.name);
    const relative = relativePath(root, absolute);

    let stats;
    try {
      stats = await lstat(absolute);
    } catch {
      addFinding(findings, 'FILESYSTEM_STAT_ERROR', relative);
      continue;
    }

    const pathRules = new Set(
      scanRepositoryPath(relative, context, { isDirectory: stats.isDirectory() }),
    );
    for (const ruleId of pathRules) {
      addFinding(findings, ruleId, relative);
    }

    if (stats.isSymbolicLink()) {
      addFinding(findings, 'FILESYSTEM_SYMLINK', relative);
      continue;
    }

    if (stats.isDirectory()) {
      if (options.skipDependencies && DEPENDENCY_DIRECTORIES.has(entry.name)) {
        continue;
      }
      if (options.skipGenerated && GENERATED_WORKTREE_DIRECTORIES.has(relative)) {
        continue;
      }
      if (
        pathRules.has('PROHIBITED_EXPORT_DIRECTORY') ||
        pathRules.has('PROHIBITED_PRIVATE_DATA_DIRECTORY')
      ) {
        continue;
      }
      await walkDisk(root, absolute, context, findings, options);
      continue;
    }

    if (!stats.isFile()) {
      addFinding(findings, 'FILESYSTEM_NON_REGULAR_FILE', relative);
      continue;
    }

    await scanDiskFile(root, absolute, relative, context, findings);
  }
}

function parseIndexEntries(buffer) {
  const entries = [];
  for (const rawEntry of buffer.toString('utf8').split('\u0000')) {
    if (!rawEntry) continue;
    const tab = rawEntry.indexOf('\t');
    if (tab === -1) continue;
    const metadata = rawEntry.slice(0, tab).split(' ');
    if (metadata.length < 3) continue;
    entries.push({
      mode: metadata[0],
      oid: metadata[1],
      stage: metadata[2],
      path: normalizeRelativePath(rawEntry.slice(tab + 1)),
    });
  }
  return entries;
}

function scanIndexBlob(root, entry, findings, contentCache) {
  for (const ruleId of scanRepositoryPath(entry.path, 'source', {
    isDirectory: entry.mode === '160000',
  })) {
    addFinding(findings, ruleId, entry.path);
  }

  if (entry.mode === '120000') {
    addFinding(findings, 'GIT_SYMLINK', entry.path);
    return;
  }
  if (entry.mode === '160000') {
    addFinding(findings, 'GIT_SUBMODULE', entry.path);
    return;
  }

  let cachedRuleIds = contentCache.get(entry.oid);
  if (!cachedRuleIds) {
    const sizeText = git(root, ['cat-file', '-s', entry.oid], {
      encoding: 'utf8',
      maxBuffer: 1024 * 1024,
    }).trim();
    const size = Number(sizeText);
    if (!Number.isSafeInteger(size) || size < 0) {
      addFinding(findings, 'GIT_OBJECT_INVALID_SIZE', entry.path);
      return;
    }
    if (size > MAX_REPOSITORY_FILE_BYTES) {
      addFinding(findings, 'FILE_TOO_LARGE', entry.path);
      return;
    }

    const buffer = git(root, ['cat-file', 'blob', entry.oid], {
      maxBuffer: MAX_REPOSITORY_FILE_BYTES + 1024 * 1024,
    });
    cachedRuleIds = scanBufferContent(buffer);
    contentCache.set(entry.oid, cachedRuleIds);
  }

  for (const ruleId of cachedRuleIds) {
    addFinding(findings, ruleId, entry.path);
  }
}

function scanTracked(root, findings, selectedPaths = null) {
  const entries = parseIndexEntries(git(root, ['ls-files', '--stage', '-z']));
  const contentCache = new Map();

  for (const entry of entries) {
    if (selectedPaths && !selectedPaths.has(entry.path)) continue;
    scanIndexBlob(root, entry, findings, contentCache);
  }
}

function stagedPaths(root) {
  const output = git(root, ['diff', '--cached', '--name-only', '--diff-filter=ACMRT', '-z']);
  return new Set(
    output.toString('utf8').split('\u0000').filter(Boolean).map(normalizeRelativePath),
  );
}

async function scanBuildDirectories(root, findings) {
  for (const directory of BUILD_DIRECTORIES) {
    const absolute = path.join(root, directory);
    let stats;
    try {
      stats = await lstat(absolute);
    } catch (error) {
      if (error.code === 'ENOENT') continue;
      addFinding(findings, 'FILESYSTEM_STAT_ERROR', directory);
      continue;
    }

    if (stats.isSymbolicLink()) {
      addFinding(findings, 'FILESYSTEM_SYMLINK', directory);
      continue;
    }
    if (!stats.isDirectory()) {
      addFinding(findings, 'BUILD_PATH_NOT_DIRECTORY', directory);
      continue;
    }

    await walkDisk(root, absolute, 'build', findings);
  }
}

function parseArguments(argv) {
  let scope = 'all';
  let help = false;

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--help' || argument === '-h') {
      help = true;
      continue;
    }
    if (argument.startsWith('--scope=')) {
      scope = argument.slice('--scope='.length);
      continue;
    }
    if (argument === '--scope' && argv[index + 1]) {
      scope = argv[index + 1];
      index += 1;
      continue;
    }
    if (VALID_SCOPES.has(argument)) {
      scope = argument;
      continue;
    }
    const error = new Error('Unknown argument');
    error.code = 'INVALID_ARGUMENT';
    throw error;
  }

  if (!VALID_SCOPES.has(scope)) {
    const error = new Error('Invalid scope');
    error.code = 'INVALID_SCOPE';
    throw error;
  }

  return { scope, help };
}

function printHelp() {
  process.stdout.write(
    [
      'Usage: node scripts/privacy/audit-repo.mjs [--scope <scope>]',
      '',
      'Scopes: worktree, staged, tracked, build, all',
      '',
    ].join('\n'),
  );
}

function printFindings(scope, findings) {
  const ordered = [...findings.values()].sort(
    (left, right) =>
      left.ruleId.localeCompare(right.ruleId) ||
      left.targetDigest.localeCompare(right.targetDigest),
  );

  if (ordered.length === 0) {
    process.stdout.write(`PRIVACY_AUDIT_PASS ${scope}\n`);
    return 0;
  }

  for (const finding of ordered) {
    process.stdout.write(`${finding.ruleId} path:${finding.targetDigest}\n`);
  }
  process.stdout.write(`PRIVACY_AUDIT_FAIL ${scope} ${ordered.length}\n`);
  return 1;
}

export async function runRepositoryAudit({ scope = 'all', cwd = process.cwd() } = {}) {
  const root = findRepositoryRoot(cwd);
  const findings = new Map();

  if (scope === 'worktree' || scope === 'all') {
    await walkDisk(root, root, 'source', findings, {
      skipDependencies: true,
      skipGenerated: true,
    });
  }
  if (scope === 'staged' || scope === 'all') {
    scanTracked(root, findings, stagedPaths(root));
  }
  if (scope === 'tracked' || scope === 'all') {
    scanTracked(root, findings);
  }
  if (scope === 'build' || scope === 'all') {
    await scanBuildDirectories(root, findings);
  }

  return { scope, findings };
}

async function main() {
  try {
    const { scope, help } = parseArguments(process.argv.slice(2));
    if (help) {
      printHelp();
      return;
    }
    const result = await runRepositoryAudit({ scope });
    process.exitCode = printFindings(result.scope, result.findings);
  } catch (error) {
    const code = typeof error?.code === 'string' ? error.code : 'AUDIT_INTERNAL_ERROR';
    process.stderr.write(`PRIVACY_AUDIT_ERROR ${code}\n`);
    process.exitCode = 2;
  }
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : '';
if (import.meta.url === invokedPath) {
  await main();
}
