import { CapabilityStore } from 'mailspring-exports';
import { isTrackingServerAvailable, trackingUnavailableReason } from '../lib/constants';

describe('Link tracking and the capability store', () => {
  it('stays off with the store reason', () => {
    expect(isTrackingServerAvailable()).toBe(false);
    expect(trackingUnavailableReason()).toBe('Link tracking: Needs a Tern server');
  });

  it('follows the server capability the engine reports', () => {
    spyOn(CapabilityStore, 'isAvailable').andReturn(true);
    expect(isTrackingServerAvailable()).toBe(true);
  });
});
