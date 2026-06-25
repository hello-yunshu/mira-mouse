// SPDX-License-Identifier: AGPL-3.0-or-later
import type { AnchorHTMLAttributes, MouseEvent, ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { notifyError } from './notify';

type ExternalLinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, 'href' | 'rel' | 'target'> & {
  children: ReactNode;
  errorTitle: string;
  href: string;
};

function isTauriRuntime() {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function ExternalLink({ children, errorTitle, href, onClick, ...props }: ExternalLinkProps) {
  async function handleClick(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (event.defaultPrevented || !isTauriRuntime()) return;
    event.preventDefault();
    try {
      await invoke('open_external_url', { url: href });
    } catch (error) {
      notifyError(errorTitle, String(error));
    }
  }

  return (
    <a {...props} href={href} target="_blank" rel="noopener noreferrer" onClick={handleClick}>
      {children}
    </a>
  );
}
