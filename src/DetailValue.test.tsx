// SPDX-License-Identifier: AGPL-3.0-or-later
import { render, fireEvent } from '@testing-library/react';
import { beforeAll, afterAll, describe, expect, it } from 'vitest';
import { DetailValue } from './DetailValue';

beforeAll(() => Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} }));
afterAll(() => Reflect.deleteProperty(window, '__TAURI_INTERNALS__'));

describe('DetailValue', () => {
  it('renders null as not reported', () => {
    const { container } = render(<DetailValue value={null} />);
    expect(container.querySelector('.detail-null')).toBeTruthy();
  });

  it('renders a string', () => {
    const { container } = render(<DetailValue value="hello" />);
    expect(container.querySelector('.detail-string')?.textContent).toBe('hello');
  });

  it('renders a number', () => {
    const { container } = render(<DetailValue value={42} />);
    expect(container.querySelector('.detail-number')?.textContent).toBe('42');
  });

  it('renders a boolean', () => {
    const { container } = render(<DetailValue value={true} />);
    expect(container.querySelector('.detail-boolean')).toBeTruthy();
  });

  it('renders an RGB color string with swatch', () => {
    const { container } = render(<DetailValue value="#aabbcc" />);
    expect(container.querySelector('.color-swatch')).toBeTruthy();
    expect(container.querySelector('.color-hex')?.textContent).toBe('#aabbcc');
  });

  it('renders a byte array with hex/decimal toggle', () => {
    const { container } = render(<DetailValue value={[0, 31, 255, 128]} />);
    expect(container.querySelector('.detail-byte-array')).toBeTruthy();
    // Default decimal mode
    expect(container.querySelector('.byte-preview')?.textContent).toContain('0, 31, 255, 128');
    // Toggle to hex
    fireEvent.click(container.querySelector('.byte-toggle-btn')!);
    expect(container.querySelector('.byte-preview')?.textContent).toContain('00 1f ff 80');
  });

  it('renders an empty array', () => {
    const { container } = render(<DetailValue value={[]} />);
    expect(container.querySelector('.detail-empty-array')).toBeTruthy();
  });

  it('renders a numeric array with item count', () => {
    const { container } = render(<DetailValue value={[100, 200, 300]} />);
    expect(container.querySelector('.detail-array')).toBeTruthy();
    expect(container.querySelector('.array-summary')?.textContent).toContain('3');
  });

  it('truncates huge arrays with expand button', () => {
    // Use values > 255 to avoid byte-array detection
    const big = Array.from({ length: 20 }, (_, i) => (i + 1) * 300);
    const { container } = render(<DetailValue value={big} />);
    expect(container.querySelector('.detail-expand-btn')).toBeTruthy();
    expect(container.querySelector('.detail-more')).toBeTruthy();
  });

  it('renders a nested object as key-value pairs', () => {
    const { container } = render(<DetailValue value={{ name: 'test', version: '1.0' }} />);
    const obj = container.querySelector('.detail-object');
    expect(obj).toBeTruthy();
    expect(container.querySelectorAll('.object-field').length).toBeGreaterThanOrEqual(2);
    expect(container.querySelector('.object-key')?.textContent).toBe('name');
  });

  it('renders an empty object', () => {
    const { container } = render(<DetailValue value={{}} />);
    expect(container.querySelector('.detail-empty-object')).toBeTruthy();
  });

  it('renders an RGB array with color swatches', () => {
    const { container } = render(<DetailValue value={[[255, 0, 0], [0, 255, 0]]} />);
    expect(container.querySelector('.detail-rgb-array')).toBeTruthy();
    expect(container.querySelectorAll('.color-swatch').length).toBe(2);
  });

  it('does not render [object Object] for objects', () => {
    const { container } = render(<DetailValue value={{ foo: 'bar' }} />);
    expect(container.textContent).not.toContain('[object Object]');
  });

  it('does not render comma-only for arrays', () => {
    const { container } = render(<DetailValue value={[1, 2, 3]} />);
    const text = container.textContent ?? '';
    // Should have meaningful content beyond just commas
    expect(text).toContain('1');
    expect(text).toContain('2');
    expect(text).toContain('3');
  });

  it('respects max recursion depth', () => {
    // Render directly at depth = MAX_DEPTH to trigger truncation
    const obj = { a: 'too deep' };
    const { container } = render(<DetailValue value={obj} depth={3} />);
    expect(container.querySelectorAll('.detail-truncated').length).toBeGreaterThan(0);
  });
});
