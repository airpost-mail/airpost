// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';

/* The kit's small glyphs, drawn in currentColor. */

export const CheckGlyph = () => (
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
    <path
      d="M3.5 8.5l3 3 6-7"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      fill="none"
    />
  </svg>
);

export const DotGlyph = () => (
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
    <circle cx="8" cy="8" r="3" fill="currentColor" />
  </svg>
);
