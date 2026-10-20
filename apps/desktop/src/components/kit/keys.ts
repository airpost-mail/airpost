// SPDX-License-Identifier: GPL-3.0-only
import type React from 'react';

/**
 * The global keymap (Mousetrap on `document`) binds plain keys: Space is
 * core:messages-page-down, Enter core:focus-item, the arrows move in the thread list. Its
 * handler returns false, which calls preventDefault() and so also cancels a focused <button>'s
 * native click. A kit control that owns a key marks it as handled for the keymap, so the key acts
 * on the control only, as it does on the native inputs these controls replace (stopCallback
 * treats INPUT as text input) and on the legacy Switch.
 *
 * The mark is the flag stopCallback reads (`isPropagationStopped()`, see enhanceEventObject in
 * src/window/app-env.ts), set without stopping propagation: Radix RadioGroup tracks arrow keys
 * with its own listener on `document` and checks the item that arrow focus lands on.
 */
export function ownKeys<E extends Element>(
  keys: string[],
  handler?: React.KeyboardEventHandler<E>
): React.KeyboardEventHandler<E> {
  return (e) => {
    handler?.(e);
    if (keys.includes(e.key) && !e.altKey && !e.ctrlKey && !e.metaKey) {
      (e.nativeEvent as any).propagationStopped = true;
    }
  };
}

/**
 * Like ownKeys, plus every printable character: for controls with typeahead, such as the
 * Select trigger, which stands in for a <select> (stopCallback shields SELECT as text input).
 */
export function ownKeysAndTypeahead<E extends Element>(
  keys: string[],
  handler?: React.KeyboardEventHandler<E>
): React.KeyboardEventHandler<E> {
  return (e) => {
    handler?.(e);
    if ((keys.includes(e.key) || e.key.length === 1) && !e.altKey && !e.ctrlKey && !e.metaKey) {
      (e.nativeEvent as any).propagationStopped = true;
    }
  };
}
