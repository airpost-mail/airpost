// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

declare module 'pick-react-known-prop' {
  // Returns the subset of `props` that are known HTML/SVG attributes.
  export function pickHTMLProps(props: object): import('react').AllHTMLAttributes<HTMLElement>;
  export function pickSVGProps(props: object): import('react').SVGAttributes<SVGElement>;
}
