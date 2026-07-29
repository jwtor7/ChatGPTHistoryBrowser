import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { AttachmentCard } from '../../src/AttachmentCard';
import type { LocalApi } from '../../src/api';
import type { AttachmentView } from '../../src/types';

const API = {} as LocalApi;

function attachment(overrides: Partial<AttachmentView>): AttachmentView {
  return {
    id: 'synthetic-attachment',
    displayName: 'Synthetic attachment',
    claimedMime: null,
    detectedMime: null,
    byteSize: 1_024,
    status: 'available',
    previewKind: 'unsupported',
    ...overrides,
  };
}

describe('AttachmentCard file types', () => {
  it('labels a detected ZIP archive accurately', () => {
    render(
      <AttachmentCard
        api={API}
        attachment={attachment({
          displayName: 'Synthetic archive.zip',
          detectedMime: 'application/zip',
        })}
      />,
    );

    expect(screen.getByText(/ZIP archive · 1\.0 KB/i)).toBeVisible();
    expect(screen.queryByText(/unknown file type/i)).not.toBeInTheDocument();
  });

  it('allows previewing the FLAC MIME emitted by signature detection', () => {
    render(
      <AttachmentCard
        api={API}
        attachment={attachment({
          displayName: 'Synthetic audio.flac',
          detectedMime: 'audio/x-flac',
          previewKind: 'audio',
        })}
      />,
    );

    expect(screen.getByText(/FLAC audio · 1\.0 KB/i)).toBeVisible();
    expect(screen.getByRole('button', { name: /preview audio/i })).toBeEnabled();
    expect(screen.queryByText(/not allowlisted/i)).not.toBeInTheDocument();
  });
});
