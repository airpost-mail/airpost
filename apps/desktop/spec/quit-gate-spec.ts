// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { QuitGate } from '../src/main/engine-supervisor';
import { tick } from './fake-serve-engine';

describe('QuitGate', function () {
  const quitEvent = () => ({ preventDefault: jasmine.createSpy('preventDefault') });

  let gate: QuitGate = null;
  let stopped: () => void = null;
  let stop: jasmine.Spy = null;
  let quit: jasmine.Spy = null;

  beforeEach(function () {
    gate = new QuitGate();
    stop = jasmine
      .createSpy('stop')
      .andCallFake(() => new Promise<void>((resolve) => (stopped = resolve)));
    quit = jasmine.createSpy('quit');
  });

  it('holds will-quit until the engine stopped, then lets the next one through', async function () {
    const first = quitEvent();
    expect(gate.hold(first, stop, quit)).toBe(true);
    expect(first.preventDefault).toHaveBeenCalled();
    expect(gate.deferLaunch()).toBe(true);

    // A second Cmd+Q while waiting doesn't stop the engine twice.
    const second = quitEvent();
    gate.hold(second, stop, quit);
    expect(second.preventDefault).toHaveBeenCalled();
    expect(stop.calls.length).toBe(1);
    expect(quit).not.toHaveBeenCalled();

    stopped();
    await tick();
    expect(quit.calls.length).toBe(1);

    const after = quitEvent();
    expect(gate.hold(after, stop, quit)).toBe(false);
    expect(after.preventDefault).not.toHaveBeenCalled();
  });

  it('keeps launches that arrive while quitting for a relaunch', async function () {
    expect(gate.deferLaunch(['mailto:a@b.c'])).toBe(false);
    gate.hold(quitEvent(), stop, quit);
    expect(gate.deferLaunch(['mailto:x@y.z'])).toBe(true);
    expect(gate.deferLaunch(['/tmp/file.pdf'])).toBe(true);
    stopped();
    await tick();
    expect(quit).toHaveBeenCalledWith(['mailto:x@y.z', '/tmp/file.pdf']);
  });

  it('quits without a relaunch when nothing arrived', async function () {
    gate.hold(quitEvent(), stop, quit);
    stopped();
    await tick();
    expect(quit).toHaveBeenCalledWith(null);
  });

  it('defers launches during a reset relaunch and hands them over once', function () {
    gate.relaunching = true;
    expect(gate.deferLaunch(['mailto:x@y.z'])).toBe(true);
    expect(gate.deferLaunch()).toBe(true);
    expect(gate.takeLaunchArgs()).toEqual(['mailto:x@y.z']);
    expect(gate.takeLaunchArgs()).toBe(null);
  });
});
