import { FileText, X } from 'lucide-react';
import { formatBytes } from '@/lib/format';
import type { DraftAttachment } from './draft';

export function AttachmentList({
  files,
  onRemove,
}: {
  files: DraftAttachment[];
  onRemove: (id: string) => void;
}) {
  if (files.length === 0) return null;
  return (
    <ul aria-label="Attachments" className="flex flex-wrap gap-1.5 px-(--pane-gutter) pb-3">
      {files.map(({ file }) => (
        <li
          key={file.id}
          className="group/file flex h-8 max-w-60 items-center gap-2 rounded-md border bg-card pr-1 pl-2 text-xs"
        >
          <FileText className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="truncate" title={file.filename}>
            {file.filename}
          </span>
          <span className="shrink-0 font-mono text-2xs text-muted-foreground tabular-nums">
            {formatBytes(file.size)}
          </span>
          <button
            type="button"
            aria-label={`Remove ${file.filename}`}
            onClick={() => onRemove(file.id)}
            className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <X className="size-3" aria-hidden />
          </button>
        </li>
      ))}
    </ul>
  );
}
