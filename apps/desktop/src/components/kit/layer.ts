// SPDX-License-Identifier: GPL-3.0-only

/**
 * Marks a focus-taking layer's content element. stopCallback in src/window/keymap-manager.ts
 * keeps plain keys pressed inside it from firing global commands.
 */
export const LAYER_ATTRIBUTE = 'data-ui-layer';
export const layerProps = { [LAYER_ATTRIBUTE]: '' };

/**
 * Wraps a layer's onEscapeKeyDown so the Escape that dismisses the layer stops there.
 * Radix only calls preventDefault(); stopping propagation keeps the keymap's stopCallback
 * (isPropagationStopped) from also firing core:pop-sheet. Propagation is stopped even when the
 * handler calls preventDefault() to keep the layer open.
 */
export function consumeEscape(handler?: (e: KeyboardEvent) => void) {
  return (e: KeyboardEvent) => {
    handler?.(e);
    e.stopPropagation();
  };
}
