/** Keeps `?redirect=` to paths on this origin so pairing can't bounce users elsewhere. */
export function safeRedirect(value: unknown): string | undefined {
  if (typeof value !== 'string' || !value.startsWith('/') || value.startsWith('//')) {
    return undefined;
  }
  if (value.startsWith('/\\') || value.startsWith('/pair')) return undefined;
  return value;
}
