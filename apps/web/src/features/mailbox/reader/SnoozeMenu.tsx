import { format, isBefore, set, startOfDay } from 'date-fns';
import { AlarmClock, CalendarClock, Inbox } from 'lucide-react';
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
import { useShortcut, useShortcutScope } from '@/lib/shortcuts';
import { formatSnooze, snoozePresets } from '../snooze';

/** The reader's snooze button: the desktop's presets, a picked time, or unsnooze. */
export function SnoozeMenu({
  onSnooze,
  onUnsnooze,
}: {
  onSnooze: (at: Date) => void;
  /** Set when the thread is snoozed. */
  onUnsnooze?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [presets, setPresets] = useState(() => snoozePresets());
  const show = (next: boolean) => {
    if (next) setPresets(snoozePresets());
    setOpen(next);
  };
  useShortcut('z', () => show(true), { group: 'Thread', description: 'Snooze' });

  return (
    <>
      <DropdownMenu open={open} onOpenChange={show}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Snooze"
                className="text-muted-foreground hover:text-foreground"
              >
                <AlarmClock className="size-4" />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent className="flex items-center gap-2">
            Snooze <ShortcutKeys keys="z" />
          </TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="start" className="w-60">
          <DropdownMenuLabel className="text-2xs tracking-widest text-muted-foreground uppercase">
            Snooze until
          </DropdownMenuLabel>
          {presets.map((p) => (
            <DropdownMenuItem key={p.label} onSelect={() => onSnooze(p.at)}>
              <span className="flex-1">{p.label}</span>
              <span className="font-mono text-2xs text-muted-foreground tabular-nums">
                {format(p.at, 'EEE HH:mm')}
              </span>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => setPickerOpen(true)}>
            <CalendarClock aria-hidden />
            Pick a date and time…
          </DropdownMenuItem>
          {onUnsnooze && (
            <DropdownMenuItem onSelect={onUnsnooze}>
              <Inbox aria-hidden />
              Unsnooze now
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <SnoozeDialog
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        onSnooze={(at) => {
          setPickerOpen(false);
          onSnooze(at);
        }}
      />
    </>
  );
}

function SnoozeDialog({
  open,
  onOpenChange,
  onSnooze,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSnooze: (at: Date) => void;
}) {
  useShortcutScope('picker', open);
  const [day, setDay] = useState<Date | undefined>(undefined);
  const [time, setTime] = useState('09:00');
  const [hours, minutes] = time.split(':').map(Number);
  const at =
    day && Number.isFinite(hours) ? set(day, { hours, minutes: minutes || 0, seconds: 0 }) : null;
  const inPast = !!at && isBefore(at, new Date());

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-xl font-semibold tracking-tight">Snooze</DialogTitle>
          <DialogDescription>Pick when this thread comes back to the inbox.</DialogDescription>
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
          <Button disabled={!at || inPast} onClick={() => at && onSnooze(at)}>
            {at && !inPast ? `Snooze until ${formatSnooze(at)}` : 'Snooze'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
