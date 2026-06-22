// SPDX-License-Identifier: AGPL-3.0-or-later
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

Object.defineProperty(window, '__TAURI_INTERNALS__', {
  configurable: true,
  value: {},
});

afterEach(cleanup);
