// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';

export type TextareaProps = React.ComponentPropsWithoutRef<'textarea'> & {
  invalid?: boolean;
};

/* Public: A multi-line text input. `invalid` sets `aria-invalid`. */
export const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(function Textarea(
  { invalid, ...props },
  ref
) {
  return <textarea data-slot="textarea" aria-invalid={invalid || undefined} ref={ref} {...props} />;
});
Textarea.displayName = 'Textarea';
