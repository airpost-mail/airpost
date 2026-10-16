import { CapabilityStore } from 'mailspring-exports';
import {
  isTrackingServerAvailable,
  trackingUnavailableReason,
} from '../lib/open-tracking-constants';

describe('Open tracking and the capability store', () => {
  it('stays off with the store reason', () => {
    expect(isTrackingServerAvailable()).toBe(false);
    expect(trackingUnavailableReason()).toBe('Open tracking: Needs a Tern server');
  });

  it('needs a tracking URL as well as the server capability', () => {
    spyOn(CapabilityStore, 'isAvailable').andReturn(true);
    expect(isTrackingServerAvailable()).toBe(false);
  });
});
