import React, { useEffect, useRef } from 'react';
import { localized } from '../intl/localize';
import * as Actions from '../data/actions';
import LabelColorStore, { LABEL_COLOR_SCHEMES } from '../data/stores/label-color';
import { LabelColorizer } from './mail-label';

type LabelColorPopoverProps = {
  label: { displayName: string; hue: () => number };
};

function hslToHex(hsl: string) {
  const m = /hsl\(([\d.]+),\s*([\d.]+)%,\s*([\d.]+)%\)/.exec(hsl);
  if (!m) return '#808080';
  const [h, s, l] = [Number(m[1]) % 360, Number(m[2]) / 100, Number(m[3]) / 100];
  const a = s * Math.min(l, 1 - l);
  const channel = (n: number) => {
    const k = (n + h / 30) % 12;
    const v = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(v * 255)
      .toString(16)
      .padStart(2, '0');
  };
  return `#${channel(0)}${channel(8)}${channel(4)}`;
}

/** The colour of one label: a swatch of the current scheme, any colour, or back to the scheme. */
export default function LabelColorPopover({ label }: LabelColorPopoverProps) {
  const scheme = LabelColorStore.scheme();
  const swatches =
    scheme === 'automatic' ? LABEL_COLOR_SCHEMES.bright : LABEL_COLOR_SCHEMES[scheme];
  const custom = LabelColorStore.customColor(label);
  const current = LabelColorStore.colorFor(label);
  const input = useRef<HTMLInputElement>(null);

  // The native `change` event fires once the system picker closes; React's onChange fires on
  // every movement, which would write the config file each time.
  useEffect(() => {
    const el = input.current;
    const onChange = () => LabelColorStore.setCustomColor(label, el.value);
    el.addEventListener('change', onChange);
    return () => el.removeEventListener('change', onChange);
  }, [label]);

  const choose = (color: string | null) => {
    LabelColorStore.setCustomColor(label, color);
    Actions.closePopover();
  };

  return (
    <div className="label-color-popover">
      <div className="swatches">
        {swatches.map((color) => (
          <button
            key={color}
            className={`swatch ${custom === color ? 'selected' : ''}`}
            style={{ backgroundColor: LabelColorizer.dotColorFor(color) }}
            aria-label={color}
            onClick={() => choose(color)}
          />
        ))}
      </div>
      <div className="actions">
        <label className="btn btn-small">
          {localized('Custom...')}
          <input
            ref={input}
            type="color"
            defaultValue={current.startsWith('#') ? current : hslToHex(current)}
          />
        </label>
        <button className="btn btn-small" disabled={!custom} onClick={() => choose(null)}>
          {localized('Use Scheme Color')}
        </button>
      </div>
    </div>
  );
}
