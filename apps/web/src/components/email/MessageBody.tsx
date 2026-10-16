import { ImageOff } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { setPreferences, usePreferences } from '@/lib/preferences';
import { cn } from '@/lib/utils';
import { EmailFrame, type EmailFrameProps } from './EmailFrame';
import { hasRemoteContent } from './sanitize';

export interface MessageBodyProps extends Pick<EmailFrameProps, 'resolveCid' | 'onLinkClick'> {
  body: string;
  plaintext?: boolean;
  className?: string;
}

/** A message body: plain text inline, HTML in the sandboxed frame with a remote-images bar. */
export function MessageBody({ body, plaintext, className, ...frame }: MessageBodyProps) {
  const prefs = usePreferences();
  const [allowed, setAllowed] = useState(false);
  const remote = !plaintext && hasRemoteContent(body);
  const showRemote = prefs.loadRemoteImages || allowed;

  if (plaintext) {
    return <div className={cn('text-base break-words whitespace-pre-wrap', className)}>{body}</div>;
  }

  return (
    <div className={cn('flex flex-col gap-2', className)}>
      {remote && !showRemote && (
        <div className="flex items-center gap-2 rounded-md border border-dashed px-3 py-1.5 text-xs text-muted-foreground">
          <ImageOff className="size-3.5" aria-hidden />
          <span className="flex-1">Remote images are hidden to protect your privacy.</span>
          <Button size="xs" variant="ghost" onClick={() => setAllowed(true)}>
            Load images
          </Button>
          <Button
            size="xs"
            variant="ghost"
            onClick={() => setPreferences({ loadRemoteImages: true })}
          >
            Always
          </Button>
        </div>
      )}
      <EmailFrame html={body} allowRemoteImages={showRemote} {...frame} />
    </div>
  );
}
