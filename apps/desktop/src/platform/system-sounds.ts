// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';

/*
The OS's own sounds, played in place from the system's sound directories at runtime: they aren't
ours to redistribute, so none of them ship with Tern. An event whose file is missing falls back
to the bundled sound (src/features/custom-sounds).

macOS (/System/Library/Sounds):
  new-mail  Glass  a light chime, noticeable without being an alert; the user can pick another
                   in Preferences > Notifications
  hit-send  Tink   the shortest and quietest tap, acknowledging the click
  send      Bottle a soft, low "pok" once the message has gone; doesn't compete with new mail
Basso, Funk, Sosumi and Frog read as errors, Hero and Submarine are too big for routine events.

Linux (the freedesktop sound theme, /usr/share/sounds/freedesktop/stereo):
  new-mail  message-new-instant
  send      complete
  hit-send  bundled (the theme has no short click)

Windows (C:\Windows\Media):
  new-mail  Windows Notify Email, the OS's own new-mail sound
  hit-send, send: bundled, Windows has no obvious equivalent
*/

export type SoundEvent = 'new-mail' | 'hit-send' | 'send';

export const MacSystemSoundsDir = '/System/Library/Sounds';
export const DefaultMacNewMailSound = 'Glass';

const MacSounds: Partial<Record<SoundEvent, string>> = {
  'new-mail': DefaultMacNewMailSound,
  'hit-send': 'Tink',
  send: 'Bottle',
};

const FreedesktopSoundsDir = '/usr/share/sounds/freedesktop/stereo';
const FreedesktopSounds: Partial<Record<SoundEvent, string>> = {
  'new-mail': 'message-new-instant.oga',
  send: 'complete.oga',
};

const WindowsSounds: Partial<Record<SoundEvent, string>> = {
  'new-mail': 'Windows Notify Email.wav',
};

function windowsMediaDir() {
  return path.join(process.env.SystemRoot || process.env.windir || 'C:\\Windows', 'Media');
}

/** The directories system sounds may play from (SoundRegistry plays file: URLs inside them). */
export function systemSoundDirs(platform = process.platform): string[] {
  if (platform === 'darwin') return [MacSystemSoundsDir];
  if (platform === 'linux') return [FreedesktopSoundsDir];
  if (platform === 'win32') return [windowsMediaDir()];
  return [];
}

function existing(file: string) {
  return fs.existsSync(file) ? file : null;
}

/** The path of a macOS system sound by name ("Glass"), or null if there is no such sound. */
export function macSystemSoundPath(name: unknown): string | null {
  if (typeof name !== 'string' || !/^[\w -]+$/.test(name)) return null;
  return existing(path.join(MacSystemSoundsDir, `${name}.aiff`));
}

/** The macOS system sound names, sorted, or [] off macOS. */
export function macSystemSoundNames(): string[] {
  if (process.platform !== 'darwin') return [];
  try {
    return fs
      .readdirSync(MacSystemSoundsDir)
      .filter((f) => f.endsWith('.aiff'))
      .map((f) => f.slice(0, -'.aiff'.length))
      .sort((a, b) => a.localeCompare(b));
  } catch {
    return [];
  }
}

/**
 * The system sound file for an event on this platform, or null to use the bundled one.
 * `macName` overrides the macOS pick (the user's choice for new mail).
 */
export function systemSoundPath(event: SoundEvent, macName?: string): string | null {
  if (process.platform === 'darwin') {
    return (macName && macSystemSoundPath(macName)) || macSystemSoundPath(MacSounds[event]);
  }
  if (process.platform === 'linux') {
    const file = FreedesktopSounds[event];
    return file ? existing(path.join(FreedesktopSoundsDir, file)) : null;
  }
  if (process.platform === 'win32') {
    const file = WindowsSounds[event];
    return file ? existing(path.join(windowsMediaDir(), file)) : null;
  }
  return null;
}
