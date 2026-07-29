import {
  AlertCircle,
  FolderOpen,
  LoaderCircle,
  LockKeyhole,
  RefreshCcw,
  ShieldCheck,
  Square,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { ApiError, LocalApi, unwrapIndexProgress } from './api';
import { BrandArtwork, BrandMark } from './BrandMark';
import { ConversationBrowser } from './ConversationBrowser';
import {
  bootstrapSessionToken,
  clearSessionToken,
  getSessionToken,
  retainSessionToken,
} from './token';
import type { AppStatus, ExportValidation, IndexProgress } from './types';

function safeMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : 'The local application could not complete this request.';
}

function isIndexActive(index: IndexProgress): boolean {
  return ['discovering', 'indexing', 'cancelling'].includes(index.phase);
}

function percentage(index: IndexProgress): number {
  if (index.bytesTotal > 0) {
    return Math.min(100, Math.round((index.bytesProcessed / index.bytesTotal) * 100));
  }
  if (index.shardsTotal > 0) {
    return Math.min(100, Math.round((index.shardsComplete / index.shardsTotal) * 100));
  }
  return 0;
}

function indexFailureMessage(code: string | null): string {
  switch (code) {
    case 'PATH_REJECTED':
      return 'The export changed or no longer passes the local safety checks. Choose it again before retrying.';
    case 'MALFORMED_JSON':
    case 'INVALID_EXPORT':
      return 'A conversation shard could not be read safely. Re-extract the export or choose a different copy.';
    case 'RESOURCE_LIMIT':
    case 'RECORD_TOO_LARGE':
      return 'The export reached a local safety limit. Choose another export or reduce local storage pressure.';
    case 'INDEX_UNAVAILABLE':
    case 'INTERNAL':
      return 'The local index could not be written. Check available storage, then try again.';
    default:
      return 'The index could not be completed safely. Try again or choose a different export.';
  }
}

function SessionUnavailable() {
  return (
    <main className="state-page">
      <section className="state-panel" aria-labelledby="session-title">
        <span className="state-symbol">
          <LockKeyhole size={27} aria-hidden="true" />
        </span>
        <p className="eyebrow">Private local session</p>
        <h1 id="session-title">Restart the application to continue</h1>
        <p>
          This window does not have a valid session capability. Close it and open History
          Browser again from the desktop application.
        </p>
      </section>
    </main>
  );
}

function LoadingApp() {
  return (
    <main className="state-page" aria-label="Starting History Browser">
      <div className="loading-lockup" role="status">
        <BrandMark className="brand-mark-large brand-mark-loading" />
        <span>
          <strong>History Browser</strong>
          <small>Starting the private local service…</small>
        </span>
      </div>
    </main>
  );
}

function Onboarding({
  selecting,
  error,
  onSelect,
}: {
  selecting: boolean;
  error: string | null;
  onSelect: () => Promise<void>;
}) {
  return (
    <main className="onboarding">
      <section className="onboarding-copy" aria-labelledby="onboarding-title">
        <div className="brand-lockup onboarding-brand">
          <BrandMark />
          <span>
            <strong>History Browser</strong>
            <small>Private by design</small>
          </span>
        </div>

        <p className="eyebrow">Your archive, readable again</p>
        <h1 id="onboarding-title">Browse your ChatGPT export without uploading it.</h1>
        <p className="onboarding-intro">
          Choose an extracted official export. History Browser builds a private, disposable
          search index on this device.
        </p>

        <button
          type="button"
          className="button button-primary button-large"
          onClick={() => void onSelect()}
          disabled={selecting}
        >
          {selecting ? (
            <LoaderCircle className="spin" size={18} aria-hidden="true" />
          ) : (
            <FolderOpen size={18} aria-hidden="true" />
          )}
          {selecting ? 'Checking folder…' : 'Choose extracted export'}
        </button>
        <p className="onboarding-hint">
          Choose the extracted folder containing <code>conversations.json</code> or numbered
          conversation shards—not the ZIP file or <code>chat.html</code>.
        </p>
        {error ? (
          <p className="onboarding-error" role="alert">
            <AlertCircle size={17} aria-hidden="true" />
            {error}
          </p>
        ) : null}
      </section>

      <aside className="trust-rail" aria-label="Privacy details">
        <BrandArtwork className="trust-artwork" />
        <div className="trust-rule" />
        <article>
          <span>01</span>
          <div>
            <h2>Local only</h2>
            <p>
              Conversation content stays on this device. The app has no cloud sync, analytics,
              or remote search.
            </p>
          </div>
        </article>
        <article>
          <span>02</span>
          <div>
            <h2>Source stays read-only</h2>
            <p>
              The selected export is never edited. Search data is stored separately and can be
              deleted at any time.
            </p>
          </div>
        </article>
        <article>
          <span>03</span>
          <div>
            <h2>Private data still needs care</h2>
            <p>
              The local index can contain personal information. No tool can guarantee universal
              detection or removal of it.
            </p>
          </div>
        </article>
      </aside>
    </main>
  );
}

function IndexWorkspace({
  status,
  busy,
  error,
  onStart,
  onCancel,
  onChoose,
}: {
  status: AppStatus;
  busy: boolean;
  error: string | null;
  onStart: () => Promise<void>;
  onCancel: () => Promise<void>;
  onChoose: () => Promise<void>;
}) {
  const active = isIndexActive(status.index);
  const progress = percentage(status.index);
  const phaseLabel: Record<IndexProgress['phase'], string> = {
    idle: 'Ready to index',
    discovering: 'Discovering conversation shards',
    indexing: 'Building the local search index',
    cancelling: 'Stopping safely',
    complete: 'Index complete',
    cancelled: 'Indexing stopped',
    failed: 'Indexing needs attention',
  };

  return (
    <main className="state-page index-page">
      <section className="index-panel" aria-labelledby="index-title">
        <header className="brand-lockup index-brand">
          <BrandMark />
          <span>
            <strong>History Browser</strong>
            <small>Local archive</small>
          </span>
        </header>

        <div className="index-heading">
          <p className="eyebrow">Selected export</p>
          <h1 id="index-title">{phaseLabel[status.index.phase]}</h1>
          <p>
            {status.shardCount.toLocaleString()} conversation shard
            {status.shardCount === 1 ? '' : 's'} and{' '}
            {status.attachmentFileCount.toLocaleString()} attachment candidate
            {status.attachmentFileCount === 1 ? '' : 's'} found.
          </p>
        </div>

        {active || status.index.phase === 'cancelled' ? (
          <div className="progress-block">
            <div className="progress-meta">
              <span>
                {status.index.shardsComplete} of {status.index.shardsTotal} shards
              </span>
              <span>{progress}%</span>
            </div>
            <div
              className="progress-track"
              role="progressbar"
              aria-label="Indexing progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progress}
            >
              <span style={{ width: `${progress}%` }} />
            </div>
            <div className="index-counters" aria-live="polite">
              <span>
                <strong>{status.index.conversationsIndexed.toLocaleString()}</strong>
                conversations indexed
              </span>
              <span>
                <strong>{status.index.conversationsSkipped.toLocaleString()}</strong>
                records skipped
              </span>
              <span>
                <strong>{status.index.diagnostics.toLocaleString()}</strong>
                compatibility warnings
              </span>
            </div>
          </div>
        ) : null}

        <div className="index-actions">
          {active ? (
            <button
              type="button"
              className="button button-quiet"
              onClick={() => void onCancel()}
              disabled={busy || status.index.phase === 'cancelling'}
            >
              <Square size={15} fill="currentColor" aria-hidden="true" />
              {status.index.phase === 'cancelling' ? 'Stopping…' : 'Stop indexing'}
            </button>
          ) : (
            <button
              type="button"
              className="button button-primary"
              onClick={() => void onStart()}
              disabled={busy}
            >
              {busy ? (
                <LoaderCircle className="spin" size={17} aria-hidden="true" />
              ) : (
                <RefreshCcw size={17} aria-hidden="true" />
              )}
              {status.index.phase === 'idle'
                ? 'Build local index'
                : status.index.phase === 'failed'
                  ? 'Try indexing again'
                  : 'Resume indexing'}
            </button>
          )}
          {!active ? (
            <button
              type="button"
              className="button button-quiet"
              onClick={() => void onChoose()}
              disabled={busy}
            >
              Choose a different export
            </button>
          ) : null}
        </div>
        {status.index.phase === 'failed' ? (
          <p className="inline-error index-error" role="alert">
            {indexFailureMessage(status.index.failureCode)}
          </p>
        ) : null}
        {error ? (
          <p className="inline-error index-error" role="alert">
            {error}
          </p>
        ) : null}

        <footer className="index-footnote">
          <ShieldCheck size={16} aria-hidden="true" />
          Your source export remains read-only. Incomplete work never replaces the last usable
          index.
        </footer>
      </section>
    </main>
  );
}

export function App({ initialToken }: { initialToken?: string | null }) {
  const [token] = useState<string | null>(() =>
    initialToken === undefined ? bootstrapSessionToken() : retainSessionToken(initialToken),
  );
  const [api] = useState(() => new LocalApi(getSessionToken));
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [loading, setLoading] = useState(token !== null);
  const [selecting, setSelecting] = useState(false);
  const [actionBusy, setActionBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [validation, setValidation] = useState<ExportValidation | null>(null);
  const [sessionUnavailable, setSessionUnavailable] = useState(token === null);

  const refreshStatus = useCallback(async () => {
    try {
      const nextStatus = await api.status();
      setStatus(nextStatus);
      setError(null);
      return nextStatus;
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 401) {
        clearSessionToken();
        setSessionUnavailable(true);
      }
      setError(safeMessage(caught));
      return null;
    } finally {
      setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    if (!token) return;
    // This effect synchronizes the view with the external loopback service.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshStatus();
  }, [refreshStatus, token]);

  useEffect(() => {
    if (!status || !isIndexActive(status.index)) return;

    const timer = window.setInterval(() => {
      void api
        .indexStatus()
        .then((index) => {
          setStatus((current) => (current ? { ...current, index } : current));
          if (index.phase === 'complete') void refreshStatus();
        })
        .catch((caught: unknown) => setError(safeMessage(caught)));
    }, 900);

    return () => window.clearInterval(timer);
  }, [api, refreshStatus, status]);

  async function selectExport() {
    setSelecting(true);
    setActionBusy(true);
    setError(null);
    try {
      const picked = await api.pickExport();
      if (!picked) return;

      setValidation(picked);
      await refreshStatus();
    } catch (caught) {
      setError(safeMessage(caught));
    } finally {
      setSelecting(false);
      setActionBusy(false);
    }
  }

  async function startIndex() {
    setActionBusy(true);
    setError(null);
    try {
      const result = await api.startIndex();
      const index = unwrapIndexProgress(result);
      setStatus((current) => (current ? { ...current, index } : current));
    } catch (caught) {
      setError(safeMessage(caught));
    } finally {
      setActionBusy(false);
    }
  }

  async function cancelIndex() {
    setActionBusy(true);
    setError(null);
    try {
      const result = await api.cancelIndex();
      const index = unwrapIndexProgress(result);
      setStatus((current) => (current ? { ...current, index } : current));
    } catch (caught) {
      setError(safeMessage(caught));
    } finally {
      setActionBusy(false);
    }
  }

  async function discardIndex() {
    setActionBusy(true);
    setError(null);
    try {
      const result = await api.discardIndex();
      if ('exportSelected' in result) {
        setStatus(result);
      } else {
        const index = unwrapIndexProgress(result);
        setStatus((current) => (current ? { ...current, index } : current));
      }
      await refreshStatus();
    } catch (caught) {
      setError(safeMessage(caught));
    } finally {
      setActionBusy(false);
    }
  }

  if (sessionUnavailable || !token) return <SessionUnavailable />;
  if (loading && !status) return <LoadingApp />;
  if (!status) {
    return (
      <main className="state-page">
        <section className="state-panel" role="alert">
          <AlertCircle className="state-alert" size={26} aria-hidden="true" />
          <h1>History Browser could not start</h1>
          <p>{error ?? 'Restart the application and try again.'}</p>
          <button
            type="button"
            className="button button-primary"
            onClick={() => void refreshStatus()}
          >
            Try again
          </button>
        </section>
      </main>
    );
  }
  if (!status.exportSelected) {
    return <Onboarding selecting={selecting} error={error} onSelect={selectExport} />;
  }
  if (status.index.phase !== 'complete') {
    return (
      <IndexWorkspace
        status={status}
        busy={actionBusy}
        error={error}
        onStart={startIndex}
        onCancel={cancelIndex}
        onChoose={selectExport}
      />
    );
  }

  return (
    <>
      {(validation && !validation.supported) || error ? (
        <div className="global-warning-stack" aria-label="Application alerts">
          {validation && !validation.supported ? (
            <div className="global-warning" role="alert">
              The selected export reported compatibility limitations.
            </div>
          ) : null}
          {error ? (
            <div className="global-warning" role="alert">
              {error}
            </div>
          ) : null}
        </div>
      ) : null}
      <ConversationBrowser
        api={api}
        status={status}
        onRebuild={startIndex}
        onDiscard={discardIndex}
      />
    </>
  );
}
