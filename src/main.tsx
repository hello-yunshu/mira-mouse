// SPDX-License-Identifier: AGPL-3.0-or-later
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './i18n';
import App from './App';
import { MiraActivityOverlay } from './activity';
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <>
      <App />
      <MiraActivityOverlay />
    </>
  </StrictMode>,
);

