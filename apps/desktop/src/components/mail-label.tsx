import React, { useEffect, useState } from 'react';
import { RetinaImg } from './retina-img';
import LabelColorStore from '../data/stores/label-color';

type MailLabelProps = {
  label: {
    id?: string;
    displayName: React.ReactElement | string;
    isLockedCategory: () => boolean;
    hue: () => number;
  };
  onRemove?: (...args: any[]) => any;
};

// A chip is a tint of the label's hue with text in a deep shade of it (light themes), or the
// reverse (dark themes). The lightness and chroma bounds are theme variables
// (static/styles/mixins/label-colors.less) picked from each theme's own backgrounds, so any
// label colour keeps 4.5:1 text contrast; the fallbacks are the light-theme values.
const chipBackground = (base: string) =>
  `oklch(from ${base} clamp(var(--label-chip-lmin, 0.86), l, var(--label-chip-lmax, 0.95)) min(c, var(--label-chip-cmax, 0.16)) h)`;

const chipText = (base: string) =>
  `oklch(from ${base} var(--label-text-l, 0.4) min(c, var(--label-text-cmax, 0.18)) h)`;

const dot = (base: string) =>
  `oklch(from ${base} clamp(var(--label-dot-lmin, 0.3), l, var(--label-dot-lmax, 0.58)) c h)`;

type ColorableLabel = Parameters<typeof LabelColorStore.colorFor>[0];

export const LabelColorizer = {
  /** A dot of any colour, fitted to the theme (scheme previews, the colour picker). */
  dotColorFor(base: string) {
    return dot(base);
  },

  /** The label's own colour, before it is fitted to the theme. */
  baseColor(label: ColorableLabel) {
    return LabelColorStore.colorFor(label);
  },

  color(label: ColorableLabel) {
    return chipText(LabelColorizer.baseColor(label));
  },

  backgroundColor(label: ColorableLabel) {
    return chipBackground(LabelColorizer.baseColor(label));
  },

  /** A dot or icon tint on the surrounding background (pickers, sidebar). */
  backgroundColorDark(label: ColorableLabel) {
    return dot(LabelColorizer.baseColor(label));
  },

  styles(label: ColorableLabel) {
    return {
      color: LabelColorizer.color(label),
      backgroundColor: LabelColorizer.backgroundColor(label),
    };
  },
};

function useLabelColors() {
  const [, setVersion] = useState(0);
  useEffect(() => {
    const unlisten = LabelColorStore.listen(() => setVersion((v) => v + 1));
    return () => {
      unlisten();
    };
  }, []);
}

export const MailLabel: React.FC<MailLabelProps> = React.memo(
  ({ label, onRemove }) => {
    useLabelColors();
    const removable = onRemove && !label.isLockedCategory();
    let classname = 'mail-label';
    let content: JSX.Element | string = label.displayName;

    let x: JSX.Element | null = null;
    if (removable) {
      classname += ' removable';
      content = <span className="inner">{content}</span>;
      x = (
        <RetinaImg
          className="x"
          name="label-x.png"
          style={{ backgroundColor: LabelColorizer.color(label) }}
          mode={RetinaImg.Mode.ContentIsMask}
          onClick={onRemove}
        />
      );
    }

    return (
      <div className={classname} style={LabelColorizer.styles(label)}>
        {content}
        {x}
      </div>
    );
  },
  (prev, next) => prev.label.id === next.label.id
);
MailLabel.displayName = 'MailLabel';
