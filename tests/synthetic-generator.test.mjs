import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  generateSyntheticExport,
  parseCliArguments,
  SyntheticExportError,
} from '../scripts/synthetic/generate-export.mjs';

const TEST_DIRECTORY = path.dirname(fileURLToPath(import.meta.url));
const REPOSITORY_DIRECTORY = path.resolve(TEST_DIRECTORY, '..');

async function createTemporaryOutput(t, label) {
  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), 'chatgpt-history-browser-synthetic-'),
  );
  t.after(async () => {
    await rm(temporaryRoot, { recursive: true, force: true });
  });
  return path.join(temporaryRoot, label);
}

async function directoryHashes(directory) {
  const fileNames = (await readdir(directory)).sort();
  const hashes = {};

  for (const fileName of fileNames) {
    const bytes = await readFile(path.join(directory, fileName));
    hashes[fileName] = createHash('sha256').update(bytes).digest('hex');
  }

  return hashes;
}

async function readValidConversations(directory, shardFiles) {
  const conversations = [];

  for (const shardFile of shardFiles) {
    const shard = JSON.parse(await readFile(path.join(directory, shardFile), 'utf8'));
    conversations.push(...shard.filter((record) => record?.mapping));
  }

  return conversations;
}

function conversationMessages(conversation) {
  return Object.values(conversation.mapping)
    .map((node) => node.message)
    .filter(Boolean);
}

test('generates deterministic synthetic coverage without source-export data', async (t) => {
  const outputA = await createTemporaryOutput(t, 'fixture-a');
  const outputB = await createTemporaryOutput(t, 'fixture-b');
  const options = {
    count: 12,
    shardSize: 5,
  };

  const summaryA = await generateSyntheticExport({
    outputDirectory: outputA,
    ...options,
  });
  const summaryB = await generateSyntheticExport({
    outputDirectory: outputB,
    ...options,
  });

  assert.equal(summaryA.conversationCount, 12);
  assert.equal(summaryA.shardCount, 3);
  assert.equal(summaryA.attachmentFileCount, 8);
  assert.equal(summaryA.emptyObjectRecordCount, 1);
  assert.deepEqual(summaryA, summaryB);
  assert.deepEqual(await directoryHashes(outputA), await directoryHashes(outputB));

  const conversations = await readValidConversations(outputA, summaryA.shardFiles);
  assert.equal(conversations.length, 12);
  assert.equal(new Set(conversations.map((item) => item.id)).size, 12);
  assert.ok(conversations.every((item) => item.synthetic_fixture === true));
  assert.ok(conversations.every((item) => item.create_time > 0 && item.create_time < 10_000));

  const contentCase = conversations[0];
  const contentMessages = conversationMessages(contentCase);
  const roles = [...new Set(contentMessages.map((message) => message.author.role))].sort();
  assert.deepEqual(roles, ['assistant', 'other', 'system', 'tool', 'user']);

  const attachmentMessage = contentMessages.find((message) =>
    Array.isArray(message.metadata.attachments),
  );
  assert.ok(attachmentMessage);
  assert.equal(attachmentMessage.metadata.attachments.length, 9);
  assert.ok(
    attachmentMessage.metadata.attachments.some(
      (item) =>
        item.id === 'synthetic-missing' &&
        item.synthetic_case === 'metadata-without-a-local-file',
    ),
  );
  assert.ok(
    attachmentMessage.metadata.attachments.some(
      (item) =>
        item.id === 'synthetic-traversal' &&
        item.name.startsWith('../') &&
        item.path.startsWith('..\\'),
    ),
  );
  assert.ok(
    attachmentMessage.metadata.attachments.some(
      (item) =>
        item.id === 'synthetic-misleading' &&
        item.mime_type === 'image/jpeg' &&
        item.name.endsWith('.jpg'),
    ),
  );

  const contentSerialization = JSON.stringify(contentCase);
  assert.match(contentSerialization, /```js/);
  assert.match(contentSerialization, /<script>/);
  assert.match(contentSerialization, /https:\/\/fixture\.example\.invalid\//);
  assert.match(contentSerialization, /javascript:/);

  const branchCase = conversations[1];
  const branchPoint = Object.values(branchCase.mapping).find(
    (node) => node.children.length === 2,
  );
  assert.ok(branchPoint);
  assert.ok(branchCase.mapping[branchCase.current_node]);
  assert.equal(branchCase.mapping[branchCase.current_node].parent.endsWith('branch-b'), true);

  const missingAndDeletedCase = conversations[2];
  const deletedNode = Object.values(missingAndDeletedCase.mapping).find((node) =>
    node.id.endsWith('node-deleted'),
  );
  assert.ok(deletedNode);
  assert.equal(deletedNode.message, null);
  const absentChildReference = Object.values(missingAndDeletedCase.mapping).flatMap((node) =>
    node.children.filter((childId) => missingAndDeletedCase.mapping[childId] === undefined),
  );
  assert.equal(absentChildReference.length, 1);

  assert.equal(conversations[3].is_archived, true);
  assert.equal(conversations[4].is_starred, true);
  assert.equal(conversations[5].title, null);

  const png = await readFile(path.join(outputA, 'file-synthetic-image.dat'));
  assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');

  const wav = await readFile(path.join(outputA, 'file_synthetic-audio.dat'));
  assert.equal(wav.subarray(0, 4).toString('ascii'), 'RIFF');
  assert.equal(wav.subarray(8, 12).toString('ascii'), 'WAVE');

  const mp4 = await readFile(path.join(outputA, 'file-synthetic-video.dat'));
  assert.equal(mp4.subarray(4, 8).toString('ascii'), 'ftyp');

  const pdf = await readFile(path.join(outputA, 'file-synthetic-pdf.dat'));
  assert.equal(pdf.subarray(0, 5).toString('ascii'), '%PDF-');

  const misleading = await readFile(path.join(outputA, 'file-synthetic-misleading.dat'));
  assert.equal(misleading.subarray(0, 5).toString('ascii'), '%PDF-');

  await assert.rejects(
    access(path.join(outputA, 'file-synthetic-missing.dat')),
    (error) => error?.code === 'ENOENT',
  );

  const malformed = await readFile(path.join(outputA, summaryA.malformedShardFile), 'utf8');
  assert.throws(() => JSON.parse(malformed), SyntaxError);

  const manifest = JSON.parse(
    await readFile(path.join(outputA, summaryA.manifestFile), 'utf8'),
  );
  assert.equal(manifest.synthetic_only, true);
  assert.equal(manifest.conversation_count, 12);
  assert.equal(manifest.intentionally_malformed_shard, summaryA.malformedShardFile);
  assert.equal(manifest.missing_attachment_file, 'file-synthetic-missing.dat');
  assert.equal(manifest.empty_object_record_count, 1);
});

test('refuses repository paths and non-empty destinations', async (t) => {
  const forbiddenOutput = path.join(
    REPOSITORY_DIRECTORY,
    '.synthetic-generator-must-not-write',
  );

  await assert.rejects(
    generateSyntheticExport({ outputDirectory: forbiddenOutput }),
    (error) =>
      error instanceof SyntheticExportError && error.message.includes('outside the repository'),
  );
  await assert.rejects(access(forbiddenOutput), (error) => error?.code === 'ENOENT');

  const nonEmptyOutput = await createTemporaryOutput(t, 'non-empty');
  await mkdir(nonEmptyOutput, { recursive: true });
  await writeFile(
    path.join(nonEmptyOutput, 'SYNTHETIC-EXISTING-MARKER.txt'),
    'SYNTHETIC EXISTING MARKER\n',
    'utf8',
  );

  await assert.rejects(
    generateSyntheticExport({ outputDirectory: nonEmptyOutput }),
    (error) => error instanceof SyntheticExportError && error.message.includes('must be empty'),
  );
});

test('large-conversation mode creates a long deterministic mapping chain', async (t) => {
  const output = await createTemporaryOutput(t, 'large-fixture');
  const summary = await generateSyntheticExport({
    outputDirectory: output,
    count: 8,
    shardSize: 3,
    includeLargeConversation: true,
    largeMessageCount: 64,
  });
  const conversations = await readValidConversations(output, summary.shardFiles);
  const largeConversation = conversations.at(-1);

  assert.equal(largeConversation.title, 'SYNTHETIC LARGE CONVERSATION');
  assert.equal(Object.keys(largeConversation.mapping).length, 65);
  assert.ok(largeConversation.mapping[largeConversation.current_node]);
  assert.equal(
    conversationMessages(largeConversation).filter((message) => message.author.role === 'user')
      .length,
    32,
  );
  assert.equal(
    conversationMessages(largeConversation).filter(
      (message) => message.author.role === 'assistant',
    ).length,
    32,
  );
});

test('--count 10000 selects lightweight sharding and produces exactly 10000 records', async (t) => {
  const output = await createTemporaryOutput(t, 'ten-thousand-fixture');
  const parsed = parseCliArguments(['--output', output, '--count', '10000']);
  assert.equal(parsed.count, 10_000);
  assert.equal(parsed.includeLargeConversation, undefined);

  const summary = await generateSyntheticExport(parsed);
  assert.equal(summary.conversationCount, 10_000);
  assert.equal(summary.shardCount, 10);

  let countedConversations = 0;
  let countedEmptyObjects = 0;
  let finalConversation;
  for (const shardFile of summary.shardFiles) {
    const shard = JSON.parse(await readFile(path.join(output, shardFile), 'utf8'));
    const conversations = shard.filter((record) => record?.mapping);
    countedConversations += conversations.length;
    countedEmptyObjects += shard.filter(
      (record) =>
        record &&
        typeof record === 'object' &&
        !Array.isArray(record) &&
        Object.keys(record).length === 0,
    ).length;
    finalConversation = conversations.at(-1) ?? finalConversation;
  }

  assert.equal(countedConversations, 10_000);
  assert.equal(countedEmptyObjects, 1);
  assert.equal(finalConversation.id, 'synthetic-conversation-010000');
  assert.equal(Object.keys(finalConversation.mapping).length, 3);

  const manifest = JSON.parse(await readFile(path.join(output, summary.manifestFile), 'utf8'));
  assert.equal(manifest.includes_large_conversation, false);
  assert.equal(manifest.large_message_count, 0);
});
