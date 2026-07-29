export type IndexPhase =
  'idle' | 'discovering' | 'indexing' | 'cancelling' | 'complete' | 'cancelled' | 'failed';

export interface IndexProgress {
  phase: IndexPhase;
  failureCode: string | null;
  shardsTotal: number;
  shardsComplete: number;
  bytesTotal: number;
  bytesProcessed: number;
  conversationsIndexed: number;
  conversationsSkipped: number;
  diagnostics: number;
}

export interface AppStatus {
  exportSelected: boolean;
  shardCount: number;
  attachmentFileCount: number;
  index: IndexProgress;
}

export interface ExportValidation {
  supported: boolean;
  shardCount: number;
  attachmentFileCount: number;
  totalJsonBytes: number;
}

export interface ConversationListItem {
  id: string;
  title: string;
  createdAt: number | null;
  updatedAt: number | null;
  archived: boolean | null;
  starred: boolean | null;
  hasAttachments: boolean;
  messageCount: number;
  matchPreview: string | null;
}

export interface ConversationPage {
  items: ConversationListItem[];
  page: number;
  pageSize: number;
  total: number;
  hasMore: boolean;
}

export type PreviewKind =
  'image' | 'audio' | 'video' | 'pdf' | 'text' | 'unsupported' | 'missing';

export type AttachmentStatus = 'available' | 'missing' | 'rejected';

export interface AttachmentView {
  id: string;
  displayName: string;
  claimedMime: string | null;
  detectedMime: string | null;
  byteSize: number | null;
  status: AttachmentStatus;
  previewKind: PreviewKind;
}

export interface BranchView {
  leafNodeId: string;
  role: string;
  preview: string;
}

export interface MessageView {
  nodeId: string;
  role: string;
  createdAt: number | null;
  contentType: string;
  text: string;
  attachments: AttachmentView[];
  alternateBranches: BranchView[];
}

export interface DiagnosticView {
  code: string;
  count: number;
}

export interface ConversationDetail {
  id: string;
  title: string;
  createdAt: number | null;
  updatedAt: number | null;
  archived: boolean | null;
  starred: boolean | null;
  selectedLeaf: string | null;
  messages: MessageView[];
  diagnostics: DiagnosticView[];
}

export interface PortableExportEstimate {
  conversationCount: number;
  messageCount: number;
  attachmentCount: number;
  byteSize: number;
}

export interface ConversationFilters {
  page: number;
  pageSize: number;
  search: string;
  dateFrom: string;
  dateTo: string;
  role: string;
  archived: '' | 'true' | 'false';
  starred: '' | 'true' | 'false';
  hasAttachments: '' | 'true' | 'false';
}

export interface ApiErrorBody {
  error?: {
    code?: string;
    message?: string;
  };
}
