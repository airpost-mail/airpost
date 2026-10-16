import { format, isBefore, set, startOfDay } from 'date-fns';
import { ChevronDown, Clock, SendHorizontal } from 'lucide-react';
import { useState } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Calendar } from '@/components/ui/calendar';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useCapability } from '@/features/settings';
import { cn } from '@/lib/utils';
import { useShortcutScope } from '@/lib/shortcuts';
import { formatSendLater, sendLaterPresets } from './send-later';

export function SendControls({
  busy,
  onSend,
  onSchedule,
}: {
  busy: boolean;
  onSend: () => void;
  onSchedule: (at: Date) => void;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [presets, setPresets] = useState(() => sendLaterPresets());
  const sendLater = useCapability('send-later').available;

  return (
    <div className="flex items-center">
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            size="sm"
            disabled={busy}
            onClick={onSend}
            className={cn(sendLater && 'rounded-r-none pr-2.5')}
          >
            <SendHorizontal aria-hidden />
            Send
          </Button>
        </TooltipTrigger>
        <TooltipContent className="flex items-center gap-2">
          Send <ShortcutKeys keys="mod+enter" />
        </TooltipContent>
      </Tooltip>
      {sendLater && (
        <>
          <DropdownMenu onOpenChange={(open) => open && setPresets(sendLaterPresets())}>
            <DropdownMenuTrigger asChild>
              <Button
                size="sm"
                disabled={busy}
                aria-label="Send later"
                className="rounded-l-none border-l border-l-primary-foreground/20 px-1.5"
              >
                <ChevronDown aria-hidden />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-60">
              <DropdownMenuLabel className="text-2xs tracking-widest text-muted-foreground uppercase">
                Send later
              </DropdownMenuLabel>
              {presets.map((p) => (
                <DropdownMenuItem key={p.label} onSelect={() => onSchedule(p.at)}>
                  <span className="flex-1">{p.label}</span>
                  <span className="font-mono text-2xs text-muted-foreground tabular-nums">
                    {format(p.at, 'EEE HH:mm')}
                  </span>
                </DropdownMenuItem>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => setPickerOpen(true)}>
                <Clock aria-hidden />
                Pick a date and time…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <SendLaterDialog
            open={pickerOpen}
            onOpenChange={setPickerOpen}
            onSchedule={(at) => {
              setPickerOpen(false);
              onSchedule(at);
            }}
          />
        </>
      )}
    </div>
  );
}

function SendLaterDialog({
  open,
  onOpenChange,
  onSchedule,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSchedule: (at: Date) => void;
}) {
  useShortcutScope('composer-dialog', open);
  const [day, setDay] = useState<Date | undefined>(undefined);
  const [time, setTime] = useState('08:00');
  const [hours, minutes] = time.split(':').map(Number);
  const at =
    day && Number.isFinite(hours) ? set(day, { hours, minutes: minutes || 0, seconds: 0 }) : null;
  const inPast = !!at && isBefore(at, new Date());

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-xl font-semibold tracking-tight">Send later</DialogTitle>
          <DialogDescription>Pick when this message goes out.</DialogDescription>
        </DialogHeader>
        <Calendar
          mode="single"
          selected={day}
          onSelect={setDay}
          disabled={{ before: startOfDay(new Date()) }}
          className="mx-auto"
        />
        <label className="flex items-center gap-3 text-sm">
          <span className="text-muted-foreground">Time</span>
          <Input
            type="time"
            value={time}
            onChange={(e) => setTime(e.target.value)}
            className="h-8 w-28 font-mono tabular-nums"
          />
        </label>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!at || inPast} onClick={() => at && onSchedule(at)}>
            {at && !inPast ? `Schedule for ${formatSendLater(at)}` : 'Schedule'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
