// SPDX-License-Identifier: AGPL-3.0-or-later
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AboutPage } from './About';

describe('AboutPage', () => {
  it('renders the complete host skeleton in web preview mode', () => {
    render(<AboutPage previewMode onBack={vi.fn()} />);
    expect(screen.getByRole('heading', { name: '关于' })).toBeInTheDocument();
    expect(screen.getByText('0.1.0-preview')).toBeInTheDocument();
    expect(screen.getByText('Web Preview')).toBeInTheDocument();
    expect(screen.queryByText(/加载关于信息失败/)).not.toBeInTheDocument();
  });
});
