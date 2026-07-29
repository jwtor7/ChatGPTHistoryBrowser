import {
  AlertTriangle,
  Download,
  File,
  FileAudio,
  FileImage,
  FileText,
  FileVideo,
  LoaderCircle,
} from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';

import type { LocalApi } from './api';
import type { AttachmentView } from './types';

const MAX_MEDIA_PREVIEW_BYTES = 64 * 1024 * 1024;
const MAX_PDF_PREVIEW_BYTES = 20 * 1024 * 1024;
const MAX_PDF_CANVAS_DIMENSION = 4_096;
const MAX_PDF_CANVAS_PIXELS = 16_777_216;

const MIME_ALLOWLIST = {
  image: new Set(['image/jpeg', 'image/png']),
  audio: new Set([
    'audio/aac',
    'audio/flac',
    'audio/m4a',
    'audio/mp4',
    'audio/mpeg',
    'audio/ogg',
    'audio/wav',
    'audio/webm',
    'audio/x-m4a',
    'audio/x-wav',
  ]),
  video: new Set(['video/mp4', 'video/mpeg', 'video/ogg', 'video/quicktime', 'video/webm']),
  pdf: new Set(['application/pdf']),
} as const;

function formatBytes(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return 'Size unavailable';
  if (bytes < 1_024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1_024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1_024; index += 1) {
    value /= 1_024;
    unit = units[index];
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}

function attachmentCategory(
  attachment: AttachmentView,
): 'image' | 'audio' | 'video' | 'pdf' | 'text' | 'unsupported' | 'missing' {
  const mime = attachment.detectedMime?.toLowerCase().split(';', 1)[0] ?? '';
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('audio/')) return 'audio';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('text/')) return 'text';
  if (mime === 'application/pdf') return 'pdf';
  return attachment.previewKind;
}

function formatAttachmentType(attachment: AttachmentView): string {
  const mime = attachment.detectedMime?.toLowerCase().split(';', 1)[0] ?? '';
  const labels: Record<string, string> = {
    'application/pdf': 'PDF document',
    'audio/aac': 'AAC audio',
    'audio/flac': 'FLAC audio',
    'audio/m4a': 'M4A audio',
    'audio/mp4': 'M4A audio',
    'audio/mpeg': 'MP3 audio',
    'audio/ogg': 'OGG audio',
    'audio/wav': 'WAV audio',
    'audio/webm': 'WebM audio',
    'audio/x-m4a': 'M4A audio',
    'audio/x-wav': 'WAV audio',
    'image/jpeg': 'JPEG image',
    'image/png': 'PNG image',
    'text/plain': 'Text file',
    'video/mp4': 'MP4 video',
    'video/ogg': 'OGG video',
    'video/mpeg': 'MPEG video',
    'video/quicktime': 'QuickTime video',
    'video/webm': 'WebM video',
  };
  if (labels[mime]) return labels[mime];
  const category = attachmentCategory(attachment);
  if (category === 'missing') return 'Missing file';
  if (category === 'unsupported') return 'Unknown file type';
  if (category === 'pdf') return 'PDF document';
  return `${category[0].toUpperCase()}${category.slice(1)} file`;
}

function safePreviewMime(
  attachment: AttachmentView,
  kind: 'image' | 'audio' | 'video' | 'pdf',
): string | null {
  const mime = attachment.detectedMime?.toLowerCase().split(';', 1)[0] ?? null;
  return mime && MIME_ALLOWLIST[kind].has(mime) ? mime : null;
}

function mediaPreviewBlockedMessage(
  attachment: AttachmentView,
  kind: 'image' | 'audio' | 'video',
): string | null {
  if (!safePreviewMime(attachment, kind)) {
    return 'Preview blocked because the detected file type is not allowlisted.';
  }
  if (attachment.byteSize !== null && attachment.byteSize > MAX_MEDIA_PREVIEW_BYTES) {
    return 'Preview blocked because this file exceeds the safe preview limit.';
  }
  return null;
}

function pdfPreviewBlockedMessage(attachment: AttachmentView): string | null {
  if (!safePreviewMime(attachment, 'pdf')) {
    return 'PDF preview blocked because the detected type is not allowlisted.';
  }
  if (attachment.byteSize !== null && attachment.byteSize > MAX_PDF_PREVIEW_BYTES) {
    return 'PDF preview is limited to local files under 20 MB.';
  }
  return null;
}

function AttachmentIcon({ attachment }: { attachment: AttachmentView }) {
  const category = attachmentCategory(attachment);
  const props = { size: 18, strokeWidth: 1.8, 'aria-hidden': true } as const;
  if (category === 'image') return <FileImage {...props} />;
  if (category === 'audio') return <FileAudio {...props} />;
  if (category === 'video') return <FileVideo {...props} />;
  if (category === 'text') return <FileText {...props} />;
  return <File {...props} />;
}

interface MediaPreviewProps {
  api: LocalApi;
  attachment: AttachmentView;
  kind: 'image' | 'audio' | 'video';
  onClose: () => void;
}

function MediaPreview({ api, attachment, kind, onClose }: MediaPreviewProps) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mime = safePreviewMime(attachment, kind);
  const blockedMessage = mediaPreviewBlockedMessage(attachment, kind);

  useEffect(() => {
    if (!mime || blockedMessage) return;

    let active = true;
    let localUrl: string | null = null;
    const controller = new AbortController();

    void api
      .attachmentContent(attachment.id, controller.signal)
      .then((blob) => {
        if (blob.size > MAX_MEDIA_PREVIEW_BYTES) {
          throw new Error('preview-limit');
        }
        const safelyTypedBlob = blob.slice(0, blob.size, mime);
        localUrl = URL.createObjectURL(safelyTypedBlob);
        if (active) setObjectUrl(localUrl);
      })
      .catch(() => {
        if (active) {
          setError('This preview could not be loaded safely.');
        }
      });

    return () => {
      active = false;
      controller.abort();
      if (localUrl) URL.revokeObjectURL(localUrl);
    };
  }, [api, attachment.id, blockedMessage, mime]);

  if (blockedMessage) return <p className="attachment-note">{blockedMessage}</p>;
  if (error) {
    return (
      <div className="attachment-inline-action">
        <span className="attachment-note">{error}</span>
        <button type="button" className="button button-quiet button-small" onClick={onClose}>
          Close preview
        </button>
      </div>
    );
  }
  if (!objectUrl) {
    return (
      <div className="attachment-inline-action">
        <p className="attachment-note attachment-loading">
          <LoaderCircle aria-hidden="true" className="spin" size={15} />
          Loading local preview…
        </p>
        <button type="button" className="button button-quiet button-small" onClick={onClose}>
          Close preview
        </button>
      </div>
    );
  }

  let preview: ReactNode;
  if (kind === 'image') {
    preview = <img className="attachment-image" src={objectUrl} alt={attachment.displayName} />;
  } else if (kind === 'audio') {
    preview = (
      <audio
        className="attachment-audio"
        src={objectUrl}
        controls
        preload="metadata"
        aria-label={`Audio preview: ${attachment.displayName}`}
      />
    );
  } else {
    preview = (
      <video
        className="attachment-video"
        src={objectUrl}
        controls
        preload="metadata"
        aria-label={`Video preview: ${attachment.displayName}`}
      />
    );
  }

  return (
    <div className="attachment-preview-stack">
      {preview}
      <button type="button" className="button button-quiet button-small" onClick={onClose}>
        Close preview
      </button>
    </div>
  );
}

function TextPreview({ api, attachment }: { api: LocalApi; attachment: AttachmentView }) {
  const [state, setState] = useState<
    { kind: 'idle' } | { kind: 'loading' } | { kind: 'ready'; text: string } | { kind: 'error' }
  >({ kind: 'idle' });

  async function loadText() {
    setState({ kind: 'loading' });
    try {
      const text = await api.attachmentText(attachment.id);
      setState({ kind: 'ready', text });
    } catch {
      setState({ kind: 'error' });
    }
  }

  if (state.kind === 'ready') {
    return (
      <pre className="text-preview" tabIndex={0}>
        <code>{state.text}</code>
      </pre>
    );
  }

  return (
    <div className="attachment-inline-action">
      <button
        type="button"
        className="button button-quiet button-small"
        onClick={() => void loadText()}
        disabled={state.kind === 'loading'}
      >
        {state.kind === 'loading' ? 'Loading…' : 'Preview text'}
      </button>
      {state.kind === 'error' ? <span role="alert">Text preview unavailable.</span> : null}
    </div>
  );
}

function PdfPreview({
  api,
  attachment,
  onClose,
}: {
  api: LocalApi;
  attachment: AttachmentView;
  onClose: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<
    { kind: 'loading' } | { kind: 'ready'; pages: number } | { kind: 'error'; message: string }
  >({ kind: 'loading' });
  const mime = safePreviewMime(attachment, 'pdf');
  const blockedMessage = pdfPreviewBlockedMessage(attachment);

  useEffect(() => {
    if (!mime || blockedMessage) return;

    let active = true;
    let cleanup: (() => void) | undefined;
    const controller = new AbortController();
    const previewCanvas = canvasRef.current;

    void (async () => {
      try {
        const blob = await api.attachmentContent(attachment.id, controller.signal);
        if (blob.size > MAX_PDF_PREVIEW_BYTES) throw new Error('preview-limit');

        await import('./pdf-worker');
        const { getDocument } = await import('pdfjs-dist');
        const data = new Uint8Array(await blob.arrayBuffer());
        const loadingTask = getDocument({
          data,
          disableAutoFetch: true,
          disableFontFace: true,
          disableStream: true,
          enableXfa: false,
          isOffscreenCanvasSupported: false,
          maxImageSize: 16_777_216,
          stopAtErrors: true,
          useSystemFonts: false,
          useWorkerFetch: false,
          useWasm: false,
        });
        cleanup = () => {
          void loadingTask.destroy();
        };

        const document = await loadingTask.promise;
        const page = await document.getPage(1);
        const viewport = page.getViewport({ scale: 1.35 });
        if (
          !Number.isFinite(viewport.width) ||
          !Number.isFinite(viewport.height) ||
          viewport.width <= 0 ||
          viewport.height <= 0 ||
          viewport.width > MAX_PDF_CANVAS_DIMENSION ||
          viewport.height > MAX_PDF_CANVAS_DIMENSION ||
          viewport.width * viewport.height > MAX_PDF_CANVAS_PIXELS
        ) {
          throw new Error('canvas-limit');
        }
        const context = previewCanvas?.getContext('2d', { alpha: false });
        if (!previewCanvas || !context) throw new Error('canvas-unavailable');

        previewCanvas.width = Math.ceil(viewport.width);
        previewCanvas.height = Math.ceil(viewport.height);
        await page.render({ canvas: previewCanvas, canvasContext: context, viewport }).promise;
        if (active) setState({ kind: 'ready', pages: document.numPages });
      } catch {
        if (active) {
          setState({
            kind: 'error',
            message:
              'A bounded first-page preview could not be created. Save a copy to inspect it locally.',
          });
        }
      }
    })();

    return () => {
      active = false;
      controller.abort();
      cleanup?.();
      if (previewCanvas) {
        previewCanvas.width = 0;
        previewCanvas.height = 0;
      }
    };
  }, [api, attachment.id, blockedMessage, mime]);

  if (blockedMessage) return <p className="attachment-note">{blockedMessage}</p>;

  return (
    <div className="pdf-preview">
      <canvas
        ref={canvasRef}
        className={state.kind === 'error' ? 'is-hidden' : undefined}
        aria-label={`First page of ${attachment.displayName}`}
      />
      {state.kind === 'loading' ? (
        <p className="attachment-note attachment-loading">
          <LoaderCircle aria-hidden="true" className="spin" size={15} />
          Rendering page one locally…
        </p>
      ) : null}
      {state.kind === 'ready' ? (
        <p className="attachment-note">Page 1 of {state.pages}</p>
      ) : null}
      {state.kind === 'error' ? <p className="attachment-note">{state.message}</p> : null}
      <button type="button" className="button button-quiet button-small" onClick={onClose}>
        Close preview
      </button>
    </div>
  );
}

function PreviewAction({
  kind,
  onActivate,
}: {
  kind: 'image' | 'audio' | 'video' | 'PDF';
  onActivate: () => void;
}) {
  return (
    <div className="attachment-inline-action">
      <button type="button" className="button button-quiet button-small" onClick={onActivate}>
        Preview {kind}
      </button>
    </div>
  );
}

export function AttachmentCard({
  api,
  attachment,
  previewActive,
  onPreviewActivate,
}: {
  api: LocalApi;
  attachment: AttachmentView;
  previewActive?: boolean;
  onPreviewActivate?: (attachmentId: string | null) => void;
}) {
  const [saving, setSaving] = useState(false);
  const [saveMessage, setSaveMessage] = useState<string | null>(null);
  const [localPreviewActive, setLocalPreviewActive] = useState(false);
  const unavailable = attachment.status !== 'available';
  const isPreviewActive = previewActive ?? localPreviewActive;
  const binaryBlockedMessage =
    attachment.previewKind === 'image' ||
    attachment.previewKind === 'audio' ||
    attachment.previewKind === 'video'
      ? mediaPreviewBlockedMessage(attachment, attachment.previewKind)
      : attachment.previewKind === 'pdf'
        ? pdfPreviewBlockedMessage(attachment)
        : null;

  function setPreviewActive(active: boolean) {
    if (onPreviewActivate) {
      onPreviewActivate(active ? attachment.id : null);
    } else {
      setLocalPreviewActive(active);
    }
  }

  async function save() {
    setSaving(true);
    setSaveMessage(null);
    try {
      const result = await api.saveAttachment(attachment.id);
      setSaveMessage(
        result.saved
          ? `Saved ${result.fileName ?? attachment.displayName}.`
          : 'Save cancelled.',
      );
    } catch {
      setSaveMessage('The attachment could not be saved.');
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="attachment" aria-label={`Attachment: ${attachment.displayName}`}>
      <header className="attachment-header">
        <span className="attachment-kind">
          <AttachmentIcon attachment={attachment} />
        </span>
        <span className="attachment-name">
          <strong>{attachment.displayName}</strong>
          <span>
            {formatAttachmentType(attachment)} · {formatBytes(attachment.byteSize)}
          </span>
        </span>
        {!unavailable ? (
          <button
            type="button"
            className="button button-quiet button-small attachment-save"
            onClick={() => void save()}
            disabled={saving}
            aria-label={`Save a copy of ${attachment.displayName}`}
            title="Save a copy"
          >
            {saving ? (
              <LoaderCircle className="spin" size={17} aria-hidden="true" />
            ) : (
              <Download size={17} aria-hidden="true" />
            )}
            <span>{saving ? 'Saving…' : 'Save copy'}</span>
          </button>
        ) : null}
      </header>

      {attachment.status === 'missing' ? (
        <p className="attachment-warning">
          <AlertTriangle size={16} aria-hidden="true" />
          This referenced file is missing from the export.
        </p>
      ) : null}
      {attachment.status === 'rejected' ? (
        <p className="attachment-warning">
          <AlertTriangle size={16} aria-hidden="true" />
          This file was rejected by the local safety checks.
        </p>
      ) : null}

      {!unavailable && binaryBlockedMessage ? (
        <p className="attachment-note">{binaryBlockedMessage}</p>
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'image' &&
      !isPreviewActive ? (
        <PreviewAction kind="image" onActivate={() => setPreviewActive(true)} />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'image' &&
      isPreviewActive ? (
        <MediaPreview
          api={api}
          attachment={attachment}
          kind="image"
          onClose={() => setPreviewActive(false)}
        />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'audio' &&
      !isPreviewActive ? (
        <PreviewAction kind="audio" onActivate={() => setPreviewActive(true)} />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'audio' &&
      isPreviewActive ? (
        <MediaPreview
          api={api}
          attachment={attachment}
          kind="audio"
          onClose={() => setPreviewActive(false)}
        />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'video' &&
      !isPreviewActive ? (
        <PreviewAction kind="video" onActivate={() => setPreviewActive(true)} />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'video' &&
      isPreviewActive ? (
        <MediaPreview
          api={api}
          attachment={attachment}
          kind="video"
          onClose={() => setPreviewActive(false)}
        />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'pdf' &&
      !isPreviewActive ? (
        <PreviewAction kind="PDF" onActivate={() => setPreviewActive(true)} />
      ) : null}
      {!unavailable &&
      !binaryBlockedMessage &&
      attachment.previewKind === 'pdf' &&
      isPreviewActive ? (
        <PdfPreview api={api} attachment={attachment} onClose={() => setPreviewActive(false)} />
      ) : null}
      {!unavailable && attachment.previewKind === 'text' ? (
        <TextPreview api={api} attachment={attachment} />
      ) : null}
      {!unavailable &&
      (attachment.previewKind === 'unsupported' || attachment.previewKind === 'missing') ? (
        <p className="attachment-note">
          No in-app preview is available for this file type. It will never be executed here.
        </p>
      ) : null}

      {saveMessage ? (
        <p className="attachment-note" role="status">
          {saveMessage}
        </p>
      ) : null}
    </section>
  );
}
