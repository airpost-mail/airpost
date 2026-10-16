// Each open composer registers its actions here so the host's shortcuts (registered once,
// listed once in the `?` sheet) can act on whichever window has focus.
export interface WindowActions {
  send(): void;
  escape(): void;
  showCc(): void;
  showBcc(): void;
  discard(): void;
}

const actions = new Map<string, WindowActions>();

export function registerWindowActions(key: string, a: WindowActions): () => void {
  actions.set(key, a);
  return () => {
    if (actions.get(key) === a) actions.delete(key);
  };
}

export function windowActions(key: string | null): WindowActions | undefined {
  return key ? actions.get(key) : undefined;
}
