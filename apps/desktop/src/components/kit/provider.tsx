// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { DirectionProvider } from '@radix-ui/react-direction';
import { isRTL } from '../../intl/localize';

/**
 * Wraps every ReactDOM.render root so Radix primitives (and their portalled content, which keeps
 * React context) resolve the UI direction. A new ReactDOM.render call has to wrap its tree in it
 * and be added to the allowlist in tests/components/kit-provider.test.tsx.
 */
export function UIProvider({ children }: { children?: React.ReactNode }) {
  return <DirectionProvider dir={isRTL ? 'rtl' : 'ltr'}>{children}</DirectionProvider>;
}
