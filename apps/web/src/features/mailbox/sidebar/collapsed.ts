import { useState } from 'react';

const KEY = 'tern.sidebar.collapsed';

function load(): Record<string, boolean> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, boolean>;
  } catch {
    return {};
  }
}

/** A section's collapsed flag, remembered per browser. */
export function useCollapsed(id: string): [boolean, () => void] {
  const [collapsed, setCollapsed] = useState(() => load()[id] ?? false);
  const toggle = () => {
    const next = !collapsed;
    setCollapsed(next);
    try {
      localStorage.setItem(KEY, JSON.stringify({ ...load(), [id]: next }));
    } catch {
      // Private mode: keep it for this session only.
    }
  };
  return [collapsed, toggle];
}
