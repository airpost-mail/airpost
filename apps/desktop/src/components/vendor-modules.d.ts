// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

declare module 'chromium-net-errors' {
  class ChromiumNetError extends Error {
    type: string;
  }
  const networkErrors: {
    ChromiumNetError: typeof ChromiumNetError;
    // Returns a plain ChromiumNetError for codes it doesn't know.
    createByCode(code: number): ChromiumNetError;
  };
  export = networkErrors;
}
