// SPDX-License-Identifier: AGPL-3.0-or-later
import type { PropsWithChildren } from 'react';
export function Tooltip({ label, children }: PropsWithChildren<{ label: string }>) {
  return <span className="tooltip" tabIndex={0} aria-label={label}>{children}<span role="tooltip">{label}</span></span>;
}

