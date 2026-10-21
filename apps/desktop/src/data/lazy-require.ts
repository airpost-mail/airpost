// Defers a require() to first use, for modules that would otherwise form an import cycle.
// Annotate `load`'s return type so the loaded module stays typed.
export function lazyRequire<T>(load: () => T): () => T {
  let cache: { value: T } | null = null;
  return () => {
    cache = cache || { value: load() };
    return cache.value;
  };
}
