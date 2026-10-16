import { useAccounts } from '@/engine';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { identitiesFor, identityKey, type Identity } from './identities';
import { formatContact } from './recipients';

export function FromSelect({
  value,
  onChange,
}: {
  value: Identity;
  onChange: (next: Identity) => void;
}) {
  const { data: accounts = [] } = useAccounts();
  const groups = accounts.map((a) => ({ account: a, identities: identitiesFor(a) }));
  const all = groups.flatMap((g) => g.identities);
  const current = identityKey(value);
  // A reopened draft can be from an address that is no longer an alias; keep it selectable.
  const known = all.some((i) => identityKey(i) === current);

  return (
    <div className="flex h-9 items-center gap-2 border-b border-hairline px-(--pane-gutter)">
      <span className="w-9 shrink-0 text-xs text-muted-foreground">From</span>
      <Select
        value={current}
        onValueChange={(k) => {
          const next = all.find((i) => identityKey(i) === k);
          if (next) onChange(next);
        }}
      >
        <SelectTrigger
          size="sm"
          aria-label="From"
          className="-ml-2.5 h-7 max-w-full min-w-0 border-transparent bg-transparent shadow-none hover:bg-accent dark:bg-transparent"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent position="popper" align="start">
          {!known && <SelectItem value={current}>{formatContact(value.contact)}</SelectItem>}
          {groups.map(({ account, identities }) => (
            <SelectGroup key={account.id}>
              {groups.length > 1 && (
                <SelectLabel className="flex items-center gap-1.5 text-2xs tracking-widest uppercase">
                  <span
                    aria-hidden
                    className="size-1.5 rounded-full"
                    style={{ backgroundColor: account.color }}
                  />
                  {account.label || account.emailAddress}
                </SelectLabel>
              )}
              {identities.map((i) => (
                <SelectItem key={identityKey(i)} value={identityKey(i)}>
                  <span className="truncate">{formatContact(i.contact)}</span>
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
