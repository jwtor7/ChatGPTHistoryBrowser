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

function jsonRequestBody(init?: RequestInit): unknown {
  if (typeof init?.body !== 'string') {
    throw new Error('Expected a synthetic JSON request body.');
  }
  return JSON.parse(init.body) as unknown;
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
    let exportSaveCalls = 0;
    let conversationSetSaveCalls = 0;
    const conversationSetRequests: Array<{ ids: string[]; format: string }> = [];
    let holdFirstPdfEstimate = true;
    let initialExportEstimateReady = false;
    let resolveInitialExportEstimate: ((response: Response) => void) | undefined;
    const initialExportEstimateResponse = new Promise<Response>((resolve) => {
      resolveInitialExportEstimate = resolve;
    });
    const initialExportEstimate = {
      conversationCount: 1,
      messageCount: 1,
      attachmentCount: 0,
      byteSize: 2_048,
      fileName: `${items[0].title.replaceAll(' ', '-')}.md`,
    };
    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = urlOf(input);
        requestedUrls.push(url);
        if (url === '/api/status') {
          return Promise.resolve(json(status(true, complete)));
        }
        if (url.startsWith('/api/conversations?')) {
          const params = new URL(url, 'http://localhost').searchParams;
          if (params.get('attachmentKind') === 'audio') {
            return Promise.resolve(
              json({
                items: [],
                page: 0,
                pageSize: 50,
                total: 0,
                hasMore: false,
              }),
            );
          }
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
        if (url.startsWith(`/api/conversations/${items[0].id}/export?`)) {
          const format = new URL(url, 'http://localhost').searchParams.get('format') ?? 'md';
          if (init?.method === 'POST') {
            exportSaveCalls += 1;
            return Promise.resolve(json({ saved: false }));
          }
          if (format === 'md') {
            return initialExportEstimateReady
              ? Promise.resolve(json(initialExportEstimate))
              : initialExportEstimateResponse;
          }
          if (format === 'pdf' && holdFirstPdfEstimate) {
            holdFirstPdfEstimate = false;
            return new Promise<Response>((_resolve, reject) => {
              init?.signal?.addEventListener(
                'abort',
                () => reject(new DOMException('Aborted', 'AbortError')),
                { once: true },
              );
            });
          }
          return Promise.resolve(
            json({
              conversationCount: 1,
              messageCount: 1,
              attachmentCount: 0,
              byteSize: format === 'pdf' ? 4_096 : 1_024,
              fileName: `${items[0].title.replaceAll(' ', '-')}.${format}`,
            }),
          );
        }
        if (url === '/api/conversation-set/export/estimate') {
          const request = jsonRequestBody(init) as {
            ids: string[];
            format: string;
          };
          conversationSetRequests.push(request);
          return Promise.resolve(
            json({
              conversationCount: request.ids.length,
              messageCount: request.ids.length,
              attachmentCount: 0,
              byteSize: request.format === 'pdf' ? 6_144 : 3_072,
              fileName: `Selected-conversations-${request.ids.length}.${request.format}`,
            }),
          );
        }
        if (url === '/api/conversation-set/export') {
          conversationSetSaveCalls += 1;
          conversationSetRequests.push(
            jsonRequestBody(init) as { ids: string[]; format: string },
          );
          return Promise.resolve(json({ saved: false }));
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
    await user.selectOptions(screen.getByRole('combobox', { name: /file type/i }), 'audio');
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('attachmentKind=audio'))).toBe(true);
    });
    expect(await screen.findByText(/file type is set to audio/i)).toBeVisible();
    expect(screen.getByText(/1 active.*audio/i)).toBeVisible();
    await user.click(screen.getByRole('button', { name: /clear file type/i }));
    await waitFor(() => {
      expect(container.querySelector('.result-summary')).toHaveTextContent('120 conversations');
    });
    await user.click(screen.getByRole('button', { name: /clear search and filters/i }));
    await waitFor(() => {
      expect(
        requestedUrls.filter((url) => url.startsWith('/api/conversations?')).length,
      ).toBeGreaterThan(listRequestsBeforeClear);
    });
    await waitFor(() => {
      expect(container.querySelector('.result-summary')).toHaveTextContent('120 conversations');
    });

    await user.click(screen.getByRole('checkbox', { name: /select this page/i }));
    expect(screen.getByText('50 selected')).toBeVisible();
    await user.click(screen.getByRole('button', { name: /next page/i }));
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('page=1'))).toBe(true);
    });
    expect(screen.getByText('Page 2 of 3')).toBeVisible();
    expect(screen.getByText('50 selected')).toBeVisible();
    expect(screen.getByRole('checkbox', { name: /select this page/i })).toBeChecked();
    await user.click(screen.getByRole('button', { name: /^clear$/i }));
    expect(screen.getByText('0 selected')).toBeVisible();

    const search = screen.getByRole('searchbox', {
      name: /search conversations/i,
    });
    await user.clear(search);
    await user.type(search, 'fictional atlas');
    await user.click(screen.getByRole('button', { name: /^search$/i }));
    await waitFor(() => {
      expect(requestedUrls.some((url) => url.includes('search=fictional+atlas'))).toBe(true);
    });

    await user.click(screen.getByRole('checkbox', { name: /select this page/i }));
    expect(screen.getByText('2 selected')).toBeVisible();
    await user.click(screen.getByRole('button', { name: /export selected/i }));
    const selectedExportDialog = await screen.findByRole('dialog', {
      name: /export 2 selected conversations/i,
    });
    expect(selectedExportDialog).toHaveTextContent(/default active paths/i);
    expect(selectedExportDialog).toHaveTextContent(/2 selected conversations/i);
    expect(selectedExportDialog).toHaveTextContent(
      /alternate branches and attachments aren.t/i,
    );
    expect(
      await within(selectedExportDialog).findByText('Selected-conversations-2.md'),
    ).toBeVisible();
    expect(conversationSetRequests[0]).toEqual({
      ids: [items[0].id, items[1].id],
      format: 'md',
    });
    await user.click(within(selectedExportDialog).getByRole('radio', { name: /plain text/i }));
    expect(
      await within(selectedExportDialog).findByText('Selected-conversations-2.txt'),
    ).toBeVisible();
    await user.click(
      within(selectedExportDialog).getByRole('button', { name: /save plain text/i }),
    );
    expect(await screen.findByRole('status')).toHaveTextContent(
      /export cancelled.*no file was created/i,
    );
    expect(conversationSetSaveCalls).toBe(1);
    expect(conversationSetRequests.at(-1)).toEqual({
      ids: [items[0].id, items[1].id],
      format: 'txt',
    });

    await user.click(screen.getByRole('button', { name: /branch 1/i }));
    expect(await screen.findByText(/conspicuously fictional alternate branch/i)).toBeVisible();

    await user.click(screen.getByRole('button', { name: /export current path/i }));
    const exportDialog = await screen.findByRole('dialog', {
      name: new RegExp(`export.*${items[0].title}`, 'i'),
    });
    expect(within(exportDialog).getByRole('button', { name: /cancel/i })).toBeEnabled();
    expect(exportDialog).toHaveTextContent(/preparing filename/i);
    await waitFor(() => {
      expect(
        requestedUrls.some((url) => url.includes('/export?') && url.includes('format=md')),
      ).toBe(true);
    });
    initialExportEstimateReady = true;
    resolveInitialExportEstimate?.(json(initialExportEstimate));
    expect(exportDialog).toHaveTextContent(/contains private conversation data/i);
    expect(exportDialog).toHaveTextContent(/filename uses the conversation title/i);
    expect(exportDialog).toHaveTextContent(/sharing or importing the saved file/i);
    expect(exportDialog).toHaveTextContent(
      /only the currently selected message path is included/i,
    );
    expect(exportDialog).toHaveTextContent(/alternate branches and attachments aren.t/i);
    expect(exportDialog).not.toHaveTextContent(/provider-neutral|portable json/i);
    await waitFor(() =>
      expect(within(exportDialog).getByRole('radio', { name: /markdown/i })).toHaveFocus(),
    );
    expect(
      await within(exportDialog).findByText(`${items[0].title.replaceAll(' ', '-')}.md`),
    ).toBeVisible();
    expect(exportDialog).toHaveTextContent(/2\.0 KB/i);
    expect(exportDialog).toHaveTextContent(/0 found, none included/i);
    await user.click(within(exportDialog).getByRole('radio', { name: /^pdf/i }));
    expect(within(exportDialog).getByRole('button', { name: /cancel/i })).toBeEnabled();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(exportDialog).not.toBeInTheDocument());
    expect(screen.getByRole('status')).toHaveTextContent(/export cancelled/i);

    await user.click(screen.getByRole('button', { name: /export current path/i }));
    const reopenedExportDialog = await screen.findByRole('dialog', {
      name: new RegExp(`export.*${items[0].title}`, 'i'),
    });
    await user.click(within(reopenedExportDialog).getByRole('radio', { name: /^pdf/i }));
    expect(
      await within(reopenedExportDialog).findByText(
        `${items[0].title.replaceAll(' ', '-')}.pdf`,
      ),
    ).toBeVisible();
    expect(requestedUrls.some((url) => url.includes('/export?format=pdf'))).toBe(true);
    await user.click(within(reopenedExportDialog).getByRole('button', { name: /save pdf/i }));
    expect(await screen.findByRole('status')).toHaveTextContent(
      /export cancelled.*no file was created/i,
    );
    expect(exportSaveCalls).toBe(1);
    expect(
      requestedUrls.some(
        (url) =>
          url.includes('/export?format=pdf&leaf=synthetic-branch-leaf') &&
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
