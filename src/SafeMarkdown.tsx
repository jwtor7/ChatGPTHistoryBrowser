import type { ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import remarkGfm from 'remark-gfm';

interface SafeMarkdownProps {
  children: string;
}

function InertLink({ children }: { children?: ReactNode }) {
  return (
    <span className="inert-link" title="Archived links are disabled">
      {children}
      <span className="sr-only"> (link disabled)</span>
    </span>
  );
}

export function SafeMarkdown({ children }: SafeMarkdownProps) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeSanitize]}
      skipHtml
      urlTransform={() => ''}
      components={{
        a: ({ children: linkChildren }) => <InertLink>{linkChildren}</InertLink>,
        img: ({ alt }) => (
          <span className="blocked-resource">
            {alt ? `Image reference blocked: ${alt}` : 'Remote image blocked'}
          </span>
        ),
        code: ({ className, children: codeChildren }) => (
          <code className={className}>{codeChildren}</code>
        ),
      }}
    >
      {children}
    </ReactMarkdown>
  );
}
