import { Check, LoaderCircle, TriangleAlert } from 'lucide-react';
import { useId, useState, type KeyboardEvent } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { parseServerUrl, serverLabel } from './server-url';
import type { ServerInfo } from './transport';

export interface ServerFieldProps {
  url: string;
  onChange: (url: string) => void;
  probe: { isPending: boolean; error: Error | null; data?: ServerInfo };
  mock: boolean;
}

/** The server address as one quiet line, editable on demand. */
export function ServerField({ url, onChange, probe, mock }: ServerFieldProps) {
  const id = useId();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(url);
  const [error, setError] = useState<string | null>(null);

  const commit = () => {
    const parsed = parseServerUrl(draft);
    if (!parsed.ok) {
      setError(parsed.reason);
      return;
    }
    setError(null);
    setEditing(false);
    onChange(parsed.url);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setDraft(url);
      setError(null);
      setEditing(false);
    }
  };

  if (editing) {
    return (
      <div className="flex flex-col gap-1.5">
        <label htmlFor={id} className="sr-only">
          Server address
        </label>
        <div className="flex gap-2">
          <Input
            id={id}
            autoFocus
            inputMode="url"
            spellCheck={false}
            autoCapitalize="off"
            placeholder="tern.example.com"
            value={draft}
            aria-invalid={!!error}
            aria-describedby={error ? `${id}-error` : undefined}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            className="font-mono text-xs"
          />
          <Button type="button" variant="outline" onClick={commit}>
            Use
          </Button>
        </div>
        {error && (
          <p id={`${id}-error`} className="text-xs text-destructive">
            {error}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2 border-y border-hairline py-2">
      <ProbeDot probe={probe} />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-mono text-xs" title={url}>
          {serverLabel(url)}
        </span>
        <span
          className={cn(
            'truncate font-mono text-2xs text-muted-foreground tabular',
            probe.error && 'text-destructive',
          )}
        >
          {probeLine(probe, mock)}
        </span>
      </div>
      <Button
        type="button"
        variant="link"
        size="xs"
        className="text-xs"
        onClick={() => {
          setDraft(url);
          setEditing(true);
        }}
      >
        Change
      </Button>
    </div>
  );
}

function probeLine(probe: ServerFieldProps['probe'], mock: boolean): string {
  if (probe.error) return probe.error.message;
  if (probe.isPending) return 'checking…';
  if (mock) return 'demo engine · any credentials work';
  const info = probe.data!;
  return `tern ${info.version} · signup ${info.signup}`;
}

function ProbeDot({ probe }: { probe: ServerFieldProps['probe'] }) {
  if (probe.isPending) {
    return <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" aria-hidden />;
  }
  if (probe.error) return <TriangleAlert className="size-3.5 text-destructive" aria-hidden />;
  return <Check className="size-3.5 text-success" aria-hidden />;
}
