import { ShortcutKeys } from '@/components/ShortcutKeys';
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet';
import {
  useShortcutList,
  useShortcutScope,
  type ShortcutDef,
  type ShortcutGroup,
} from '@/lib/shortcuts';

const ORDER: ShortcutGroup[] = [
  'Navigation',
  'Thread',
  'Compose',
  'Calendar',
  'Search',
  'Application',
];

export function ShortcutsSheet({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
}) {
  const list = useShortcutList();
  // Thread shortcuts must not act on the list behind the sheet.
  useShortcutScope('sheet', open);
  const groups = new Map<ShortcutGroup, ShortcutDef[]>();
  for (const s of list) {
    const g = s.group ?? 'Application';
    if (!groups.has(g)) groups.set(g, []);
    // Several components may register the same binding; list it once.
    if (!groups.get(g)!.some((x) => x.description === s.description)) groups.get(g)!.push(s);
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-80 sm:max-w-80">
        <SheetHeader>
          <SheetTitle className="text-xl font-semibold tracking-tight">Shortcuts</SheetTitle>
          <SheetDescription>Everything here works without the mouse.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-5 overflow-y-auto px-4 pb-6">
          {ORDER.filter((g) => groups.has(g)).map((g) => (
            <section key={g} className="flex flex-col gap-1">
              <h3 className="text-2xs font-medium tracking-widest text-muted-foreground uppercase">
                {g}
              </h3>
              {groups.get(g)!.map((s) => (
                <div key={s.description} className="flex items-center justify-between gap-3 py-1">
                  <span className="text-sm">{s.description}</span>
                  <span className="flex gap-1.5">
                    {(Array.isArray(s.keys) ? s.keys : [s.keys]).slice(0, 2).map((k) => (
                      <ShortcutKeys key={k} keys={k} />
                    ))}
                  </span>
                </div>
              ))}
            </section>
          ))}
        </div>
      </SheetContent>
    </Sheet>
  );
}
