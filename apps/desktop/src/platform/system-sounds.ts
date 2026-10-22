// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';

/*
The OS's own sounds, played in place from the system's sound directories at runtime: they aren't
ours to redistribute, so none of them ship with Tern. An event whose file is missing falls back
to the bundled sound (src/features/custom-sounds).

macOS plays what Apple Mail plays, from Mail.app's Resources:
  new-mail  New Mail   Mail's default new message sound; the user can pick a sound from
                       /System/Library/Sounds instead in Preferences > Notifications
  hit-send  Mail Sent  the whoosh, played on the click as Mail plays it when the compose window
                       goes, not after the undo-send delay
  send      none       Mail plays nothing more once the message has gone
Mail's other sounds (Mail Fetch Error, Mail Scheduled) have no event here yet.

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
export const MacMailSoundsDir = '/System/Applications/Mail.app/Contents/Resources';
export const DefaultMacNewMailSound = 'New Mail';

// Sounds by name in MacMailSoundsDir; every other name is looked up in MacSystemSoundsDir.
const MacMailSounds = ['New Mail', 'Mail Sent'];

// null: the event is silent on macOS, as in Apple Mail. New mail: macNewMailSoundPath.
const MacSounds: Record<Exclude<SoundEvent, 'new-mail'>, string | null> = {
  'hit-send': 'Mail Sent',
  send: null,
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
  if (platform === 'darwin') return [MacSystemSoundsDir, MacMailSoundsDir];
  if (platform === 'linux') return [FreedesktopSoundsDir];
  if (platform === 'win32') return [windowsMediaDir()];
  return [];
}

function existing(file: string) {
  return fs.existsSync(file) ? file : null;
}

/**
 * The path of a macOS sound by name ("Glass", or Mail's "New Mail"), or null if there is no
 * such sound.
 */
export function macSystemSoundPath(name: unknown): string | null {
  if (typeof name !== 'string' || !/^[\w -]+$/.test(name)) return null;
  const dir = MacMailSounds.includes(name) ? MacMailSoundsDir : MacSystemSoundsDir;
  return existing(path.join(dir, `${name}.aiff`));
}

/** The new-mail sound file on macOS: the user's pick, else Mail's New Mail, else Glass. */
export function macNewMailSoundPath(picked: unknown): string | null {
  return (
    macSystemSoundPath(picked) ||
    macSystemSoundPath(DefaultMacNewMailSound) ||
    macSystemSoundPath('Glass')
  );
}

/**
 * True if NotificationCenter can play the file by name, which it only looks up in the
 * Library/Sounds directories and the app bundle, not in Mail.app.
 */
export function playsFromNotificationCenter(file: string | null): file is string {
  return !!file && path.dirname(file) === MacSystemSoundsDir;
}

/** The sounds the user can pick for new mail on macOS (Mail's first), or [] off macOS. */
export function macSystemSoundNames(): string[] {
  if (process.platform !== 'darwin') return [];
  let system: string[] = [];
  try {
    system = fs
      .readdirSync(MacSystemSoundsDir)
      .filter((f) => f.endsWith('.aiff'))
      .map((f) => f.slice(0, -'.aiff'.length))
      .sort((a, b) => a.localeCompare(b));
  } catch {
    // no system sounds
  }
  const mail = macSystemSoundPath(DefaultMacNewMailSound) ? [DefaultMacNewMailSound] : [];
  return [...mail, ...system];
}

/**
 * The system sound file for an event on this platform, null to use the bundled one, or false
 * when the platform's mail app plays nothing for it. `macName` is the user's new-mail pick.
 */
export function systemSoundPath(event: SoundEvent, macName?: unknown): string | null | false {
  if (process.platform === 'darwin') {
    if (event === 'new-mail') return macNewMailSoundPath(macName);
    const name = MacSounds[event];
    return name === null ? false : macSystemSoundPath(name);
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
