import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';

export interface Choice<T extends string | number> {
  value: T;
  label: string;
}

/** A Select over a fixed list; numeric values round-trip through their string form. */
export function ChoiceSelect<T extends string | number>({
  id,
  value,
  choices,
  onChange,
  className = 'w-44',
  'aria-label': ariaLabel,
}: {
  id?: string;
  value: T;
  choices: readonly Choice<T>[];
  onChange: (v: T) => void;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <Select
      value={String(value)}
      onValueChange={(s) => {
        const hit = choices.find((c) => String(c.value) === s);
        if (hit) onChange(hit.value);
      }}
    >
      <SelectTrigger id={id} size="sm" className={className} aria-label={ariaLabel}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {choices.map((c) => (
          <SelectItem key={String(c.value)} value={String(c.value)}>
            {c.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
