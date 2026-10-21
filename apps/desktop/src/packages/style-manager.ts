import { Disposable } from 'event-kit';

// A <style> this manager added, with what it was added with.
type ManagedStyleElement = HTMLStyleElement & { sourcePath?: string; priority?: number };

export default class StyleManager {
  private styleElementsBySourcePath: { [sourcePath: string]: ManagedStyleElement } = {};
  private el: HTMLElement;

  constructor() {
    this.el = document.createElement('managed-styles');
    document.head.appendChild(this.el);
  }

  // Only addStyleSheet puts elements into `el`.
  getStyleElements() {
    return Array.from(this.el.children) as ManagedStyleElement[];
  }

  addStyleSheet(
    source: string,
    { sourcePath, priority }: { sourcePath?: string; priority?: number }
  ) {
    let styleElement: ManagedStyleElement | null = sourcePath
      ? this.styleElementsBySourcePath[sourcePath]
      : null;

    if (styleElement) {
      styleElement.textContent = source;
    } else {
      styleElement = document.createElement('style') as ManagedStyleElement;
      if (sourcePath !== undefined) {
        styleElement.sourcePath = sourcePath;
        styleElement.setAttribute('source-path', sourcePath);
        this.styleElementsBySourcePath[sourcePath] = styleElement;
      }
      if (priority !== undefined) {
        styleElement.priority = priority;
        styleElement.setAttribute('priority', String(priority));
      }
      styleElement.textContent = source;
      this.insertStyleElementIntoDOM(styleElement);
    }

    const added = styleElement;
    return new Disposable(() => this.removeStyleElement(added));
  }

  insertStyleElementIntoDOM(styleElement: ManagedStyleElement) {
    const { priority } = styleElement;
    const beforeEl =
      priority !== undefined &&
      this.getStyleElements().find((el) => el.priority !== undefined && el.priority > priority);
    if (!beforeEl) {
      this.el.appendChild(styleElement);
    } else {
      this.el.insertBefore(styleElement, beforeEl);
    }
  }

  removeStyleElement(styleElement: ManagedStyleElement) {
    if (styleElement.sourcePath) {
      delete this.styleElementsBySourcePath[styleElement.sourcePath];
    }
    styleElement.remove();
  }

  getSnapshot() {
    return this.getStyleElements();
  }

  restoreSnapshot(styleElements: ManagedStyleElement[]) {
    for (const el of this.getStyleElements()) {
      this.removeStyleElement(el);
    }
    for (const el of styleElements) {
      const { sourcePath, priority } = el;
      this.addStyleSheet(el.textContent ?? '', { sourcePath, priority });
    }
  }
}
