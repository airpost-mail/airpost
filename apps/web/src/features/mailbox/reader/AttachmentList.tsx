import {
  File,
  FileArchive,
  FileImage,
  FileSpreadsheet,
  FileText,
  type LucideIcon,
} from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useEngine, type FileJSON } from '@/engine';
import { formatBytes } from '@/lib/format';

function iconFor(f: FileJSON): LucideIcon {
  const t = f.contentType.toLowerCase();
  if (t.startsWith('image/')) return FileImage;
  if (t.includes('zip') || t.includes('compressed') || t.includes('tar')) return FileArchive;
  if (t.includes('sheet') || t.includes('csv') || t.includes('excel')) return FileSpreadsheet;
  if (t.startsWith('text/') || t.includes('pdf') || t.includes('document') || t.includes('word'))
    return FileText;
  return File;
}

export function AttachmentList({ files }: { files: FileJSON[] }) {
  const engine = useEngine();
  if (files.length === 0) return null;
  return (
    <ul aria-label="Attachments" className="flex flex-wrap gap-2">
      {files.map((f) => {
        const Icon = iconFor(f);
        // Null on an engine without `files`.
        const href = engine.fileUrl(f);
        const body = (
          <>
            <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <span className="min-w-0 flex-1 truncate">{f.filename || 'Untitled'}</span>
            <span className="font-mono text-2xs text-muted-foreground tabular">
              {formatBytes(f.size)}
            </span>
          </>
        );
        const cls =
          'flex h-9 w-56 items-center gap-2 rounded-md border px-2.5 text-xs transition-colors duration-(--duration-fast)';
        return (
          <li key={f.id}>
            {href ? (
              <a href={href} download={f.filename} className={`${cls} hover:bg-accent`}>
                {body}
              </a>
            ) : (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span aria-disabled className={`${cls} cursor-not-allowed opacity-60`}>
                    {body}
                  </span>
                </TooltipTrigger>
                <TooltipContent>This engine can't serve attachments; update it</TooltipContent>
              </Tooltip>
            )}
          </li>
        );
      })}
    </ul>
  );
}
