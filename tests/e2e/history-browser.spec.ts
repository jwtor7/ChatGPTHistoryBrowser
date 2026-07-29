import { expect, type Page, test } from '@playwright/test';
import process from 'node:process';

const ORIGIN = 'http://127.0.0.1:4173';
const TOKEN = 'synthetic_playwright_capability';

const INDEX = {
  phase: 'complete',
  failureCode: null,
  shardsTotal: 4,
  shardsComplete: 4,
  bytesTotal: 8_000,
  bytesProcessed: 8_000,
  conversationsIndexed: 50,
  conversationsSkipped: 1,
  diagnostics: 2,
};

const ITEMS = Array.from({ length: 50 }, (_, index) => ({
  id: `synthetic-conversation-${index + 1}`,
  title: `Fictional Lantern Atlas ${index + 1}`,
  createdAt: 1_735_689_600 + index,
  updatedAt: 1_735_776_000 + index,
  archived: false,
  starred: index === 0,
  hasAttachments: index === 0,
  messageCount: 3,
  matchPreview: null,
}));

function conversationDetail(branch: boolean) {
  return {
    id: ITEMS[0].id,
    title: ITEMS[0].title,
    createdAt: ITEMS[0].createdAt,
    updatedAt: ITEMS[0].updatedAt,
    archived: false,
    starred: true,
    selectedLeaf: branch ? 'synthetic-alternate-leaf' : 'synthetic-main-leaf',
    diagnostics: [],
    messages: branch
      ? [
          {
            nodeId: 'synthetic-alternate-node',
            role: 'assistant',
            createdAt: ITEMS[0].createdAt,
            contentType: 'text',
            text: 'The alternate path is conspicuously fictional and contains no personal data.',
            attachments: [],
            alternateBranches: [],
          },
        ]
      : [
          {
            nodeId: 'synthetic-product-node',
            role: 'user',
            createdAt: ITEMS[0].createdAt,
            contentType: 'text',
            text: [
              '# A searchable local archive',
              '',
              'This fictional **Lantern Atlas** export is indexed privately on this Mac.',
            ].join('\n'),
            attachments: [
              {
                id: 'synthetic-text-attachment',
                displayName: 'fictional-map-notes.txt',
                claimedMime: 'text/plain',
                detectedMime: 'text/plain',
                byteSize: 74,
                status: 'available',
                previewKind: 'text',
              },
              {
                id: 'synthetic-unsupported-attachment',
                displayName: 'fictional-active-format.html',
                claimedMime: 'text/html',
                detectedMime: 'text/html',
                byteSize: 44,
                status: 'available',
                previewKind: 'unsupported',
              },
            ],
            alternateBranches: [],
          },
          {
            nodeId: 'synthetic-security-node',
            role: 'assistant',
            createdAt: ITEMS[0].createdAt + 1,
            contentType: 'text',
            text: [
              '# Synthetic archive message',
              '',
              'A fictional cartographer mapped **Example Island**.',
              '',
              '![Tracking pixel](https://pixel.invalid/tracker.png)',
              '',
              '[Archived destination](https://destination.invalid/private)',
              '',
              '<script>globalThis.compromised = true</script>',
            ].join('\n'),
            attachments: [],
            alternateBranches: [
              {
                leafNodeId: 'synthetic-alternate-leaf',
                role: 'assistant',
                preview: 'Fictional alternate answer',
              },
            ],
          },
        ],
  };
}

async function installSameOriginApi(page: Page) {
  const offOriginRequests: string[] = [];
  const apiRequests: URL[] = [];

  page.on('request', (request) => {
    const url = new URL(request.url());
    if ((url.protocol === 'http:' || url.protocol === 'https:') && url.origin !== ORIGIN) {
      offOriginRequests.push(url.href);
    }
  });

  await page.route('**/*', async (route) => {
    const request = route.request();
    const url = new URL(request.url());

    if ((url.protocol === 'http:' || url.protocol === 'https:') && url.origin !== ORIGIN) {
      await route.abort('blockedbyclient');
      throw new Error(`Off-origin request blocked: ${url.origin}`);
    }

    if (!url.pathname.startsWith('/api/')) {
      await route.continue();
      return;
    }

    apiRequests.push(url);
    if (request.headers().authorization !== `Bearer ${TOKEN}`) {
      await route.fulfill({
        status: 401,
        contentType: 'application/json',
        body: JSON.stringify({
          error: { code: 'UNAUTHORIZED', message: 'Synthetic denial.' },
        }),
      });
      return;
    }

    if (url.pathname === '/api/status') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          exportSelected: true,
          shardCount: 4,
          attachmentFileCount: 2,
          index: INDEX,
        }),
      });
      return;
    }

    if (url.pathname === '/api/conversations') {
      const search = url.searchParams.get('search');
      const filtered = search ? [ITEMS[0]] : ITEMS;
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          items: filtered,
          page: Number(url.searchParams.get('page') ?? '0'),
          pageSize: 50,
          total: search ? 1 : 50,
          hasMore: false,
        }),
      });
      return;
    }

    if (url.pathname === `/api/conversations/${ITEMS[0].id}`) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify(
          conversationDetail(url.searchParams.get('leaf') === 'synthetic-alternate-leaf'),
        ),
      });
      return;
    }

    if (url.pathname.startsWith('/api/conversations/')) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify(conversationDetail(false)),
      });
      return;
    }

    if (url.pathname === '/api/attachments/synthetic-text-attachment/text') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          text: '<img src="https://pixel.invalid/from-text"> remains inert text',
        }),
      });
      return;
    }

    if (url.pathname.startsWith('/api/attachments/') && url.pathname.endsWith('/save')) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ saved: true }),
      });
      return;
    }

    await route.fulfill({
      status: 404,
      contentType: 'application/json',
      body: JSON.stringify({
        error: { code: 'SYNTHETIC_NOT_FOUND', message: 'Synthetic not found.' },
      }),
    });
  });

  return { apiRequests, offOriginRequests };
}

test('production build browses safely with same-origin mocked APIs only', async ({ page }) => {
  const observed = await installSameOriginApi(page);

  await page.goto(`/#token=${TOKEN}`);

  await expect(page).toHaveURL(`${ORIGIN}/`);
  await expect(page.getByRole('heading', { name: ITEMS[0].title })).toBeVisible();
  await expect(page.getByText('Page 1 of 1')).toBeVisible();
  await expect(page.getByRole('button', { name: /previous page/i })).toBeDisabled();
  await expect
    .poll(() => observed.apiRequests.some((url) => url.searchParams.get('page') === '0'))
    .toBe(true);
  await expect(page.getByText('Synthetic archive message')).toBeVisible();
  await expect(page.getByText('Image reference blocked: Tracking pixel')).toBeVisible();
  await expect(page.getByText('Archived destination')).toBeVisible();

  await expect(page.locator('.message-content script')).toHaveCount(0);
  await expect(page.locator('.message-content img')).toHaveCount(0);
  await expect(page.locator('.message-content a[href]')).toHaveCount(0);
  await expect(page.locator('.conversation-row')).not.toHaveCount(50);

  if (process.env.GENERATE_SYNTHETIC_SCREENSHOT === '1') {
    await page.waitForTimeout(800);
    await page.screenshot({
      path: 'docs/images/synthetic-browser.png',
      fullPage: true,
    });
  }

  await page.getByRole('button', { name: /preview text/i }).click();
  await expect(page.getByText(/pixel\.invalid\/from-text.*remains inert text/i)).toBeVisible();
  await expect(page.locator('.text-preview img')).toHaveCount(0);

  await page.getByRole('button', { name: /branch 1/i }).click();
  await expect(page.getByText(/alternate path is conspicuously fictional/i)).toBeVisible();

  const search = page.getByRole('searchbox', {
    name: /search conversations/i,
  });
  await search.fill('fictional atlas');
  await page.getByRole('button', { name: /^search$/i }).click();
  await expect
    .poll(() =>
      observed.apiRequests.some((url) => url.searchParams.get('search') === 'fictional atlas'),
    )
    .toBe(true);

  expect(observed.offOriginRequests).toEqual([]);
  expect(observed.apiRequests.every((url) => url.origin === ORIGIN)).toBe(true);
});

test('mobile layout opens a conversation and returns to the virtualized list', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const observed = await installSameOriginApi(page);
  await page.goto(`/#token=${TOKEN}`);

  await expect(page.getByRole('list', { name: /conversations/i })).toBeVisible();
  await page
    .locator('.conversation-row')
    .filter({ has: page.getByText(ITEMS[0].title, { exact: true }) })
    .click();

  await expect(page.getByRole('heading', { name: ITEMS[0].title })).toBeVisible();
  await expect(page.getByRole('button', { name: /conversations/i })).toBeVisible();
  await page.getByRole('button', { name: /conversations/i }).click();
  await expect(page.getByRole('list', { name: /conversations/i })).toBeVisible();

  expect(observed.offOriginRequests).toEqual([]);
});
