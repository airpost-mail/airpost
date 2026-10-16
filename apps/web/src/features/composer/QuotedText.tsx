import { Ellipsis } from 'lucide-react';
import { useState } from 'react';
import { MessageBody } from '@/components/email';
import { Button } from '@/components/ui/button';

/** The quoted original, previewed in the sandboxed frame; it is never editable. */
export function QuotedText({
  html,
  loading,
  included,
  forwarded,
  onIncludedChange,
}: {
  html: string | null;
  loading: boolean;
  included: boolean;
  forwarded: boolean;
  onIncludedChange: (included: boolean) => void;
}) {
  const [expanded, setExpanded] = useState(forwarded);

  if (loading) {
    return (
      <p className="px-(--pane-gutter) pb-3 text-xs text-muted-foreground">
        Loading the original message…
      </p>
    );
  }
  if (!html) return null;

  if (!included) {
    return (
      <div className="flex items-center gap-2 px-(--pane-gutter) pb-3 text-xs text-muted-foreground">
        <span>{forwarded ? 'Forwarded message removed.' : 'Quoted text removed.'}</span>
        <Button size="xs" variant="ghost" onClick={() => onIncludedChange(true)}>
          Put it back
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5 px-(--pane-gutter) pb-3">
      <div className="flex items-center gap-1">
        <Button
          size="xs"
          variant="secondary"
          aria-expanded={expanded}
          aria-label={expanded ? 'Hide quoted text' : 'Show quoted text'}
          onClick={() => setExpanded((e) => !e)}
          className="h-4 px-1.5"
        >
          <Ellipsis aria-hidden />
        </Button>
        {expanded && (
          <Button
            size="xs"
            variant="ghost"
            className="text-muted-foreground"
            onClick={() => onIncludedChange(false)}
          >
            {forwarded ? 'Remove forwarded message' : 'Remove quoted text'}
          </Button>
        )}
      </div>
      {expanded && (
        <MessageBody
          body={html}
          className="animate-in rounded-md bg-white p-2 duration-(--duration-base) fade-in"
        />
      )}
    </div>
  );
}
