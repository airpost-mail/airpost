import { categoryDisplayName, isStandardCategory, type CategoryJSON } from '@/engine';

/**
 * Label hue from its name, the desktop's `Category.hue()` formula, so a label keeps its
 * colour across clients. The engine has no label colours.
 */
export function labelHue(cat: Pick<CategoryJSON, 'path' | 'role'>): number {
  const name = categoryDisplayName(cat);
  let hue = 0;
  for (let i = 0; i < name.length; i++) hue += name.charCodeAt(i);
  return (hue * 396) / 512;
}

/** User labels worth showing on a thread: not system roles, not the one being viewed. */
export function visibleLabels<T extends Pick<CategoryJSON, 'id' | 'role'>>(
  labels: T[],
  viewingId?: string,
): T[] {
  return labels.filter((l) => !isStandardCategory(l) && l.id !== viewingId);
}
