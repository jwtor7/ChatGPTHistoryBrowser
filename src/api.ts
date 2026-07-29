import type {
  ApiErrorBody,
  AppStatus,
  ConversationDetail,
  ConversationFilters,
  ConversationPage,
  ExportValidation,
  IndexProgress,
  PortableExportEstimate,
} from './types';

const API_PREFIX = '/api/';

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, message: string, status: number) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

function assertPrivateApiPath(path: string): void {
  const hasControlCharacter = Array.from(path).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint < 32 || codePoint === 127;
  });
  if (
    !path.startsWith(API_PREFIX) ||
    path.startsWith('//') ||
    path.includes('\\') ||
    hasControlCharacter
  ) {
    throw new Error('Only same-origin private API paths are allowed.');
  }
}

async function safeError(response: Response): Promise<ApiError> {
  let body: ApiErrorBody | null = null;

  try {
    body = (await response.json()) as ApiErrorBody;
  } catch {
    // The renderer deliberately ignores raw server bodies.
  }

  const code = body?.error?.code ?? 'REQUEST_FAILED';
  const message =
    body?.error?.message ?? 'The local application could not complete this request.';

  return new ApiError(code, message, response.status);
}

function dateBoundarySeconds(value: string, endOfDay: boolean): string | null {
  if (!value) {
    return null;
  }

  const suffix = endOfDay ? 'T23:59:59.999' : 'T00:00:00.000';
  const milliseconds = new Date(`${value}${suffix}`).getTime();
  return Number.isFinite(milliseconds) ? String(milliseconds / 1_000) : null;
}

export class LocalApi {
  constructor(private readonly getToken: () => string | null) {}

  private async request<T>(
    path: string,
    init: Omit<RequestInit, 'credentials' | 'redirect' | 'referrerPolicy'> = {},
  ): Promise<T> {
    assertPrivateApiPath(path);

    const token = this.getToken();
    if (!token) {
      throw new ApiError(
        'AUTH_REQUIRED',
        'This local session has expired. Restart the application to continue.',
        401,
      );
    }

    const headers = new Headers(init.headers);
    headers.set('Accept', 'application/json');
    headers.set('Authorization', `Bearer ${token}`);
    if (init.body !== undefined && !headers.has('Content-Type')) {
      headers.set('Content-Type', 'application/json');
    }

    const response = await fetch(path, {
      ...init,
      headers,
      cache: 'no-store',
      credentials: 'same-origin',
      redirect: 'error',
      referrerPolicy: 'no-referrer',
    });

    if (!response.ok) {
      throw await safeError(response);
    }

    if (response.status === 204) {
      return undefined as T;
    }

    return (await response.json()) as T;
  }

  status(): Promise<AppStatus> {
    return this.request<AppStatus>('/api/status');
  }

  pickExport(): Promise<ExportValidation | undefined> {
    return this.request('/api/export/pick', {
      method: 'POST',
      body: JSON.stringify({}),
    });
  }

  startIndex(): Promise<IndexProgress | { index: IndexProgress }> {
    return this.request('/api/index/start', {
      method: 'POST',
      body: JSON.stringify({}),
    });
  }

  cancelIndex(): Promise<IndexProgress | { index: IndexProgress }> {
    return this.request('/api/index/cancel', {
      method: 'POST',
      body: JSON.stringify({}),
    });
  }

  discardIndex(): Promise<IndexProgress | AppStatus | { index: IndexProgress }> {
    return this.request('/api/index/discard', {
      method: 'POST',
      body: JSON.stringify({}),
    });
  }

  indexStatus(): Promise<IndexProgress> {
    return this.request('/api/index/status');
  }

  conversations(filters: ConversationFilters): Promise<ConversationPage> {
    const query = new URLSearchParams({
      page: String(filters.page),
      pageSize: String(filters.pageSize),
    });

    if (filters.search) query.set('search', filters.search);
    const from = dateBoundarySeconds(filters.dateFrom, false);
    const to = dateBoundarySeconds(filters.dateTo, true);
    if (from) query.set('dateFrom', from);
    if (to) query.set('dateTo', to);
    if (filters.role) query.set('role', filters.role);
    if (filters.archived) query.set('archived', filters.archived);
    if (filters.starred) query.set('starred', filters.starred);
    if (filters.hasAttachments) {
      query.set('hasAttachments', filters.hasAttachments);
    }

    return this.request(`/api/conversations?${query.toString()}`);
  }

  conversation(id: string, leaf?: string): Promise<ConversationDetail> {
    const suffix = leaf ? `?leaf=${encodeURIComponent(leaf)}` : '';
    return this.request(`/api/conversations/${encodeURIComponent(id)}${suffix}`);
  }

  portableExportEstimate(id: string, leaf?: string): Promise<PortableExportEstimate> {
    const suffix = leaf ? `?leaf=${encodeURIComponent(leaf)}` : '';
    return this.request(
      `/api/conversations/${encodeURIComponent(id)}/portable-export${suffix}`,
    );
  }

  savePortableExport(id: string, leaf?: string): Promise<{ saved: boolean }> {
    const suffix = leaf ? `?leaf=${encodeURIComponent(leaf)}` : '';
    return this.request(
      `/api/conversations/${encodeURIComponent(id)}/portable-export${suffix}`,
      {
        method: 'POST',
        body: JSON.stringify({}),
      },
    );
  }

  attachmentContent(id: string, signal?: AbortSignal): Promise<Blob> {
    return this.requestBlob(`/api/attachments/${encodeURIComponent(id)}/content`, signal);
  }

  async attachmentText(id: string): Promise<string> {
    const path = `/api/attachments/${encodeURIComponent(id)}/text`;
    assertPrivateApiPath(path);
    const token = this.getToken();
    if (!token) {
      throw new ApiError('AUTH_REQUIRED', 'This local session has expired.', 401);
    }

    const response = await fetch(path, {
      headers: {
        Accept: 'application/json, text/plain',
        Authorization: `Bearer ${token}`,
      },
      cache: 'no-store',
      credentials: 'same-origin',
      redirect: 'error',
      referrerPolicy: 'no-referrer',
    });
    if (!response.ok) throw await safeError(response);

    if (response.headers.get('content-type')?.includes('application/json')) {
      const body = (await response.json()) as { text?: unknown };
      return typeof body.text === 'string' ? body.text : '';
    }
    return response.text();
  }

  saveAttachment(id: string): Promise<{ saved: boolean }> {
    return this.request(`/api/attachments/${encodeURIComponent(id)}/save`, {
      method: 'POST',
      body: JSON.stringify({}),
    });
  }

  private async requestBlob(path: string, signal?: AbortSignal): Promise<Blob> {
    assertPrivateApiPath(path);
    const token = this.getToken();
    if (!token) {
      throw new ApiError('AUTH_REQUIRED', 'This local session has expired.', 401);
    }

    const response = await fetch(path, {
      headers: {
        Accept: 'application/octet-stream',
        Authorization: `Bearer ${token}`,
      },
      cache: 'no-store',
      credentials: 'same-origin',
      redirect: 'error',
      referrerPolicy: 'no-referrer',
      signal,
    });
    if (!response.ok) throw await safeError(response);
    return response.blob();
  }
}

export function unwrapIndexProgress(
  value: IndexProgress | AppStatus | { index: IndexProgress },
): IndexProgress {
  if ('phase' in value) return value;
  return value.index;
}
