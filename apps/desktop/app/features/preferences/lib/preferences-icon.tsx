import React from 'react';
import { RetinaImg } from 'mailspring-component-kit';

// Built-in tabs draw a white line glyph on a coloured tile. Plugin tabs keep their
// `icon-preferences-<tabId>.png`, the convention plugins already ship.
const STROKE = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
} as const;

const GLYPHS: { [tabId: string]: { color: string; glyph: React.ReactNode } } = {
  General: {
    color: 'linear-gradient(#9aa0aa, #7c828d)',
    glyph: (
      <>
        <circle cx="12" cy="12" r="3" {...STROKE} />
        <path
          {...STROKE}
          d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"
        />
      </>
    ),
  },
  Notifications: {
    color: 'linear-gradient(#ff6b6b, #e84545)',
    glyph: (
      <path
        {...STROKE}
        d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.94 1.94 0 0 0 3.4 0"
      />
    ),
  },
  Appearance: {
    color: 'linear-gradient(#7b7bff, #5a4fe0)',
    glyph: (
      <>
        <circle cx="13.5" cy="6.5" r="1.5" fill="currentColor" />
        <circle cx="17.5" cy="10.5" r="1.5" fill="currentColor" />
        <circle cx="8.5" cy="7.5" r="1.5" fill="currentColor" />
        <circle cx="6.5" cy="12.5" r="1.5" fill="currentColor" />
        <path
          {...STROKE}
          d="M12 2a10 10 0 0 0 0 20c1 0 1.6-.8 1.6-1.7 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.9.8-1.7 1.7-1.7h2a5.6 5.6 0 0 0 5.5-5.5C22 6 17.5 2 12 2Z"
        />
      </>
    ),
  },
  Shortcuts: {
    color: 'linear-gradient(#5d6472, #444a56)',
    glyph: (
      <path
        {...STROKE}
        d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"
      />
    ),
  },
  Accounts: {
    color: 'linear-gradient(#4aa3ff, #0f78f0)',
    glyph: (
      <>
        <circle cx="12" cy="12" r="4" {...STROKE} />
        <path {...STROKE} d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8" />
      </>
    ),
  },
  Reading: {
    color: 'linear-gradient(#3cc8b4, #1aa594)',
    glyph: (
      <path
        {...STROKE}
        d="M2 4h6a4 4 0 0 1 4 4v13a3 3 0 0 0-3-3H2zM22 4h-6a4 4 0 0 0-4 4v13a3 3 0 0 1 3-3h7z"
      />
    ),
  },
  Composing: {
    color: 'linear-gradient(#ffa24d, #f07b16)',
    glyph: <path {...STROKE} d="M12 20h9M16.5 3.5a2.1 2.1 0 1 1 3 3L7 19l-4 1 1-4Z" />,
  },
  Signatures: {
    color: 'linear-gradient(#ff7aa8, #e8467f)',
    glyph: <path {...STROKE} d="M3 17c3-1 4-9 7-9s-1 9 2 9 3-5 5-5 1 4 4 4M3 21h18" />,
  },
  Templates: {
    color: 'linear-gradient(#ffcb5a, #f2941a)',
    glyph: (
      <path
        {...STROKE}
        d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M8 13h8M8 17h5"
      />
    ),
  },
  'Mail Rules': {
    color: 'linear-gradient(#58d68d, #26a95d)',
    glyph: (
      <path
        {...STROKE}
        d="M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM18 9a9 9 0 0 1-9 9"
      />
    ),
  },
  Folders: {
    color: 'linear-gradient(#9aa0aa, #7c828d)',
    glyph: (
      <path
        {...STROKE}
        d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
      />
    ),
  },
  AI: {
    color: 'linear-gradient(#8478ff, #523ae6)',
    glyph: (
      <path
        fill="currentColor"
        d="M10 3l1.6 5.4L17 10l-5.4 1.6L10 17l-1.6-5.4L3 10l5.4-1.6zM18 13l.8 2.2L21 16l-2.2.8L18 19l-.8-2.2L15 16l2.2-.8zM18 2l.6 1.4L20 4l-1.4.6L18 6l-.6-1.4L16 4l1.4-.6z"
      />
    ),
  },
  Agents: {
    color: 'linear-gradient(#1f3f86, #0a1a3d)',
    glyph: <path {...STROKE} d="M4 17l6-6-6-6M12 19h8" />,
  },
  Integrations: {
    color: 'linear-gradient(#ffcb5a, #f2941a)',
    glyph: <path {...STROKE} d="M9 2v5M15 2v5M6 7h12v4a6 6 0 0 1-12 0zM12 17v5" />,
  },
  Plugins: {
    color: 'linear-gradient(#a96bff, #7b2ff7)',
    glyph: <path {...STROKE} d="M12 2l9 5v10l-9 5-9-5V7zM12 22V12M21 7l-9 5-9-5" />,
  },
  Advanced: {
    color: 'linear-gradient(#9aa0aa, #7c828d)',
    glyph: (
      <path {...STROKE} d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" />
    ),
  },
};

/** The coloured tile for a Preferences tab, at `size` px (22 in the sidebar, 40 in headers). */
export function PreferencesIcon({ tabId, size = 22 }: { tabId: string; size?: number }) {
  const known = GLYPHS[tabId];
  if (!known) {
    return (
      <RetinaImg
        className="preferences-icon"
        style={{ width: size, height: size }}
        name={`icon-preferences-${tabId.toLowerCase().replace(/ /g, '-')}.png`}
        fallback="icon-preferences-plugins.png"
        mode={RetinaImg.Mode.ContentPreserve}
      />
    );
  }
  const glyph = Math.round(size * 0.62);
  return (
    <span
      className="preferences-icon"
      style={{ width: size, height: size, borderRadius: size * 0.27, background: known.color }}
      aria-hidden
    >
      <svg width={glyph} height={glyph} viewBox="0 0 24 24">
        {known.glyph}
      </svg>
    </span>
  );
}
