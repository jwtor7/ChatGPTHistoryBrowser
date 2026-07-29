#!/usr/bin/env node

import { Buffer } from 'node:buffer';
import { mkdir, readdir, realpath, writeFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { deflateSync } from 'node:zlib';

const SCRIPT_DIRECTORY = path.dirname(fileURLToPath(import.meta.url));
const REPOSITORY_DIRECTORY = path.resolve(SCRIPT_DIRECTORY, '../..');

const DEFAULT_CONVERSATION_COUNT = 12;
const DEFAULT_SMALL_SHARD_SIZE = 5;
const DEFAULT_LARGE_SHARD_SIZE = 1_000;
const DEFAULT_LARGE_MESSAGE_COUNT = 2_500;
const MAX_CONVERSATION_COUNT = 1_000_000;
const MAX_SHARD_SIZE = 10_000;
const MAX_LARGE_MESSAGE_COUNT = 100_000;

export class SyntheticExportError extends Error {
  constructor(message) {
    super(message);
    this.name = 'SyntheticExportError';
  }
}

function assertPositiveInteger(value, label, maximum) {
  if (!Number.isSafeInteger(value) || value < 1 || value > maximum) {
    throw new SyntheticExportError(
      `${label} must be a positive integer no greater than ${maximum}.`,
    );
  }
}

function isSameOrInside(candidatePath, parentPath) {
  const relative = path.relative(parentPath, candidatePath);
  return (
    relative === '' ||
    (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative))
  );
}

async function canonicalizeProspectivePath(candidatePath) {
  let cursor = path.resolve(candidatePath);
  const missingSegments = [];

  while (true) {
    try {
      const canonicalParent = await realpath(cursor);
      return path.resolve(canonicalParent, ...missingSegments.reverse());
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }

      const parent = path.dirname(cursor);
      if (parent === cursor) {
        throw new SyntheticExportError('The output directory could not be resolved safely.');
      }

      missingSegments.push(path.basename(cursor));
      cursor = parent;
    }
  }
}

async function prepareOutputDirectory(outputDirectory) {
  if (typeof outputDirectory !== 'string' || outputDirectory.trim() === '') {
    throw new SyntheticExportError('A caller-provided output directory is required.');
  }

  const canonicalRepository = await realpath(REPOSITORY_DIRECTORY);
  const prospectiveOutput = await canonicalizeProspectivePath(outputDirectory);

  if (isSameOrInside(prospectiveOutput, canonicalRepository)) {
    throw new SyntheticExportError(
      'The synthetic export output directory must be outside the repository.',
    );
  }

  if (prospectiveOutput === path.parse(prospectiveOutput).root) {
    throw new SyntheticExportError(
      'The filesystem root cannot be used as the synthetic export directory.',
    );
  }

  await mkdir(prospectiveOutput, { recursive: true });
  const canonicalOutput = await realpath(prospectiveOutput);

  if (isSameOrInside(canonicalOutput, canonicalRepository)) {
    throw new SyntheticExportError(
      'The synthetic export output directory must be outside the repository.',
    );
  }

  const existingEntries = await readdir(canonicalOutput);
  if (existingEntries.length > 0) {
    throw new SyntheticExportError('The synthetic export output directory must be empty.');
  }

  return canonicalOutput;
}

function makePngChunk(type, data) {
  const typeBytes = Buffer.from(type, 'ascii');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);

  const checksumInput = Buffer.concat([typeBytes, data]);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(checksumInput), 0);

  return Buffer.concat([length, typeBytes, data, checksum]);
}

function crc32(bytes) {
  let checksum = 0xffffffff;

  for (const byte of bytes) {
    checksum ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      checksum = checksum & 1 ? (checksum >>> 1) ^ 0xedb88320 : checksum >>> 1;
    }
  }

  return (checksum ^ 0xffffffff) >>> 0;
}

function createSyntheticPng() {
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(1, 0);
  header.writeUInt32BE(1, 4);
  header[8] = 8;
  header[9] = 6;
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  const scanline = Buffer.from([0, 0x24, 0x68, 0xac, 0xff]);
  const compressed = deflateSync(scanline, { level: 9 });

  return Buffer.concat([
    signature,
    makePngChunk('IHDR', header),
    makePngChunk('IDAT', compressed),
    makePngChunk('IEND', Buffer.alloc(0)),
  ]);
}

function createSyntheticWav() {
  const sampleRate = 8_000;
  const samples = Buffer.alloc(64);

  for (let index = 0; index < samples.length; index += 1) {
    samples[index] = 96 + ((index * 17) % 64);
  }

  const header = Buffer.alloc(44);
  header.write('RIFF', 0, 'ascii');
  header.writeUInt32LE(36 + samples.length, 4);
  header.write('WAVE', 8, 'ascii');
  header.write('fmt ', 12, 'ascii');
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate, 28);
  header.writeUInt16LE(1, 32);
  header.writeUInt16LE(8, 34);
  header.write('data', 36, 'ascii');
  header.writeUInt32LE(samples.length, 40);

  return Buffer.concat([header, samples]);
}

function createMp4Box(type, payload) {
  const box = Buffer.alloc(8 + payload.length);
  box.writeUInt32BE(box.length, 0);
  box.write(type, 4, 'ascii');
  payload.copy(box, 8);
  return box;
}

function createSyntheticMp4() {
  const fileType = Buffer.alloc(24);
  fileType.write('isom', 0, 'ascii');
  fileType.writeUInt32BE(0x200, 4);
  fileType.write('isom', 8, 'ascii');
  fileType.write('iso2', 12, 'ascii');
  fileType.write('mp41', 16, 'ascii');
  fileType.write('avc1', 20, 'ascii');

  const marker = Buffer.from('SYNTHETIC VIDEO FIXTURE', 'ascii');
  const mediaData = Buffer.alloc(96);
  for (let index = 0; index < mediaData.length; index += 1) {
    mediaData[index] = (index * 29 + 7) % 256;
  }

  return Buffer.concat([
    createMp4Box('ftyp', fileType),
    createMp4Box('free', marker),
    createMp4Box('mdat', mediaData),
  ]);
}

function createSyntheticPdf() {
  const stream = 'BT\n/F1 18 Tf\n36 72 Td\n(SYNTHETIC PDF FIXTURE) Tj\nET\n';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 120] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${Buffer.byteLength(stream, 'ascii')} >>\nstream\n${stream}endstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];

  let document = '%PDF-1.4\n% SYNTHETIC FIXTURE\n';
  const offsets = [0];

  for (let index = 0; index < objects.length; index += 1) {
    offsets.push(Buffer.byteLength(document, 'ascii'));
    document += `${index + 1} 0 obj\n${objects[index]}\nendobj\n`;
  }

  const crossReferenceOffset = Buffer.byteLength(document, 'ascii');
  document += `xref\n0 ${objects.length + 1}\n`;
  document += '0000000000 65535 f \n';
  for (const offset of offsets.slice(1)) {
    document += `${String(offset).padStart(10, '0')} 00000 n \n`;
  }
  document += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\n`;
  document += `startxref\n${crossReferenceOffset}\n%%EOF\n`;

  return Buffer.from(document, 'ascii');
}

function createUnsupportedBytes() {
  const marker = Buffer.from('SYNTHETIC UNSUPPORTED FIXTURE\n', 'ascii');
  const payload = Buffer.alloc(96);

  for (let index = 0; index < payload.length; index += 1) {
    payload[index] = (index * 37 + 11) % 256;
  }

  return Buffer.concat([marker, payload]);
}

function createAttachmentFixtures() {
  const png = createSyntheticPng();
  const wav = createSyntheticWav();
  const pdf = createSyntheticPdf();

  return [
    {
      id: 'synthetic-image',
      fileName: 'file-synthetic-image.dat',
      originalName: 'SYNTHETIC-IMAGE.png',
      mimeType: 'image/png',
      bytes: png,
    },
    {
      id: 'synthetic-audio',
      fileName: 'file_synthetic-audio.dat',
      originalName: 'SYNTHETIC-AUDIO.wav',
      mimeType: 'audio/wav',
      bytes: wav,
    },
    {
      id: 'synthetic-video',
      fileName: 'file-synthetic-video.dat',
      originalName: 'SYNTHETIC-VIDEO.mp4',
      mimeType: 'video/mp4',
      bytes: createSyntheticMp4(),
    },
    {
      id: 'synthetic-pdf',
      fileName: 'file-synthetic-pdf.dat',
      originalName: 'SYNTHETIC-DOCUMENT.pdf',
      mimeType: 'application/pdf',
      bytes: pdf,
    },
    {
      id: 'synthetic-text',
      fileName: 'file-synthetic-text.dat',
      originalName: 'SYNTHETIC-NOTES.txt',
      mimeType: 'text/plain',
      bytes: Buffer.from(
        'SYNTHETIC TEXT FIXTURE\nNo real export content is present.\n',
        'utf8',
      ),
    },
    {
      id: 'synthetic-unsupported',
      fileName: 'file-synthetic-unsupported.dat',
      originalName: 'SYNTHETIC-PAYLOAD.fixturebin',
      mimeType: 'application/x-synthetic-unsupported',
      bytes: createUnsupportedBytes(),
    },
    {
      id: 'synthetic-misleading',
      fileName: 'file-synthetic-misleading.dat',
      originalName: 'SYNTHETIC-MISLEADING.jpg',
      mimeType: 'image/jpeg',
      bytes: pdf,
      syntheticCase: 'declared-jpeg-with-pdf-signature',
    },
    {
      id: 'synthetic-traversal',
      fileName: 'file-synthetic-traversal.dat',
      originalName: '../../SYNTHETIC-TRAVERSAL.txt',
      mimeType: 'text/plain',
      bytes: Buffer.from('SYNTHETIC PATH-TRAVERSAL FIXTURE\n', 'utf8'),
      metadataPath: '..\\..\\SYNTHETIC-TRAVERSAL.txt',
    },
    {
      id: 'synthetic-missing',
      fileName: 'file-synthetic-missing.dat',
      originalName: 'SYNTHETIC-MISSING.png',
      mimeType: 'image/png',
      bytes: null,
      declaredSize: 128,
      syntheticCase: 'metadata-without-a-local-file',
    },
  ];
}

function toAttachmentMetadata(fixture) {
  const metadata = {
    id: fixture.id,
    name: fixture.originalName,
    mime_type: fixture.mimeType,
    size: fixture.declaredSize ?? fixture.bytes?.length ?? 0,
    file_name: fixture.fileName,
    synthetic_fixture: true,
  };

  if (fixture.syntheticCase) {
    metadata.synthetic_case = fixture.syntheticCase;
  }
  if (fixture.metadataPath) {
    metadata.path = fixture.metadataPath;
  }

  return metadata;
}

function conversationId(index) {
  return `synthetic-conversation-${String(index + 1).padStart(6, '0')}`;
}

function fixtureTime(index, ordinal = 0) {
  return (index + 1) * 100 + ordinal / 100;
}

function createMessage({
  conversationIndex,
  label,
  ordinal,
  role,
  parts,
  metadata = {},
  contentType = 'text',
}) {
  const id = `${conversationId(conversationIndex)}-message-${label}`;

  return {
    id,
    author: {
      role,
      name: role === 'tool' ? 'synthetic-tool' : null,
      metadata: {},
    },
    create_time: fixtureTime(conversationIndex, ordinal),
    update_time: null,
    content: {
      content_type: contentType,
      parts,
    },
    status: 'finished_successfully',
    end_turn: role === 'assistant',
    weight: 1,
    metadata: {
      synthetic_fixture: true,
      ...metadata,
    },
    recipient: 'all',
    channel: null,
  };
}

function createNode(id, message, parent, children = []) {
  return {
    id,
    message,
    parent,
    children,
  };
}

function createConversationRecord({
  index,
  title = `SYNTHETIC FIXTURE ${String(index + 1).padStart(6, '0')}`,
  mapping,
  currentNode,
  archived = false,
  starred = false,
}) {
  const id = conversationId(index);

  return {
    title,
    create_time: fixtureTime(index),
    update_time: fixtureTime(index, 99),
    mapping,
    moderation_results: [],
    current_node: currentNode,
    plugin_ids: null,
    conversation_id: id,
    conversation_template_id: null,
    gizmo_id: null,
    id,
    is_archived: archived,
    is_starred: starred,
    synthetic_fixture: true,
  };
}

function createContentAndAttachmentConversation(index, attachments) {
  const id = conversationId(index);
  const root = `${id}-node-root`;
  const system = `${id}-node-system`;
  const user = `${id}-node-user`;
  const assistant = `${id}-node-assistant`;
  const tool = `${id}-node-tool`;
  const other = `${id}-node-other`;
  const attachmentMetadata = attachments.map(toAttachmentMetadata);

  const mapping = {
    [root]: createNode(root, null, null, [system]),
    [system]: createNode(
      system,
      createMessage({
        conversationIndex: index,
        label: 'system',
        ordinal: 1,
        role: 'system',
        parts: ['SYNTHETIC SYSTEM MESSAGE FOR FIXTURE TESTING.'],
      }),
      root,
      [user],
    ),
    [user]: createNode(
      user,
      createMessage({
        conversationIndex: index,
        label: 'user',
        ordinal: 2,
        role: 'user',
        contentType: 'multimodal_text',
        parts: [
          'SYNTHETIC USER MESSAGE WITH ATTACHMENT METADATA.',
          {
            content_type: 'image_asset_pointer',
            asset_pointer: 'file-service://synthetic-image',
            size_bytes:
              attachments.find((item) => item.id === 'synthetic-image')?.bytes?.length ?? 0,
            width: 1,
            height: 1,
          },
        ],
        metadata: {
          attachments: attachmentMetadata,
        },
      }),
      system,
      [assistant],
    ),
    [assistant]: createNode(
      assistant,
      createMessage({
        conversationIndex: index,
        label: 'assistant',
        ordinal: 3,
        role: 'assistant',
        parts: [
          [
            '# SYNTHETIC MARKDOWN',
            '',
            '```js',
            'const syntheticValue = "SYNTHETIC CODE BLOCK";',
            '```',
            '',
            '<script>globalThis.syntheticExecutionMarker = true;</script>',
            '<img src="https://fixture.example.invalid/remote.png" onerror="globalThis.syntheticImageMarker=true">',
            '<a href="javascript:globalThis.syntheticLinkMarker=true">SYNTHETIC UNSAFE LINK</a>',
          ].join('\n'),
        ],
      }),
      user,
      [tool],
    ),
    [tool]: createNode(
      tool,
      createMessage({
        conversationIndex: index,
        label: 'tool',
        ordinal: 4,
        role: 'tool',
        parts: ['{"synthetic_fixture":true,"result":"SYNTHETIC TOOL OUTPUT"}'],
      }),
      assistant,
      [other],
    ),
    [other]: createNode(
      other,
      createMessage({
        conversationIndex: index,
        label: 'other',
        ordinal: 5,
        role: 'other',
        parts: ['SYNTHETIC OTHER-ROLE MESSAGE.'],
      }),
      tool,
      [],
    ),
  };

  return createConversationRecord({
    index,
    title: 'SYNTHETIC CONTENT AND ATTACHMENT CASES',
    mapping,
    currentNode: other,
  });
}

function createBranchedConversation(index) {
  const id = conversationId(index);
  const root = `${id}-node-root`;
  const prompt = `${id}-node-prompt`;
  const branchA = `${id}-node-branch-a`;
  const branchB = `${id}-node-branch-b`;
  const branchAFollowup = `${id}-node-branch-a-followup`;
  const branchBFollowup = `${id}-node-branch-b-followup`;

  const mapping = {
    [root]: createNode(root, null, null, [prompt]),
    [prompt]: createNode(
      prompt,
      createMessage({
        conversationIndex: index,
        label: 'branch-prompt',
        ordinal: 1,
        role: 'user',
        parts: ['SYNTHETIC BRANCH PROMPT.'],
      }),
      root,
      [branchA, branchB],
    ),
    [branchA]: createNode(
      branchA,
      createMessage({
        conversationIndex: index,
        label: 'branch-a',
        ordinal: 2,
        role: 'assistant',
        parts: ['SYNTHETIC ALTERNATE BRANCH A.'],
      }),
      prompt,
      [branchAFollowup],
    ),
    [branchAFollowup]: createNode(
      branchAFollowup,
      createMessage({
        conversationIndex: index,
        label: 'branch-a-followup',
        ordinal: 3,
        role: 'user',
        parts: ['SYNTHETIC FOLLOW-UP ON BRANCH A.'],
      }),
      branchA,
      [],
    ),
    [branchB]: createNode(
      branchB,
      createMessage({
        conversationIndex: index,
        label: 'branch-b',
        ordinal: 4,
        role: 'assistant',
        parts: ['SYNTHETIC ACTIVE BRANCH B.'],
      }),
      prompt,
      [branchBFollowup],
    ),
    [branchBFollowup]: createNode(
      branchBFollowup,
      createMessage({
        conversationIndex: index,
        label: 'branch-b-followup',
        ordinal: 5,
        role: 'user',
        parts: ['SYNTHETIC FOLLOW-UP ON ACTIVE BRANCH B.'],
      }),
      branchB,
      [],
    ),
  };

  return createConversationRecord({
    index,
    title: 'SYNTHETIC BRANCHED CONVERSATION',
    mapping,
    currentNode: branchBFollowup,
  });
}

function createMissingAndDeletedConversation(index) {
  const id = conversationId(index);
  const root = `${id}-node-root`;
  const deleted = `${id}-node-deleted`;
  const surviving = `${id}-node-surviving`;
  const missing = `${id}-node-missing-reference`;

  const mapping = {
    [root]: createNode(root, null, null, [deleted]),
    [deleted]: createNode(deleted, null, root, [surviving]),
    [surviving]: createNode(
      surviving,
      createMessage({
        conversationIndex: index,
        label: 'surviving',
        ordinal: 1,
        role: 'assistant',
        parts: ['SYNTHETIC MESSAGE AFTER A DELETED NODE.'],
      }),
      deleted,
      [missing],
    ),
  };

  return createConversationRecord({
    index,
    title: 'SYNTHETIC DELETED AND MISSING NODE CASES',
    mapping,
    currentNode: surviving,
  });
}

function createLightweightConversation(
  index,
  { title, archived = false, starred = false } = {},
) {
  const id = conversationId(index);
  const root = `${id}-node-root`;
  const user = `${id}-node-user`;
  const assistant = `${id}-node-assistant`;
  const mapping = {
    [root]: createNode(root, null, null, [user]),
    [user]: createNode(
      user,
      createMessage({
        conversationIndex: index,
        label: 'user',
        ordinal: 1,
        role: 'user',
        parts: [`SYNTHETIC USER MESSAGE ${String(index + 1).padStart(6, '0')}.`],
      }),
      root,
      [assistant],
    ),
    [assistant]: createNode(
      assistant,
      createMessage({
        conversationIndex: index,
        label: 'assistant',
        ordinal: 2,
        role: 'assistant',
        parts: [`SYNTHETIC ASSISTANT MESSAGE ${String(index + 1).padStart(6, '0')}.`],
      }),
      user,
      [],
    ),
  };

  return createConversationRecord({
    index,
    title,
    mapping,
    currentNode: assistant,
    archived,
    starred,
  });
}

function createLargeConversation(index, messageCount) {
  const id = conversationId(index);
  const root = `${id}-node-root`;
  const mapping = {
    [root]: createNode(root, null, null, []),
  };
  let parent = root;

  for (let messageIndex = 0; messageIndex < messageCount; messageIndex += 1) {
    const label = `large-${String(messageIndex + 1).padStart(6, '0')}`;
    const nodeId = `${id}-node-${label}`;
    const role = messageIndex % 2 === 0 ? 'user' : 'assistant';
    mapping[parent].children.push(nodeId);
    mapping[nodeId] = createNode(
      nodeId,
      createMessage({
        conversationIndex: index,
        label,
        ordinal: messageIndex + 1,
        role,
        parts: [
          `SYNTHETIC LARGE MESSAGE ${String(messageIndex + 1).padStart(6, '0')}.\n${'SYNTHETIC-LARGE-FILLER '.repeat(48)}`,
        ],
      }),
      parent,
      [],
    );
    parent = nodeId;
  }

  return createConversationRecord({
    index,
    title: 'SYNTHETIC LARGE CONVERSATION',
    mapping,
    currentNode: parent,
  });
}

function createConversation(index, options, attachments) {
  if (options.includeLargeConversation && index === options.count - 1) {
    return createLargeConversation(index, options.largeMessageCount);
  }

  switch (index) {
    case 0:
      return createContentAndAttachmentConversation(index, attachments);
    case 1:
      return createBranchedConversation(index);
    case 2:
      return createMissingAndDeletedConversation(index);
    case 3:
      return createLightweightConversation(index, {
        title: 'SYNTHETIC ARCHIVED CONVERSATION',
        archived: true,
      });
    case 4:
      return createLightweightConversation(index, {
        title: 'SYNTHETIC STARRED CONVERSATION',
        starred: true,
      });
    case 5:
      return createLightweightConversation(index, { title: null });
    default:
      return createLightweightConversation(index);
  }
}

function normalizeOptions(options) {
  if (options === null || typeof options !== 'object') {
    throw new SyntheticExportError('Generator options are required.');
  }

  const count = options.count ?? DEFAULT_CONVERSATION_COUNT;
  assertPositiveInteger(count, 'Conversation count', MAX_CONVERSATION_COUNT);

  const shardSize =
    options.shardSize ??
    (count >= 10_000 ? DEFAULT_LARGE_SHARD_SIZE : DEFAULT_SMALL_SHARD_SIZE);
  assertPositiveInteger(shardSize, 'Shard size', MAX_SHARD_SIZE);

  const includeLargeConversation = options.includeLargeConversation === true;
  const largeMessageCount = options.largeMessageCount ?? DEFAULT_LARGE_MESSAGE_COUNT;
  assertPositiveInteger(largeMessageCount, 'Large-message count', MAX_LARGE_MESSAGE_COUNT);

  return {
    outputDirectory: options.outputDirectory,
    count,
    shardSize,
    includeLargeConversation,
    largeMessageCount,
  };
}

async function writeExclusive(directory, fileName, contents) {
  await writeFile(path.join(directory, fileName), contents, {
    flag: 'wx',
  });
}

async function writeJson(directory, fileName, value) {
  await writeExclusive(directory, fileName, `${JSON.stringify(value, null, 2)}\n`);
}

export async function generateSyntheticExport(options) {
  const normalized = normalizeOptions(options);
  const outputDirectory = await prepareOutputDirectory(normalized.outputDirectory);
  const attachments = createAttachmentFixtures();
  const writtenAttachmentFiles = [];

  for (const attachment of attachments) {
    if (attachment.bytes === null) {
      continue;
    }
    await writeExclusive(outputDirectory, attachment.fileName, attachment.bytes);
    writtenAttachmentFiles.push(attachment.fileName);
  }

  const shardFiles = [];
  let shardIndex = 0;

  for (
    let firstConversation = 0;
    firstConversation < normalized.count;
    firstConversation += normalized.shardSize
  ) {
    const conversations = [];
    const lastConversation = Math.min(
      firstConversation + normalized.shardSize,
      normalized.count,
    );

    for (
      let conversationIndex = firstConversation;
      conversationIndex < lastConversation;
      conversationIndex += 1
    ) {
      conversations.push(createConversation(conversationIndex, normalized, attachments));
    }
    if (shardIndex === 0) {
      // Current official exports may include empty object records between
      // substantive conversations. This independently authored fixture keeps
      // the ingestion skip path covered without copying any export content.
      conversations.push({});
    }

    const shardFile = `conversations-${String(shardIndex).padStart(3, '0')}.json`;
    await writeJson(outputDirectory, shardFile, conversations);
    shardFiles.push(shardFile);
    shardIndex += 1;
  }

  const malformedShardFile = 'conversations-malformed.json';
  await writeExclusive(
    outputDirectory,
    malformedShardFile,
    '[\n  {"synthetic_fixture": true, "case": "INTENTIONALLY MALFORMED JSON"\n',
  );

  const manifest = {
    fixture_format: 'chatgpt-history-browser-synthetic-v1',
    synthetic_only: true,
    conversation_count: normalized.count,
    shard_count: shardFiles.length,
    shard_size: normalized.shardSize,
    includes_large_conversation: normalized.includeLargeConversation,
    large_message_count: normalized.includeLargeConversation ? normalized.largeMessageCount : 0,
    shard_files: shardFiles,
    intentionally_malformed_shard: malformedShardFile,
    attachment_files: writtenAttachmentFiles,
    missing_attachment_file: 'file-synthetic-missing.dat',
    empty_object_record_count: 1,
  };
  const manifestFile = 'synthetic-export-manifest.json';
  await writeJson(outputDirectory, manifestFile, manifest);

  return {
    conversationCount: normalized.count,
    shardCount: shardFiles.length,
    attachmentFileCount: writtenAttachmentFiles.length,
    emptyObjectRecordCount: 1,
    shardFiles,
    malformedShardFile,
    manifestFile,
  };
}

function parseIntegerArgument(rawValue, label, maximum) {
  if (!/^[1-9]\d*$/.test(rawValue ?? '')) {
    throw new SyntheticExportError(`${label} must be a positive integer.`);
  }

  const value = Number(rawValue);
  assertPositiveInteger(value, label, maximum);
  return value;
}

export function parseCliArguments(argumentsList) {
  const options = {};

  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];

    switch (argument) {
      case '--help':
      case '-h':
        return { help: true };
      case '--output':
        index += 1;
        if (index >= argumentsList.length) {
          throw new SyntheticExportError('--output requires a directory.');
        }
        options.outputDirectory = argumentsList[index];
        break;
      case '--count':
        index += 1;
        options.count = parseIntegerArgument(
          argumentsList[index],
          'Conversation count',
          MAX_CONVERSATION_COUNT,
        );
        break;
      case '--shard-size':
        index += 1;
        options.shardSize = parseIntegerArgument(
          argumentsList[index],
          'Shard size',
          MAX_SHARD_SIZE,
        );
        break;
      case '--large':
        options.includeLargeConversation = true;
        break;
      case '--large-messages':
        index += 1;
        options.largeMessageCount = parseIntegerArgument(
          argumentsList[index],
          'Large-message count',
          MAX_LARGE_MESSAGE_COUNT,
        );
        break;
      default:
        throw new SyntheticExportError(
          'Unknown generator argument. Use --help for supported options.',
        );
    }
  }

  if (!options.outputDirectory) {
    throw new SyntheticExportError('--output is required.');
  }

  return options;
}

function helpText() {
  return [
    'Generate a deterministic, entirely synthetic ChatGPT export fixture.',
    '',
    'Usage:',
    '  node scripts/synthetic/generate-export.mjs --output <empty-directory> [options]',
    '',
    'Options:',
    `  --count <number>          Conversation count (default: ${DEFAULT_CONVERSATION_COUNT})`,
    '  --shard-size <number>     Conversations per valid JSON shard',
    '  --large                   Make the final conversation a large mapping chain',
    `  --large-messages <number> Messages in --large mode (default: ${DEFAULT_LARGE_MESSAGE_COUNT})`,
    '  --help                    Show this help',
    '',
    'The output directory must be outside the repository and must be empty.',
  ].join('\n');
}

async function runCli() {
  const options = parseCliArguments(process.argv.slice(2));

  if (options.help) {
    process.stdout.write(`${helpText()}\n`);
    return;
  }

  const summary = await generateSyntheticExport(options);
  process.stdout.write(
    `Generated ${summary.conversationCount} synthetic conversations in ${summary.shardCount} valid shards with ${summary.attachmentFileCount} local attachment fixtures.\n`,
  );
}

const isDirectExecution =
  process.argv[1] !== undefined &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url;

if (isDirectExecution) {
  runCli().catch((error) => {
    const message =
      error instanceof SyntheticExportError
        ? error.message
        : 'Synthetic export generation failed because of an unexpected filesystem error.';
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
  });
}
