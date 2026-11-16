import { useState } from 'react';

const KEY = 'airpost.sidebar.collapsed';

function load(): Record<string, boolean> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, boolean>;
  } catch {
    return {};
  }
}

/** A section's collapsed flag, remembered per browser once toggled. */
export function useCollapsed(id: string, initial = false): [boolean, () => void] {
  const [collapsed, setCollapsed] = useState(() => load()[id] ?? initial);
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
