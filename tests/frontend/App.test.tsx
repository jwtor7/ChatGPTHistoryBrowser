import axe from 'axe-core';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { App } from '../../src/App';
import type {
  AppStatus,
  ConversationDetail,
  ConversationListItem,
  IndexProgress,
} from '../../src/types';

const TOKEN = 'synthetic_e2e_capability';

const IDLE_INDEX: IndexProgress = {
  phase: 'idle',
  failureCode: null,
  shardsTotal: 2,
  shardsComplete: 0,
  bytesTotal: 2_000,
  bytesProcessed: 0,
  conversationsIndexed: 0,
  conversationsSkipped: 0,
  diagnostics: 0,
};

function status(exportSelected: boolean, index: IndexProgress = IDLE_INDEX): AppStatus {
  return {
    exportSelected,
    shardCount: exportSelected ? 2 : 0,
    attachmentFileCount: exportSelected ? 3 : 0,
    index,
  };
}

function json(body: unknown, init?: ResponseInit): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  });
}

function urlOf(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input;
  if (input instanceof URL) return input.href;
  return input.url;
}

afterEach(() => {
  vi.useRealTimers();
});

describe('History Browser frontend', () => {
  it('presents the complete privacy onboarding with no serious accessibility violations', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(json(status(false)))),
    );

    const { container } = render(<App initialToken={TOKEN} />);

    expect(
      await screen.findByRole('heading', {
        name: /browse your chatgpt export without uploading it/i,
      }),
    ).toBeVisible();
    expect(screen.getByText(/selected export is never edited/i)).toBeVisible();
    expect(screen.getByText(/no tool can guarantee universal detection/i)).toBeVisible();
    expect(screen.getByRole('button', { name: /choose extracted export/i })).toBeEnabled();

    const results = await axe.run(container);
    expect(
      results.violations.filter((violation) =>
        ['serious', 'critical'].includes(violation.impact ?? ''),
      ),
    ).toEqual([]);
  });

  it('selects an export, reports indexing progress, and cancels cooperatively', async () => {
    const indexing: IndexProgress = {
      ...IDLE_INDEX,
      phase: 'indexing',
      shardsComplete: 1,
      bytesProcessed: 1_000,
      conversationsIndexed: 42,
      conversationsSkipped: 1,
      diagnostics: 2,
    };
    const cancelled: IndexProgress = { ...indexing, phase: 'cancelled' };
    let statusCalls = 0;

    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        void init;
        const url = urlOf(input);
        if (url === '/api/status') {
          statusCalls += 1;
          return Promise.resolve(
            json(statusCalls === 1 ? status(false) : status(true, indexing)),
          );
        }
        if (url === '/api/export/pick') {
          return Promise.resolve(
            json({
              supported: true,
              shardCount: 2,
              attachmentFileCount: 3,
              totalJsonBytes: 2_000,
            }),
          );
        }
        if (url === '/api/index/cancel') return Promise.resolve(json(cancelled));
        if (url === '/api/index/status') return Promise.resolve(json(indexing));
        return Promise.reject(new Error(`Unexpected synthetic request: ${url}`));
      },
    );
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();

    render(<App initialToken={TOKEN} />);
    await user.click(await screen.findByRole('button', { name: /choose extracted export/i }));

    expect(
      await screen.findByRole('heading', {
        name: /building the local search index/i,
      }),
    ).toBeVisible();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '50');
    expect(screen.getByText('42')).toBeVisible();

    await user.click(screen.getByRole('button', { name: /stop indexing/i }));
    expect(await screen.findByRole('heading', { name: /indexing stopped/i })).toBeVisible();

    for (const call of fetchMock.mock.calls) {
      const init = call[1];
      const headers = new Headers(init?.headers);
      expect(headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
    }
  });

  it('virtualizes, searches, paginates, opens active paths, and switches branches', async () => {
    const complete: IndexProgress = {
      ...IDLE_INDEX,
      phase: 'complete',
      shardsComplete: 2,
      bytesProcessed: 2_000,
      conversationsIndexed: 120,
    };
    const items: ConversationListItem[] = Array.from({ length: 50 }, (_, index) => ({
      id: `synthetic-conversation-${index + 1}`,
      title: `Fictional Lantern Log ${index + 1}`,
      createdAt: 1_735_689_600 + index,
      updatedAt: 1_735_776_000 + index,
      archived: false,
      starred: index === 0,
      hasAttachments: index === 0,
      messageCount: 3,
      matchPreview: null,
    }));

    function detail(branch = false, item: ConversationListItem = items[0]): ConversationDetail {
      return {
        id: item.id,
        title: item.title,
        createdAt: item.createdAt,
        updatedAt: item.updatedAt,
        archived: false,
        starred: true,
        selectedLeaf: branch ? 'synthetic-branch-leaf' : 'synthetic-main-leaf',
        diagnostics: [],
        messages: [
          {
            nodeId: branch ? 'synthetic-node-branch' : 'synthetic-node-main',
            role: branch ? 'assistant' : 'user',
            createdAt: item.createdAt,
            contentType: 'text',
            text: branch
              ? 'This is the conspicuously fictional alternate branch.'
              : 'This is a **synthetic** question about a lantern on Example Island.',
            attachments: [],
            alternateBranches: branch
              ? []
              : [
                  {
                    leafNodeId: 'synthetic-branch-leaf',
                    role: 'assistant',
                    preview: 'Fictional alternate answer',
                  },
                ],
          },
        ],
      };
    }

    const requestedUrls: string[] = [];
    let portableSaveCalls = 0;
    let resolvePortableEstimate: ((response: Response) => void) | undefined;
    const portableEstimateResponse = new Promise<Response>((resolve) => {
      resolvePortableEstimate = resolve;
    });
    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = urlOf(input);
        requestedUrls.push(url);
        if (url === '/api/status') {
          return Promise.resolve(json(status(true, complete)));
        }
        if (url.startsWith('/api/conversations?')) {
          const params = new URL(url, 'http://localhost').searchParams;
          if (params.get('search') === 'broken query') {
            return Promise.reject(new Error('Synthetic list failure'));
          }
          if (params.get('search') === 'fictional atlas') {
            return Promise.resolve(
              json({
                items: [items[0], items[1]],
                page: 0,
                pageSize: 50,
                total: 2,
                hasMore: false,
              }),
            );
          }
          if (params.get('search') === 'switch target') {
            return Promise.resolve(
              json({
                items: [items[1]],
                page: 0,
                pageSize: 50,
                total: 1,
                hasMore: false,
              }),
            );
          }
          return Promise.resolve(
            json({
              items,
              page: Number(params.get('page') ?? '0'),
              pageSize: 50,
              total: 120,
              hasMore: true,
            }),
          );
        }
        if (url.startsWith(`/api/conversations/${items[0].id}/portable-export`)) {
          if (init?.method === 'POST') {
            portableSaveCalls += 1;
            return Promise.resolve(json({ saved: false }));
          }
          return portableEstimateResponse;
        }
        if (url.includes('?leaf=synthetic-branch-leaf')) {
          return Promise.resolve(json(detail(true)));
        }
        if (url === `/api/conversations/${items[0].id}`) {
          return Promise.resolve(json(detail()));
        }
        if (url.startsWith('/api/conversations/')) {
          const item =
            items.find((candidate) => url.startsWith(`/api/conversations/${candidate.id}`)) ??
            items[0];
          return Promise.resolve(json(detail(false, item)));
        }
        return Promise.reject(new Error(`Unexpected synthetic request: ${url}`));
      },
    );
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();

    const { container } = render(<App initialToken={TOKEN} />);
    expect(await screen.findByRole('heading', { name: items[0].title })).toBeVisible();
    expect(requestedUrls.some((url) => url.includes('page=0'))).toBe(true);
    expect(screen.getByText('Page 1 of 3')).toBeVisible();
    expect(screen.getByRole('button', { name: /previous page/i })).toBeDisabled();
    expect(screen.getByRole('article', { name: /you message/i })).toHaveTextContent(
      /synthetic question/i,
    );

    const list = screen.getByRole('list', { name: /conversations/i });
    expect(list.querySelectorAll('[role="listitem"]').length).toBeLessThan(50);

    const listRequestsBeforeClear = requestedUrls.filter((url) =>
      url.startsWith('/api/conversations?'),
    ).length;
    await user.click(screen.getByText(/^filters$/i));
    await user.click(screen.getByRole('button', { name: /clear filters/i }));
    await waitFor(() => {
      expect(
        requestedUrls.filter((url) => url.startsWith('/api/conversations?')).length,
      ).toBeGreaterThan(listRequestsBeforeClear);
    });
    await waitFor(() => {
      expect(container.querySelector('.result-summary')).toHaveTextContent('120 conversations');
    });

    await user.click(screen.getByRole('button', { name: /next page/i }));
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('page=1'))).toBe(true);
    });
    expect(screen.getByText('Page 2 of 3')).toBeVisible();

    const search = screen.getByRole('searchbox', {
      name: /search conversations/i,
    });
    await user.clear(search);
    await user.type(search, 'fictional atlas');
    await user.click(screen.getByRole('button', { name: /^search$/i }));
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('search=fictional+atlas'))).toBe(true);
    });

    await user.click(screen.getByRole('button', { name: /branch 1/i }));
    expect(await screen.findByText(/conspicuously fictional alternate branch/i)).toBeVisible();

    await user.click(screen.getByRole('button', { name: /export active path/i }));
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('/portable-export'))).toBe(true);
    });
    await user.clear(search);
    await user.type(search, 'switch target');
    await user.click(screen.getByRole('button', { name: /^search$/i }));
    expect(await screen.findByRole('heading', { name: items[1].title })).toBeVisible();
    resolvePortableEstimate?.(
      json({
        conversationCount: 1,
        messageCount: 1,
        attachmentCount: 0,
        byteSize: 2_048,
      }),
    );
    const exportDialog = await screen.findByRole('dialog', {
      name: /export this active path/i,
    });
    expect(exportDialog).toHaveTextContent(/provider-neutral json package/i);
    expect(exportDialog).toHaveTextContent(items[0].title);
    expect(exportDialog).toHaveTextContent(/2\.0 KB/i);
    expect(exportDialog).toHaveTextContent(/0 detected, none included/i);
    await user.click(
      within(exportDialog).getByRole('button', { name: /choose save location/i }),
    );
    expect(await screen.findByRole('status')).toHaveTextContent(
      /export cancelled.*no file was created/i,
    );
    expect(portableSaveCalls).toBe(1);
    expect(
      requestedUrls.some(
        (url) =>
          url.includes('/portable-export?leaf=synthetic-branch-leaf') &&
          url.startsWith('/api/'),
      ),
    ).toBe(true);

    const discardTrigger = screen.getByRole('button', { name: /delete local index/i });
    await user.click(discardTrigger);
    const dialog = screen.getByRole('dialog', { name: /delete the local index/i });
    const keepIndex = within(dialog).getByRole('button', { name: /keep index/i });
    const deleteIndex = within(dialog).getByRole('button', {
      name: /^delete local index$/i,
    });
    await waitFor(() => expect(keepIndex).toHaveFocus());
    await user.tab();
    expect(deleteIndex).toHaveFocus();
    await user.tab();
    expect(keepIndex).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(dialog).not.toBeInTheDocument();
    expect(discardTrigger).toHaveFocus();

    await user.clear(search);
    await user.type(search, 'broken query');
    await user.click(screen.getByRole('button', { name: /^search$/i }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      /could not complete this request/i,
    );
    expect(screen.queryByRole('heading', { name: items[0].title })).toBeNull();
    expect(screen.getByRole('button', { name: /try again/i })).toBeVisible();

    const results = await axe.run(container);
    expect(
      results.violations.filter((violation) =>
        ['serious', 'critical'].includes(violation.impact ?? ''),
      ),
    ).toEqual([]);
  });
});
