import { useMemo } from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

/** A Markdown text card, sanitised before it is inserted. */
export function TextCard({ text }: { text: string }) {
  const html = useMemo(() => {
    if (!text) return '';
    const raw = marked.parse(text, { async: false }) as string;
    return DOMPurify.sanitize(raw);
  }, [text]);

  if (!text) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--muted)', fontSize: 12 }}>
        No text configured
      </div>
    );
  }

  return (
    <div
      style={{ padding: '12px 16px', overflow: 'auto', height: '100%', fontSize: 14, lineHeight: 1.6, color: 'var(--fg)' }}
      dangerouslySetInnerHTML={{ __html: html }}
      className="md-text"
    />
  );
}
