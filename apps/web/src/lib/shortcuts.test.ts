import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  formatShortcut,
  installShortcuts,
  normalizeChord,
  pushShortcutScope,
  registerShortcut,
  resetShortcuts,
} from './shortcuts';

function press(key: string, init: KeyboardEventInit = {}, target: EventTarget = document.body) {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
}

describe('shortcuts', () => {
  let uninstall: () => void;
  beforeEach(() => {
    uninstall = installShortcuts(window);
  });
  afterEach(() => {
    uninstall();
    resetShortcuts();
    document.body.innerHTML = '';
  });

  it('normalises chords', () => {
    expect(normalizeChord('Shift+Ctrl+K')).toBe('ctrl+shift+k');
    expect(normalizeChord('esc')).toBe('escape');
  });

  it('fires single keys and prevents default', () => {
    const fn = vi.fn();
    registerShortcut({ keys: 'j', description: 'next', handler: fn });
    const e = press('j');
    expect(fn).toHaveBeenCalledOnce();
    expect(e.defaultPrevented).toBe(true);
  });

  it('matches punctuation by character regardless of shift', () => {
    const fn = vi.fn();
    registerShortcut({ keys: '#', description: 'trash', handler: fn });
    press('#', { shiftKey: true });
    expect(fn).toHaveBeenCalledOnce();
  });

  it('distinguishes shift on letters', () => {
    const lower = vi.fn();
    const upper = vi.fn();
    registerShortcut({ keys: 'u', description: 'a', handler: lower });
    registerShortcut({ keys: 'shift+u', description: 'b', handler: upper });
    press('U', { shiftKey: true });
    expect(upper).toHaveBeenCalledOnce();
    expect(lower).not.toHaveBeenCalled();
  });

  it('handles sequences', () => {
    const fn = vi.fn();
    registerShortcut({ keys: 'g i', description: 'inbox', handler: fn });
    press('g');
    expect(fn).not.toHaveBeenCalled();
    press('i');
    expect(fn).toHaveBeenCalledOnce();
  });

  it('ignores keys typed into inputs unless allowed', () => {
    const plain = vi.fn();
    const allowed = vi.fn();
    registerShortcut({ keys: 'c', description: 'compose', handler: plain });
    registerShortcut({
      keys: 'escape',
      description: 'close',
      handler: allowed,
      allowInInputs: true,
    });
    const input = document.createElement('input');
    document.body.append(input);
    press('c', {}, input);
    press('Escape', {}, input);
    expect(plain).not.toHaveBeenCalled();
    expect(allowed).toHaveBeenCalledOnce();
  });

  it('leaves Enter and Space on a focused control to that control', () => {
    const open = vi.fn();
    const next = vi.fn();
    registerShortcut({ keys: 'enter', description: 'open', handler: open });
    registerShortcut({ keys: 'j', description: 'next', handler: next });
    const button = document.createElement('button');
    document.body.append(button);
    expect(press('Enter', {}, button).defaultPrevented).toBe(false);
    press('j', {}, button);
    expect(open).not.toHaveBeenCalled();
    expect(next).toHaveBeenCalledOnce();
    press('Enter');
    expect(open).toHaveBeenCalledOnce();
  });

  it('keeps global shortcuts away from keys pressed inside a menu or dialog', () => {
    const archive = vi.fn();
    const help = vi.fn();
    registerShortcut({ keys: 'e', description: 'archive', handler: archive });
    registerShortcut({ keys: '?', description: 'help', handler: help, scope: '*' });
    const menu = document.createElement('div');
    menu.setAttribute('role', 'menu');
    const item = document.createElement('div');
    item.setAttribute('role', 'menuitem');
    menu.append(item);
    document.body.append(menu);
    press('e', {}, item);
    press('?', { shiftKey: true }, item);
    expect(archive).not.toHaveBeenCalled();
    expect(help).toHaveBeenCalledOnce();
  });

  it('newest registration wins and unregistering restores the previous one', () => {
    const first = vi.fn();
    const second = vi.fn();
    registerShortcut({ keys: 'e', description: 'a', handler: first });
    const off = registerShortcut({ keys: 'e', description: 'b', handler: second });
    press('e');
    off();
    press('e');
    expect(second).toHaveBeenCalledOnce();
    expect(first).toHaveBeenCalledOnce();
  });

  it('only fires shortcuts of the active scope', () => {
    const global = vi.fn();
    const dialog = vi.fn();
    const always = vi.fn();
    registerShortcut({ keys: 'j', description: 'g', handler: global });
    registerShortcut({ keys: 'j', description: 'd', handler: dialog, scope: 'dialog' });
    registerShortcut({ keys: '?', description: 'help', handler: always, scope: '*' });
    const pop = pushShortcutScope('dialog');
    press('j');
    press('?');
    pop();
    press('j');
    expect(dialog).toHaveBeenCalledOnce();
    expect(global).toHaveBeenCalledOnce();
    expect(always).toHaveBeenCalledOnce();
  });

  it('formats bindings for display', () => {
    expect(formatShortcut('g i')).toEqual(['G', 'then', 'I']);
    expect(formatShortcut('shift+j').at(-1)).toBe('J');
  });
});
