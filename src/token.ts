const BASE64URL_TOKEN = /^[A-Za-z0-9_-]{1,4096}$/;
let sessionToken: string | null = null;

interface LocationLike {
  hash: string;
  pathname: string;
  search: string;
}

interface HistoryLike {
  state: unknown;
  replaceState(data: unknown, unused: string, url?: string | URL | null): void;
}

/**
 * Moves the one-time fragment capability into process-local module memory.
 *
 * The fragment is removed before the token is persisted so it cannot linger in
 * address-bar history, screenshots, copied URLs, or subsequent referrers.
 */
export function bootstrapSessionToken(
  locationLike: LocationLike = window.location,
  historyLike: HistoryLike = window.history,
): string | null {
  const params = new URLSearchParams(locationLike.hash.replace(/^#/, ''));
  const fragmentToken = params.get('token');

  if (fragmentToken !== null) {
    historyLike.replaceState(
      historyLike.state,
      '',
      `${locationLike.pathname}${locationLike.search}`,
    );

    if (BASE64URL_TOKEN.test(fragmentToken)) {
      sessionToken = fragmentToken;
    } else {
      sessionToken = null;
    }
  }

  return sessionToken;
}

export function retainSessionToken(token: string | null): string | null {
  sessionToken = token !== null && BASE64URL_TOKEN.test(token) ? token : null;
  return sessionToken;
}

export function getSessionToken(): string | null {
  return sessionToken;
}

export function clearSessionToken(): void {
  sessionToken = null;
}
