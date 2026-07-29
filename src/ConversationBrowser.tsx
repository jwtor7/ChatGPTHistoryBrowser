import { useVirtualizer } from '@tanstack/react-virtual';
import {
  Archive,
  ArrowLeft,
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  Database,
  Download,
  FileStack,
  Filter,
  MessageSquareText,
  Paperclip,
  Search,
  ShieldCheck,
  Star,
  Trash2,
} from 'lucide-react';
import { type FormEvent, type ReactNode, useEffect, useMemo, useRef, useState } from 'react';

import { ApiError, type LocalApi } from './api';
import { AttachmentCard } from './AttachmentCard';
import { SafeMarkdown } from './SafeMarkdown';
import type {
  AppStatus,
  AttachmentView,
  ConversationDetail,
  ConversationFilters,
  ConversationListItem,
  ConversationPage,
  MessageView,
  PortableExportEstimate,
} from './types';

const PAGE_SIZE = 50;
const ATTACHMENT_BATCH_SIZE = 24;

const INITIAL_FILTERS: ConversationFilters = {
  page: 0,
  pageSize: PAGE_SIZE,
  search: '',
  dateFrom: '',
  dateTo: '',
  role: '',
  archived: '',
  starred: '',
  hasAttachments: '',
};

function safeMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : 'The local application could not complete this request.';
}

function formatDate(timestamp: number | null, includeTime = false): string {
  if (timestamp === null || !Number.isFinite(timestamp)) return 'Date unavailable';
  const milliseconds = timestamp > 10_000_000_000 ? timestamp : timestamp * 1_000;
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) return 'Date unavailable';
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    ...(includeTime ? { timeStyle: 'short' as const } : {}),
  }).format(date);
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return 'Size unavailable';
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

function roleDetails(role: string): { className: string; label: string } {
  const normalized = role.toLowerCase();
  if (normalized === 'user') return { className: 'user', label: 'You' };
  if (normalized === 'assistant') {
    return { className: 'assistant', label: 'Assistant' };
  }
  if (normalized === 'system') return { className: 'system', label: 'System' };
  if (normalized === 'tool') return { className: 'tool', label: 'Tool' };
  return { className: 'other', label: role || 'Other' };
}

function ConversationRow({
  item,
  selected,
  onSelect,
}: {
  item: ConversationListItem;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      className={`conversation-row${selected ? ' is-selected' : ''}${
        item.matchPreview ? ' has-match' : ''
      }`}
      onClick={onSelect}
      aria-current={selected ? 'true' : undefined}
    >
      <span className="conversation-row-heading">
        <strong>{item.title || 'Untitled conversation'}</strong>
        {item.starred ? <Star size={14} aria-label="Starred" /> : null}
      </span>
      <span className="conversation-row-meta">
        <span>{formatDate(item.updatedAt ?? item.createdAt)}</span>
        <span>{item.messageCount} messages</span>
        {item.hasAttachments ? <Paperclip size={13} aria-label="Has attachments" /> : null}
      </span>
      {item.matchPreview ? (
        <span className="conversation-row-match">{item.matchPreview}</span>
      ) : null}
    </button>
  );
}

function ConversationList({
  page,
  selectedId,
  onSelect,
}: {
  page: ConversationPage;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // TanStack Virtual intentionally returns imperative functions used only here.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: page.items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 82,
    getItemKey: (index) => page.items[index]?.id ?? index,
    initialRect: { width: 360, height: 620 },
    overscan: 6,
  });

  return (
    <div ref={scrollRef} className="conversation-list" role="list" aria-label="Conversations">
      <div className="conversation-list-spacer" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const item = page.items[virtualRow.index];
          if (!item) return null;
          return (
            <div
              key={item.id}
              ref={virtualizer.measureElement}
              data-index={virtualRow.index}
              className="conversation-list-position"
              style={{ transform: `translateY(${virtualRow.start}px)` }}
              role="listitem"
            >
              <ConversationRow
                item={item}
                selected={selectedId === item.id}
                onSelect={() => onSelect(item.id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function BoundedAttachmentList({
  api,
  attachments,
}: {
  api: LocalApi;
  attachments: AttachmentView[];
}) {
  const [visibleAttachmentCount, setVisibleAttachmentCount] = useState(ATTACHMENT_BATCH_SIZE);
  const [activePreviewId, setActivePreviewId] = useState<string | null>(null);
  const visibleAttachments = attachments.slice(0, visibleAttachmentCount);

  return (
    <div className="message-attachments" aria-label="Message attachments">
      {visibleAttachments.map((attachment) => (
        <AttachmentCard
          key={attachment.id}
          api={api}
          attachment={attachment}
          previewActive={activePreviewId === attachment.id}
          onPreviewActivate={setActivePreviewId}
        />
      ))}
      {visibleAttachmentCount < attachments.length ? (
        <button
          type="button"
          className="button button-quiet button-small"
          onClick={() =>
            setVisibleAttachmentCount((count) =>
              Math.min(count + ATTACHMENT_BATCH_SIZE, attachments.length),
            )
          }
        >
          Show {Math.min(ATTACHMENT_BATCH_SIZE, attachments.length - visibleAttachmentCount)}{' '}
          more attachments
        </button>
      ) : null}
    </div>
  );
}

function Message({
  api,
  message,
  onBranch,
}: {
  api: LocalApi;
  message: MessageView;
  onBranch: (leaf: string) => void;
}) {
  const role = roleDetails(message.role);
  return (
    <article
      className={`message message-${role.className}`}
      aria-label={`${role.label} message`}
    >
      <header className="message-header">
        <span className="message-role">{role.label}</span>
        <span>{formatDate(message.createdAt, true)}</span>
      </header>
      <div className="message-content">
        <SafeMarkdown>{message.text}</SafeMarkdown>
      </div>

      {message.attachments.length > 0 ? (
        <BoundedAttachmentList api={api} attachments={message.attachments} />
      ) : null}

      {message.alternateBranches.length > 0 ? (
        <div className="branches">
          <p>Other branches from here</p>
          <div className="branch-actions">
            {message.alternateBranches.map((branch, index) => (
              <button
                key={branch.leafNodeId}
                type="button"
                className="branch-button"
                onClick={() => onBranch(branch.leafNodeId)}
              >
                <span>Branch {index + 1}</span>
                <small>
                  {roleDetails(branch.role).label}: {branch.preview}
                </small>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </article>
  );
}

function FilterSelect({
  label,
  value,
  onChange,
  children,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {children}
      </select>
    </label>
  );
}

interface ConversationBrowserProps {
  api: LocalApi;
  status: AppStatus;
  onRebuild: () => Promise<void>;
  onDiscard: () => Promise<void>;
}

export function ConversationBrowser({
  api,
  status,
  onRebuild,
  onDiscard,
}: ConversationBrowserProps) {
  const [filters, setFilters] = useState(INITIAL_FILTERS);
  const [searchDraft, setSearchDraft] = useState('');
  const [page, setPage] = useState<ConversationPage | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [listLoading, setListLoading] = useState(true);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<ConversationDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [mobileDetail, setMobileDetail] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmExport, setConfirmExport] = useState(false);
  const [exportEstimate, setExportEstimate] = useState<PortableExportEstimate | null>(null);
  const [exportTarget, setExportTarget] = useState<{
    id: string;
    leaf?: string;
    title: string;
  } | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [listRequestVersion, setListRequestVersion] = useState(0);
  const [detailRequestVersion, setDetailRequestVersion] = useState(0);
  const shellRef = useRef<HTMLDivElement>(null);
  const discardTriggerRef = useRef<HTMLButtonElement>(null);
  const discardDialogRef = useRef<HTMLElement>(null);
  const keepIndexRef = useRef<HTMLButtonElement>(null);
  const exportTriggerRef = useRef<HTMLButtonElement>(null);
  const exportDialogRef = useRef<HTMLElement>(null);
  const cancelExportRef = useRef<HTMLButtonElement>(null);
  const actionBusyRef = useRef(actionBusy);
  const exportBusyRef = useRef(exportBusy);

  useEffect(() => {
    actionBusyRef.current = actionBusy;
  }, [actionBusy]);

  useEffect(() => {
    exportBusyRef.current = exportBusy;
  }, [exportBusy]);

  useEffect(() => {
    let active = true;

    void api
      .conversations(filters)
      .then((nextPage) => {
        if (!active) return;
        setPage(nextPage);
        setSelectedId((current) => {
          if (current && nextPage.items.some((item) => item.id === current)) {
            return current;
          }
          if (nextPage.items[0]) {
            setDetailLoading(true);
            setDetailError(null);
          }
          return nextPage.items[0]?.id ?? null;
        });
      })
      .catch((error: unknown) => {
        if (active) {
          setPage(null);
          setSelectedId(null);
          setDetail(null);
          setListError(safeMessage(error));
        }
      })
      .finally(() => {
        if (active) setListLoading(false);
      });

    return () => {
      active = false;
    };
  }, [api, filters, listRequestVersion]);

  useEffect(() => {
    if (!selectedId) return;

    let active = true;

    void api
      .conversation(selectedId)
      .then((nextDetail) => {
        if (active) setDetail(nextDetail);
      })
      .catch((error: unknown) => {
        if (active) {
          setDetail(null);
          setDetailError(safeMessage(error));
        }
      })
      .finally(() => {
        if (active) setDetailLoading(false);
      });

    return () => {
      active = false;
    };
  }, [api, detailRequestVersion, selectedId]);

  useEffect(() => {
    if (!confirmDiscard) return;

    const trigger = discardTriggerRef.current;
    const shell = shellRef.current;
    if (shell) shell.inert = true;
    const focusFrame = window.requestAnimationFrame(() => keepIndexRef.current?.focus());

    function handleDialogKeydown(event: KeyboardEvent) {
      if (event.key === 'Escape' && !actionBusyRef.current) {
        event.preventDefault();
        setConfirmDiscard(false);
        return;
      }
      if (event.key !== 'Tab') return;

      const focusable = Array.from(
        discardDialogRef.current?.querySelectorAll<HTMLButtonElement>(
          'button:not([disabled])',
        ) ?? [],
      );
      const first = focusable[0];
      const last = focusable.at(-1);
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener('keydown', handleDialogKeydown);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      document.removeEventListener('keydown', handleDialogKeydown);
      if (shell) shell.inert = false;
      trigger?.focus();
    };
  }, [confirmDiscard]);

  useEffect(() => {
    if (!confirmExport) return;

    const trigger = exportTriggerRef.current;
    const shell = shellRef.current;
    if (shell) shell.inert = true;
    const focusFrame = window.requestAnimationFrame(() => cancelExportRef.current?.focus());

    function handleDialogKeydown(event: KeyboardEvent) {
      if (event.key === 'Escape' && !exportBusyRef.current) {
        event.preventDefault();
        setConfirmExport(false);
        setExportStatus('Export cancelled. No file was created.');
        return;
      }
      if (event.key !== 'Tab') return;

      const focusable = Array.from(
        exportDialogRef.current?.querySelectorAll<HTMLButtonElement>(
          'button:not([disabled])',
        ) ?? [],
      );
      const first = focusable[0];
      const last = focusable.at(-1);
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener('keydown', handleDialogKeydown);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      document.removeEventListener('keydown', handleDialogKeydown);
      if (shell) shell.inert = false;
      trigger?.focus();
    };
  }, [confirmExport]);

  const totalPages = useMemo(
    () => (page ? Math.max(1, Math.ceil(page.total / page.pageSize)) : 1),
    [page],
  );

  function updateFilter<Key extends keyof ConversationFilters>(
    key: Key,
    value: ConversationFilters[Key],
  ) {
    setListLoading(true);
    setListError(null);
    setPage(null);
    setSelectedId(null);
    setDetail(null);
    setDetailError(null);
    setFilters((current) => ({ ...current, [key]: value, page: 0 }));
  }

  function retryList() {
    setListLoading(true);
    setListError(null);
    setPage(null);
    setListRequestVersion((version) => version + 1);
  }

  function retryDetail() {
    setDetailLoading(true);
    setDetailError(null);
    setDetail(null);
    setDetailRequestVersion((version) => version + 1);
  }

  function submitSearch(event: FormEvent) {
    event.preventDefault();
    updateFilter('search', searchDraft.trim());
  }

  function chooseConversation(id: string) {
    if (id !== selectedId) {
      setDetailLoading(true);
      setDetailError(null);
      setDetail(null);
      setSelectedId(id);
    }
    setMobileDetail(true);
  }

  async function chooseBranch(leaf: string) {
    if (!selectedId) return;
    setDetailLoading(true);
    setDetailError(null);
    setDetail(null);
    try {
      setDetail(await api.conversation(selectedId, leaf));
    } catch (error) {
      setDetailError(safeMessage(error));
    } finally {
      setDetailLoading(false);
    }
  }

  async function rebuild() {
    setActionBusy(true);
    try {
      await onRebuild();
    } finally {
      setActionBusy(false);
    }
  }

  async function discard() {
    setActionBusy(true);
    try {
      await onDiscard();
      setConfirmDiscard(false);
    } finally {
      setActionBusy(false);
    }
  }

  async function preparePortableExport() {
    if (!detail) return;
    const target = {
      id: detail.id,
      leaf: detail.selectedLeaf ?? undefined,
      title: detail.title || 'Untitled conversation',
    };
    setExportBusy(true);
    setExportError(null);
    setExportStatus(null);
    try {
      const estimate = await api.portableExportEstimate(target.id, target.leaf);
      setExportEstimate(estimate);
      setExportTarget(target);
      setConfirmExport(true);
    } catch (error) {
      setExportError(safeMessage(error));
    } finally {
      setExportBusy(false);
    }
  }

  function cancelPortableExport() {
    setConfirmExport(false);
    setExportStatus('Export cancelled. No file was created.');
  }

  async function savePortableExport() {
    if (!exportTarget) return;
    setExportBusy(true);
    setExportError(null);
    try {
      const result = await api.savePortableExport(exportTarget.id, exportTarget.leaf);
      setConfirmExport(false);
      setExportStatus(
        result.saved
          ? 'Portable context package saved.'
          : 'Export cancelled. No file was created.',
      );
    } catch (error) {
      setExportError(safeMessage(error));
    } finally {
      setExportBusy(false);
    }
  }

  return (
    <>
      <div
        ref={shellRef}
        className={`browser-shell${mobileDetail ? ' show-detail' : ''}`}
        aria-busy={listLoading || detailLoading}
      >
        <header className="app-header">
          <div className="brand-lockup">
            <span className="brand-mark" aria-hidden="true">
              <MessageSquareText size={20} />
            </span>
            <span>
              <strong>History Browser</strong>
              <small>Local archive</small>
            </span>
          </div>
          <div className="header-status" aria-label="Archive status">
            <span>
              <Database size={15} aria-hidden="true" />
              {status.index.conversationsIndexed.toLocaleString()} conversations
            </span>
            <span>
              <FileStack size={15} aria-hidden="true" />
              {status.shardCount} shards
            </span>
            <span className="private-status">
              <ShieldCheck size={15} aria-hidden="true" />
              Local only
            </span>
          </div>
          <div className="header-actions">
            <button
              type="button"
              className="button button-quiet button-small"
              onClick={() => void rebuild()}
              disabled={actionBusy}
            >
              Rebuild index
            </button>
            <button
              ref={discardTriggerRef}
              type="button"
              className="icon-button"
              onClick={() => setConfirmDiscard(true)}
              aria-label="Delete local index"
              title="Delete local index"
            >
              <Trash2 size={17} aria-hidden="true" />
            </button>
          </div>
        </header>

        <aside className="browser-sidebar" aria-label="Conversation browser">
          <div className="sidebar-tools">
            <form className="search-form" role="search" onSubmit={submitSearch}>
              <label className="search-field">
                <Search size={17} aria-hidden="true" />
                <span className="sr-only">Search conversations</span>
                <input
                  type="search"
                  placeholder="Search conversations"
                  value={searchDraft}
                  onChange={(event) => setSearchDraft(event.target.value)}
                />
              </label>
              <button type="submit" className="button button-primary button-small">
                Search
              </button>
            </form>

            <details className="filter-panel">
              <summary>
                <Filter size={15} aria-hidden="true" />
                Filters
                {Object.entries(filters).some(
                  ([key, value]) =>
                    !['page', 'pageSize', 'search'].includes(key) && value !== '',
                ) ? (
                  <span className="filter-dot" aria-label="Filters active" />
                ) : null}
              </summary>
              <div className="filter-grid">
                <label className="field">
                  <span>From</span>
                  <input
                    type="date"
                    value={filters.dateFrom}
                    onChange={(event) => updateFilter('dateFrom', event.target.value)}
                  />
                </label>
                <label className="field">
                  <span>To</span>
                  <input
                    type="date"
                    value={filters.dateTo}
                    onChange={(event) => updateFilter('dateTo', event.target.value)}
                  />
                </label>
                <FilterSelect
                  label="Role"
                  value={filters.role}
                  onChange={(value) => updateFilter('role', value)}
                >
                  <option value="">Any role</option>
                  <option value="user">You</option>
                  <option value="assistant">Assistant</option>
                  <option value="system">System</option>
                  <option value="tool">Tool</option>
                </FilterSelect>
                <FilterSelect
                  label="Archived"
                  value={filters.archived}
                  onChange={(value) =>
                    updateFilter('archived', value as ConversationFilters['archived'])
                  }
                >
                  <option value="">Any</option>
                  <option value="true">Archived</option>
                  <option value="false">Not archived</option>
                </FilterSelect>
                <FilterSelect
                  label="Starred"
                  value={filters.starred}
                  onChange={(value) =>
                    updateFilter('starred', value as ConversationFilters['starred'])
                  }
                >
                  <option value="">Any</option>
                  <option value="true">Starred</option>
                  <option value="false">Not starred</option>
                </FilterSelect>
                <FilterSelect
                  label="Attachments"
                  value={filters.hasAttachments}
                  onChange={(value) =>
                    updateFilter(
                      'hasAttachments',
                      value as ConversationFilters['hasAttachments'],
                    )
                  }
                >
                  <option value="">Any</option>
                  <option value="true">Has attachments</option>
                  <option value="false">No attachments</option>
                </FilterSelect>
              </div>
              <button
                type="button"
                className="button button-quiet button-small"
                onClick={() => {
                  setListLoading(true);
                  setListError(null);
                  setPage(null);
                  setSelectedId(null);
                  setDetail(null);
                  setDetailError(null);
                  setFilters(INITIAL_FILTERS);
                  setSearchDraft('');
                  setListRequestVersion((version) => version + 1);
                }}
              >
                Clear filters
              </button>
            </details>
          </div>

          <div className="result-summary" aria-live="polite">
            {listLoading
              ? 'Loading conversations…'
              : `${page?.total.toLocaleString() ?? 0} conversations`}
          </div>

          {listError ? (
            <div className="inline-error" role="alert">
              <p>{listError}</p>
              <button
                type="button"
                className="button button-quiet button-small"
                onClick={retryList}
              >
                Try again
              </button>
            </div>
          ) : null}
          {!listLoading && page?.items.length === 0 ? (
            <div className="empty-list">
              <Search size={24} aria-hidden="true" />
              <p>No conversations match these filters.</p>
            </div>
          ) : null}
          {page && page.items.length > 0 ? (
            <ConversationList
              page={page}
              selectedId={selectedId}
              onSelect={chooseConversation}
            />
          ) : null}

          <nav className="pagination" aria-label="Conversation pages">
            <button
              type="button"
              className="icon-button"
              disabled={filters.page <= 0 || listLoading}
              onClick={() => {
                setListLoading(true);
                setListError(null);
                setPage(null);
                setSelectedId(null);
                setDetail(null);
                setDetailError(null);
                setFilters((current) => ({
                  ...current,
                  page: Math.max(0, current.page - 1),
                }));
              }}
              aria-label="Previous page"
            >
              <ChevronLeft size={17} aria-hidden="true" />
            </button>
            <span>
              Page {filters.page + 1} of {totalPages}
            </span>
            <button
              type="button"
              className="icon-button"
              disabled={!page?.hasMore || listLoading}
              onClick={() => {
                setListLoading(true);
                setListError(null);
                setPage(null);
                setSelectedId(null);
                setDetail(null);
                setDetailError(null);
                setFilters((current) => ({
                  ...current,
                  page: current.page + 1,
                }));
              }}
              aria-label="Next page"
            >
              <ChevronRight size={17} aria-hidden="true" />
            </button>
          </nav>
        </aside>

        <main
          className="conversation-pane"
          aria-label="Selected conversation"
          aria-busy={detailLoading}
        >
          <button
            type="button"
            className="mobile-back button button-quiet button-small"
            onClick={() => setMobileDetail(false)}
          >
            <ArrowLeft size={16} aria-hidden="true" />
            Conversations
          </button>

          {detailLoading ? (
            <div className="detail-state" role="status">
              Loading conversation…
            </div>
          ) : null}
          {detailError ? (
            <div className="detail-state inline-error" role="alert">
              <p>{detailError}</p>
              <button
                type="button"
                className="button button-quiet button-small"
                onClick={retryDetail}
              >
                Try again
              </button>
            </div>
          ) : null}
          {!selectedId && !detailLoading && !detailError ? (
            <div className="detail-state">
              <MessageSquareText size={28} aria-hidden="true" />
              <p>Select a conversation to read its active path.</p>
            </div>
          ) : null}
          {detail && !detailLoading && !detailError ? (
            <>
              <header className="conversation-heading">
                <p>Active conversation path</p>
                <h1>{detail.title || 'Untitled conversation'}</h1>
                <div className="conversation-heading-meta">
                  <span>
                    <CalendarDays size={14} aria-hidden="true" />
                    Updated {formatDate(detail.updatedAt ?? detail.createdAt)}
                  </span>
                  {detail.archived ? (
                    <span>
                      <Archive size={14} aria-hidden="true" />
                      Archived
                    </span>
                  ) : null}
                  {detail.starred ? (
                    <span>
                      <Star size={14} aria-hidden="true" />
                      Starred
                    </span>
                  ) : null}
                </div>
                <div className="conversation-export-actions">
                  <button
                    ref={exportTriggerRef}
                    type="button"
                    className="button button-quiet button-small"
                    onClick={() => void preparePortableExport()}
                    disabled={exportBusy}
                  >
                    <Download size={15} aria-hidden="true" />
                    {exportBusy && !confirmExport ? 'Preparing…' : 'Export active path'}
                  </button>
                  {exportStatus ? (
                    <span className="export-status" role="status">
                      {exportStatus}
                    </span>
                  ) : null}
                  {exportError && !confirmExport ? (
                    <span className="export-error" role="alert">
                      {exportError}
                    </span>
                  ) : null}
                </div>
                {detail.diagnostics.length > 0 ? (
                  <p className="diagnostic-note">
                    This path has{' '}
                    {detail.diagnostics.reduce((sum, item) => sum + item.count, 0)}{' '}
                    compatibility warning
                    {detail.diagnostics.reduce((sum, item) => sum + item.count, 0) === 1
                      ? ''
                      : 's'}
                    .
                  </p>
                ) : null}
              </header>
              <div className="message-stream">
                {detail.messages.map((message) => (
                  <Message
                    key={message.nodeId}
                    api={api}
                    message={message}
                    onBranch={(leaf) => void chooseBranch(leaf)}
                  />
                ))}
              </div>
            </>
          ) : null}
        </main>
      </div>

      {confirmDiscard ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !actionBusyRef.current) {
              setConfirmDiscard(false);
            }
          }}
        >
          <section
            ref={discardDialogRef}
            className="confirm-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="discard-title"
            aria-describedby="discard-description"
          >
            <Database size={24} aria-hidden="true" />
            <h2 id="discard-title">Delete the local index?</h2>
            <p id="discard-description">
              This removes the disposable search index and generated previews. Your source
              export remains unchanged.
            </p>
            <div className="dialog-actions">
              <button
                ref={keepIndexRef}
                type="button"
                className="button button-quiet"
                onClick={() => setConfirmDiscard(false)}
                disabled={actionBusy}
              >
                Keep index
              </button>
              <button
                type="button"
                className="button button-danger"
                onClick={() => void discard()}
                disabled={actionBusy}
              >
                {actionBusy ? 'Deleting…' : 'Delete local index'}
              </button>
            </div>
          </section>
        </div>
      ) : null}
      {confirmExport && exportEstimate && exportTarget ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !exportBusyRef.current) {
              cancelPortableExport();
            }
          }}
        >
          <section
            ref={exportDialogRef}
            className="confirm-dialog export-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="export-title"
            aria-describedby="export-description"
          >
            <Download size={24} aria-hidden="true" />
            <h2 id="export-title">Export this active path?</h2>
            <p id="export-description">
              This creates a plaintext, provider-neutral JSON package with readable Markdown.
              The confirmed package is bound to <strong>{exportTarget.title}</strong>. Saving it
              does not upload anything, but sharing or importing it transfers the selected
              private conversation under the destination provider&apos;s policies.
            </p>
            <dl className="export-estimate">
              <div>
                <dt>Conversations</dt>
                <dd>{exportEstimate.conversationCount}</dd>
              </div>
              <div>
                <dt>Messages</dt>
                <dd>{exportEstimate.messageCount.toLocaleString()}</dd>
              </div>
              <div>
                <dt>Estimated size</dt>
                <dd>{formatBytes(exportEstimate.byteSize)}</dd>
              </div>
              <div>
                <dt>Attachments</dt>
                <dd>
                  {exportEstimate.attachmentCount.toLocaleString()} detected, none included
                </dd>
              </div>
            </dl>
            {exportError ? (
              <p className="export-dialog-error" role="alert">
                {exportError}
              </p>
            ) : null}
            <div className="dialog-actions">
              <button
                ref={cancelExportRef}
                type="button"
                className="button button-quiet"
                onClick={cancelPortableExport}
                disabled={exportBusy}
              >
                Cancel
              </button>
              <button
                type="button"
                className="button button-primary"
                onClick={() => void savePortableExport()}
                disabled={exportBusy}
              >
                {exportBusy ? 'Opening save dialog…' : 'Choose save location'}
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
