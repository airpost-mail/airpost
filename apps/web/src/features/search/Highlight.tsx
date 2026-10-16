import { Fragment } from 'react';

function escapeRegExp(s: string) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Marks every occurrence of `terms` in `text`, case-insensitively. */
export function Highlight({ text, terms }: { text: string; terms: string[] }) {
  const words = terms.map((t) => t.trim()).filter((t) => t.length > 1);
  if (words.length === 0) return <>{text}</>;
  const re = new RegExp(`(${words.map(escapeRegExp).join('|')})`, 'gi');
  return (
    <>
      {text.split(re).map((part, i) =>
        i % 2 === 1 ? (
          <mark key={i} className="rounded-[2px] bg-primary/15 text-inherit">
            {part}
          </mark>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </>
  );
}
