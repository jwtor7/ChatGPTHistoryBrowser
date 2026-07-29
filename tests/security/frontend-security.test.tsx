import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { LocalApi } from '../../src/api';
import { AttachmentCard } from '../../src/AttachmentCard';
import { BoundedAttachmentList } from '../../src/ConversationBrowser';
import { SafeMarkdown } from '../../src/SafeMarkdown';
import { bootstrapSessionToken, clearSessionToken } from '../../src/token';
import type { AttachmentView } from '../../src/types';

describe('renderer security boundaries', () => {
  beforeEach(() => clearSessionToken());

  it('removes the fragment before persisting a valid capability', () => {
    const order: string[] = [];
    const location = {
      hash: '#token=synthetic_base64url-token',
      pathname: '/index.html',
      search: '?local=1',
    };
    const history = {
      state: null,
      replaceState: vi.fn(() => order.push('replace')),
    };

    expect(bootstrapSessionToken(location, history)).toBe('synthetic_base64url-token');
    expect(order).toEqual(['replace']);
    expect(history.replaceState).toHaveBeenCalledWith(null, '', '/index.html?local=1');
    expect(window.sessionStorage.length).toBe(0);
    clearSessionToken();
  });

  it('drops invalid fragment capabilities after clearing the visible URL', () => {
    const history = { state: null, replaceState: vi.fn() };

    bootstrapSessionToken({ hash: '#token=bad%20token', pathname: '/', search: '' }, history);

    expect(history.replaceState).toHaveBeenCalled();
    expect(window.sessionStorage.length).toBe(0);
  });

  it('renders archived Markdown without executable elements, fetchable resources, or live links', () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const malicious = [
      '# Synthetic safety sample',
      '<script>globalThis.compromised = true</script>',
      '<iframe src="https://frame.invalid"></iframe>',
      '<img src="https://pixel.invalid/tracker.png" onerror="alert(1)">',
      '![Remote tracking image](https://pixel.invalid/markdown.png)',
      '[Archived destination](https://destination.invalid/private?q=content)',
      '[Script destination](javascript:alert(1))',
      '```html',
      '<button onclick="alert(1)">code remains text</button>',
      '```',
    ].join('\n\n');

    const { container } = render(<SafeMarkdown>{malicious}</SafeMarkdown>);

    expect(screen.getByText(/synthetic safety sample/i)).toBeVisible();
    expect(screen.getByText(/image reference blocked/i)).toBeVisible();
    expect(screen.getByText('Archived destination')).toBeVisible();
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('iframe')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('a[href]')).toBeNull();
    expect(container.querySelector('[onclick]')).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('uses only authenticated same-origin requests with redirects and referrers disabled', async () => {
    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        void input;
        void init;
        return Promise.resolve(
          new Response(
            JSON.stringify({
              exportSelected: false,
              shardCount: 0,
              attachmentFileCount: 0,
              index: {
                phase: 'idle',
                shardsTotal: 0,
                shardsComplete: 0,
                bytesTotal: 0,
                bytesProcessed: 0,
                conversationsIndexed: 0,
                conversationsSkipped: 0,
                diagnostics: 0,
              },
            }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        );
      },
    );
    vi.stubGlobal('fetch', fetchMock);
    const api = new LocalApi(() => 'synthetic_api_capability');

    await api.status();

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    expect(init).toBeDefined();
    expect(url).toBe('/api/status');
    const urlText = typeof url === 'string' ? url : url instanceof URL ? url.href : url.url;
    expect(urlText).not.toContain('synthetic_api_capability');
    expect(init?.redirect).toBe('error');
    expect(init?.referrerPolicy).toBe('no-referrer');
    expect(init?.credentials).toBe('same-origin');
    expect(new Headers(init?.headers).get('Authorization')).toBe(
      ['Bearer', 'synthetic_api_capability'].join(' '),
    );
  });

  it('fails closed on an active MIME type even when metadata requests an image preview', () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachment: AttachmentView = {
      id: 'synthetic-svg-attachment',
      displayName: 'fictional-diagram.svg',
      claimedMime: 'image/png',
      detectedMime: 'image/svg+xml',
      byteSize: 123,
      status: 'available',
      previewKind: 'image',
    };

    render(<AttachmentCard api={api} attachment={attachment} />);

    expect(screen.getByText(/detected file type is not allowlisted/i)).toBeVisible();
    expect(screen.queryByRole('img')).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('never fetches preview bytes before an explicit user action', async () => {
    const fetchMock = vi.fn(() =>
      Promise.resolve(
        new Response(new Uint8Array([137, 80, 78, 71]), {
          status: 200,
          headers: { 'Content-Type': 'image/png' },
        }),
      ),
    );
    vi.stubGlobal('fetch', fetchMock);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachment: AttachmentView = {
      id: 'synthetic-safe-image',
      displayName: 'fictional-safe-image.png',
      claimedMime: 'image/png',
      detectedMime: 'image/png',
      byteSize: 4,
      status: 'available',
      previewKind: 'image',
    };
    const user = userEvent.setup();

    render(<AttachmentCard api={api} attachment={attachment} />);
    expect(fetchMock).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: /preview image/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
  });

  it('reveals large attachment collections in bounded batches', async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachments: AttachmentView[] = Array.from({ length: 2_000 }, (_, index) => ({
      id: `synthetic-attachment-${index}`,
      displayName: `fictional-attachment-${index}.png`,
      claimedMime: 'image/png',
      detectedMime: 'image/png',
      byteSize: 4,
      status: 'available',
      previewKind: 'image',
    }));
    const user = userEvent.setup();
    const { container } = render(<BoundedAttachmentList api={api} attachments={attachments} />);

    expect(container.querySelectorAll('.attachment')).toHaveLength(24);
    expect(fetchSpy).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: /show 24 more attachments/i }));
    expect(container.querySelectorAll('.attachment')).toHaveLength(48);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('keeps at most one binary attachment preview active per message', async () => {
    const revokeObjectUrl = vi.spyOn(URL, 'revokeObjectURL');
    const fetchMock = vi.fn(() =>
      Promise.resolve(
        new Response(new Uint8Array([137, 80, 78, 71]), {
          status: 200,
          headers: { 'Content-Type': 'image/png' },
        }),
      ),
    );
    vi.stubGlobal('fetch', fetchMock);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachments: AttachmentView[] = [0, 1].map((index) => ({
      id: `synthetic-preview-${index}`,
      displayName: `fictional-preview-${index}.png`,
      claimedMime: 'image/png',
      detectedMime: 'image/png',
      byteSize: 4,
      status: 'available',
      previewKind: 'image',
    }));
    const user = userEvent.setup();

    render(<BoundedAttachmentList api={api} attachments={attachments} />);
    const initialButtons = screen.getAllByRole('button', { name: /preview image/i });
    await user.click(initialButtons[0]);
    await screen.findByRole('img', { name: attachments[0].displayName });

    await user.click(screen.getByRole('button', { name: /preview image/i }));
    await screen.findByRole('img', { name: attachments[1].displayName });

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('img', { name: attachments[0].displayName })).toBeNull();
    expect(revokeObjectUrl).toHaveBeenCalled();
  });

  it('keeps oversized PDFs behind the documented bounded fallback', () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachment: AttachmentView = {
      id: 'synthetic-large-pdf',
      displayName: 'fictional-large-document.pdf',
      claimedMime: 'application/pdf',
      detectedMime: 'application/pdf',
      byteSize: 20 * 1024 * 1024 + 1,
      status: 'available',
      previewKind: 'pdf',
    };

    render(<AttachmentCard api={api} attachment={attachment} />);

    expect(screen.getByText(/limited to local files under 20 mb/i)).toBeVisible();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('renders text attachment bytes as inert text rather than DOM', async () => {
    const fetchMock = vi.fn(() =>
      Promise.resolve(
        new Response('<script>globalThis.compromised = true</script>', {
          status: 200,
          headers: { 'Content-Type': 'text/plain' },
        }),
      ),
    );
    vi.stubGlobal('fetch', fetchMock);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachment: AttachmentView = {
      id: 'synthetic-text-attachment',
      displayName: 'fictional-note.txt',
      claimedMime: 'text/plain',
      detectedMime: 'text/plain',
      byteSize: 54,
      status: 'available',
      previewKind: 'text',
    };
    const user = userEvent.setup();
    const { container } = render(<AttachmentCard api={api} attachment={attachment} />);

    await user.click(screen.getByRole('button', { name: /preview text/i }));

    expect(await screen.findByText(/globalThis\.compromised/i)).toBeVisible();
    expect(container.querySelector('script')).toBeNull();
    expect(fetchMock).toHaveBeenCalledWith(
      '/api/attachments/synthetic-text-attachment/text',
      expect.objectContaining({ redirect: 'error' }),
    );
  });

  it('distinguishes a cancelled attachment save from a completed copy', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ saved: false }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ saved: true }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    vi.stubGlobal('fetch', fetchMock);
    const api = new LocalApi(() => 'synthetic_api_capability');
    const attachment: AttachmentView = {
      id: 'synthetic-save-result',
      displayName: 'fictional-save-result.txt',
      claimedMime: 'text/plain',
      detectedMime: 'text/plain',
      byteSize: 32,
      status: 'available',
      previewKind: 'text',
    };
    const user = userEvent.setup();

    render(<AttachmentCard api={api} attachment={attachment} />);
    const save = screen.getByRole('button', { name: /save a copy/i });
    await user.click(save);
    expect(await screen.findByText(/save cancelled/i)).toBeVisible();
    await user.click(save);
    expect(await screen.findByText(/saved fictional-save-result\.txt/i)).toBeVisible();
  });
});
