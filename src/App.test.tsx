// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import App from './App';

describe('Mira shell', () => {
  it('shows a quiet no-device state without stale numbers', () => {
    render(<App />);
    expect(screen.getByText('未发现受支持的鼠标')).toBeInTheDocument();
    expect(screen.queryByText(/0 DPI|--%/)).not.toBeInTheDocument();
  });
  it('renders capability data and labels the application-layer link', () => {
    render(<App />);
    fireEvent.click(screen.getByText('打开 Fixture 演示'));
    expect(screen.getByText('82%')).toBeInTheDocument();
    expect(screen.getByText('1000 DPI')).toBeInTheDocument();
    expect(screen.getByText(/不是接收器原生功能/)).toBeInTheDocument();
    expect(screen.getByText('fixture-verified')).toBeInTheDocument();
  });
});

