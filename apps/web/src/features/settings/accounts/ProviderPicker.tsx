import { ChevronRight } from 'lucide-react';
import { PRESETS, type ProviderPreset } from './presets';

export function ProviderPicker({ onPick }: { onPick: (p: ProviderPreset) => void }) {
  return (
    <ul aria-label="Providers" className="grid grid-cols-2 gap-2">
      {PRESETS.map((p) => (
        <li key={p.id}>
          <button
            type="button"
            onClick={() => onPick(p)}
            className="group flex w-full items-center justify-between gap-3 rounded-lg border bg-card px-4 py-3 text-left transition-colors duration-(--duration-fast) hover:border-primary/50 hover:bg-row-hover"
          >
            <span className="flex flex-col">
              <span className="text-sm font-medium">{p.name}</span>
              <span className="font-mono text-2xs text-muted-foreground">
                {p.imap?.host ?? 'any IMAP and SMTP server'}
              </span>
            </span>
            <ChevronRight className="size-4 text-muted-foreground transition-transform duration-(--duration-fast) group-hover:translate-x-0.5" />
          </button>
        </li>
      ))}
    </ul>
  );
}
