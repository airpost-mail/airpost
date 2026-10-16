import { Expand, Maximize2, Minimize2, Minus, Shrink, X } from 'lucide-react';
import type { ReactNode } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { focusComposer, setComposerView, type ComposerView } from './store';

function HeaderButton({
  label,
  keys,
  onClick,
  children,
}: {
  label: string;
  keys?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          size="icon-xs"
          variant="ghost"
          aria-label={label}
          onClick={(e) => {
            e.stopPropagation();
            onClick();
          }}
          className="text-muted-foreground hover:text-foreground"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent className="flex items-center gap-2">
        {label}
        {keys && <ShortcutKeys keys={keys} />}
      </TooltipContent>
    </Tooltip>
  );
}

export function WindowHeader({
  windowKey,
  title,
  view,
  status,
  onClose,
}: {
  windowKey: string;
  title: string;
  view: ComposerView;
  status?: ReactNode;
  onClose: () => void;
}) {
  const minimised = view === 'minimised';
  return (
    <div
      className="flex h-9 shrink-0 cursor-default items-center gap-2 rounded-t-lg border-b border-hairline bg-topbar pr-1.5 pl-(--pane-gutter) select-none"
      onClick={() => minimised && focusComposer(windowKey)}
      onDoubleClick={() => setComposerView(windowKey, minimised ? 'normal' : 'minimised')}
    >
      <h2
        className="min-w-0 flex-1 truncate text-sm font-medium text-muted-foreground transition-colors group-data-focused/composer:text-foreground"
        title={title}
      >
        {title}
      </h2>
      {status}
      <div className="flex items-center">
        <HeaderButton
          label={minimised ? 'Restore' : 'Minimise'}
          keys={minimised ? undefined : 'escape'}
          onClick={() => setComposerView(windowKey, minimised ? 'normal' : 'minimised')}
        >
          <Minus aria-hidden />
        </HeaderButton>
        {view !== 'fullscreen' && (
          <HeaderButton
            label={view === 'maximised' ? 'Smaller' : 'Taller'}
            onClick={() =>
              setComposerView(windowKey, view === 'maximised' ? 'normal' : 'maximised')
            }
          >
            {view === 'maximised' ? <Minimize2 aria-hidden /> : <Maximize2 aria-hidden />}
          </HeaderButton>
        )}
        <HeaderButton
          label={view === 'fullscreen' ? 'Exit full screen' : 'Full screen'}
          onClick={() =>
            setComposerView(windowKey, view === 'fullscreen' ? 'normal' : 'fullscreen')
          }
        >
          {view === 'fullscreen' ? <Shrink aria-hidden /> : <Expand aria-hidden />}
        </HeaderButton>
        <HeaderButton label="Close and keep draft" onClick={onClose}>
          <X aria-hidden />
        </HeaderButton>
      </div>
    </div>
  );
}
