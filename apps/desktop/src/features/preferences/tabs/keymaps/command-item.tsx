import React from 'react';
import ReactDOM from 'react-dom';
import classnames from 'classnames';
import { localized } from '../../../../intl/localize';
import fs from 'fs';

import { keyAndModifiersForEvent } from './mousetrap-keybinding-helpers';

interface CommandKeybindingProps {
  bindings: string[];
  label: string;
  command: string;
}
interface CommandKeybindingState {
  editing: boolean;
  editingBinding?: string | null;
  modifiers: string[];
  keys: string[];
}

export default class CommandKeybinding extends React.Component<
  CommandKeybindingProps,
  CommandKeybindingState
> {
  _mounted = false;

  constructor(props: CommandKeybindingProps) {
    super(props);

    // _onEdit resets keys and modifiers; they are only read while editing.
    this.state = {
      editing: false,
      keys: [],
      modifiers: [],
    };
  }

  componentDidMount() {
    this._mounted = true;
  }

  componentDidUpdate() {
    const { modifiers, keys, editing } = this.state;
    if (editing) {
      const finished = (modifiers.length > 0 && keys.length > 0) || keys.length >= 2;
      if (finished) {
        (ReactDOM.findDOMNode(this) as HTMLElement).blur();
      }
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _formatKeystrokes(original: string) {
    // macOS shows menu-bar glyphs (⌘⇧D); Windows and Linux spell shortcuts out
    // (Ctrl+Shift+D), and their users don't read ^ or ⌥ as modifier keys.
    const isMac = process.platform === 'darwin';
    const modifiers: [RegExp, string][] = isMac
      ? [
          [/\+(?!$)/gi, ''],
          [/command/gi, '⌘'],
          [/meta/gi, '⌘'],
          [/alt/gi, '⌥'],
          [/shift/gi, '⇧'],
          [/ctrl/gi, '^'],
          [/mod/gi, '⌘'],
        ]
      : [
          [/alt/gi, 'Alt'],
          [/shift/gi, 'Shift'],
          [/ctrl/gi, 'Ctrl'],
          [/mod/gi, 'Ctrl'],
        ];
    let clean = original;
    for (const [regexp, char] of modifiers) {
      clean = clean.replace(regexp, char);
    }

    if (isMac) {
      // ⌘⇧c => ⌘⇧C
      if (clean !== original) {
        clean = clean.toUpperCase();
      }
      // backspace => Backspace
      if (original.length > 1 && clean === original) {
        clean = clean[0].toUpperCase() + clean.slice(1);
      }
      return clean;
    }

    // ctrl+shift+d => Ctrl+Shift+D, alt+backspace => Alt+Backspace
    return clean
      .split('+')
      .map((part) => (part.length > 0 ? part[0].toUpperCase() + part.slice(1) : part))
      .join('+');
  }

  /** "⌘⇧C" or "Ctrl+Shift+C" as one chip per key. */
  _keyParts(formatted: string) {
    if (process.platform !== 'darwin') return formatted.split(/\+(?!$)/);
    const parts: string[] = [];
    let rest = formatted;
    while (rest.length > 1 && '⌘⌥⇧^'.includes(rest[0])) {
      parts.push(rest[0]);
      rest = rest.slice(1);
    }
    return [...parts, rest];
  }

  _renderKeystrokes = (keystrokes: string, idx: number) => {
    const elements: JSX.Element[] = [];
    const splitKeystrokes = keystrokes.split(' ');
    splitKeystrokes.forEach((keystroke, kidx) => {
      for (const [pidx, part] of this._keyParts(this._formatKeystrokes(keystroke)).entries()) {
        elements.push(
          <kbd key={`${kidx}-${pidx}`} className="shortcut-key">
            {part.length === 1 ? part.toUpperCase() : part}
          </kbd>
        );
      }
      if (kidx < splitKeystrokes.length - 1) {
        elements.push(
          <span className="then" key={`then${kidx}`}>
            {localized('then')}
          </span>
        );
      }
    });
    return (
      <span key={`keystrokes-${idx}`} className="shortcut-value">
        {idx > 0 && <span className="or">{localized('or')}</span>}
        {elements}
      </span>
    );
  };

  _onEdit = () => {
    this.setState({ editing: true, editingBinding: null, keys: [], modifiers: [] });
    AppEnv.keymaps.suspendAllKeymaps();
  };

  _onFinishedEditing = () => {
    if (this.state.editingBinding) {
      const keymapPath = AppEnv.keymaps.getUserKeymapPath();
      let keymaps: { [command: string]: string } = {};

      try {
        const exists = fs.existsSync(keymapPath);
        if (exists) {
          keymaps = JSON.parse(fs.readFileSync(keymapPath).toString());
        }
      } catch (err) {
        console.error(err);
      }

      keymaps[this.props.command] = this.state.editingBinding;

      try {
        fs.writeFileSync(keymapPath, JSON.stringify(keymaps, null, 2));
      } catch (err) {
        AppEnv.showErrorDialog(
          localized(`Airpost was unable to modify your keymaps at %@.`, keymapPath) + ' ' + String(err)
        );
      }
    }

    AppEnv.keymaps.resumeAllKeymaps();

    setTimeout(() => {
      if (!this._mounted) return;
      this.setState({ editing: false, editingBinding: null });
    }, 100);
  };

  _onKey = (event: React.KeyboardEvent<HTMLElement>) => {
    if (!this.state.editing) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    const [eventKey, eventMods] = keyAndModifiersForEvent(event);
    if (!eventKey || ['mod', 'meta', 'command', 'ctrl', 'alt', 'shift'].includes(eventKey)) {
      return;
    }

    let { keys, modifiers } = this.state;
    keys = keys.concat([eventKey]);
    modifiers = [...new Set(modifiers.concat(eventMods))];

    let editingBinding = keys.join(' ');
    if (modifiers.length > 0) {
      editingBinding = [...modifiers, ...keys].join('+');
      if (process.platform === 'darwin') {
        editingBinding = editingBinding.replace(/meta/g, 'mod');
      } else {
        editingBinding = editingBinding.replace(/ctrl/g, 'mod');
      }
    }

    this.setState({ keys, modifiers, editingBinding });
  };

  render() {
    const { editing, editingBinding } = this.state;
    const bindings = editingBinding ? [editingBinding] : this.props.bindings;

    let value: React.ReactNode = <span className="shortcut-none">{localized('None')}</span>;
    if (editing && !editingBinding) {
      value = <span className="shortcut-prompt">{localized('Press the new keys')}</span>;
    } else if (bindings.length > 0) {
      // Templates may list mod+a and ctrl+a for one command; they are the same key
      // on Windows and Linux, so dedupe by what the user would actually press.
      const mod = process.platform === 'darwin' ? 'command' : 'ctrl';
      const byKey = new Map(bindings.map((b) => [b.replace(/\bmod\b/g, mod), b]));
      value = [...byKey.values()].map(this._renderKeystrokes);
    }

    return (
      <div
        className={classnames('settings-row shortcut-row', { editing })}
        tabIndex={-1}
        aria-label={localized('Change the shortcut for %@', this.props.label)}
        onKeyDown={this._onKey}
        onKeyPress={this._onKey}
        onFocus={this._onEdit}
        onBlur={this._onFinishedEditing}
      >
        <div className="settings-row-label">
          <div className="settings-row-title">{this.props.label}</div>
        </div>
        <div className="settings-row-control shortcut-values">{value}</div>
      </div>
    );
  }
}
