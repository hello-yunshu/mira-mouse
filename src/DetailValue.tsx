// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import i18n from './i18n';

const MAX_DEPTH = 3;
const MAX_VISIBLE_ITEMS = 8;
const MAX_EXPANDED_ITEMS = 64;
const MAX_BYTE_PREVIEW = 16;

function isByteArray(value: unknown[]): boolean {
  return value.length > 0 && value.every((item) => typeof item === 'number' && item >= 0 && item <= 255 && Number.isInteger(item));
}

function isRgbColor(value: unknown): boolean {
  if (typeof value === 'string') {
    return /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(value.trim());
  }
  if (Array.isArray(value) && value.length === 3 && value.every((item) => typeof item === 'number' && item >= 0 && item <= 255)) {
    return true;
  }
  return false;
}

function rgbToHex(rgb: number[]): string {
  return '#' + rgb.map((v) => v.toString(16).padStart(2, '0')).join('');
}

function formatBytesHex(bytes: number[], max: number): string {
  return bytes.slice(0, max).map((b) => b.toString(16).padStart(2, '0')).join(' ');
}

function formatBytesDec(bytes: number[], max: number): string {
  return bytes.slice(0, max).join(', ');
}

function copyToClipboard(text: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
}

/** Render a structured value with support for objects, arrays, byte arrays, and RGB colors. */
export function DetailValue({ value, depth = 0 }: { value: unknown; depth?: number }) {
  if (value === null || value === undefined) {
    return <span className="detail-null">{i18n.t('common.notReported')}</span>;
  }

  if (typeof value === 'string') {
    if (isRgbColor(value)) {
      return <ColorSwatch hex={value.trim()} />;
    }
    return <span className="detail-string">{value}</span>;
  }

  if (typeof value === 'number') {
    return <span className="detail-number">{value}</span>;
  }

  if (typeof value === 'boolean') {
    return <span className="detail-boolean">{value ? i18n.t('common.on') : i18n.t('common.off')}</span>;
  }

  if (Array.isArray(value)) {
    return <ArrayValue items={value} depth={depth} />;
  }

  if (typeof value === 'object') {
    return <ObjectValue obj={value as Record<string, unknown>} depth={depth} />;
  }

  return <span className="detail-fallback">{String(value)}</span>;
}

function ColorSwatch({ hex }: { hex: string }) {
  return (
    <span className="detail-color">
      <span className="color-swatch" style={{ background: hex }} aria-hidden="true" />
      <span className="color-hex">{hex}</span>
    </span>
  );
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  const onClick = useCallback(() => {
    copyToClipboard(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }, [text]);
  return (
    <button className="detail-copy-btn" onClick={onClick} title={label} aria-label={label}>
      {copied ? '✓' : '⧉'}
    </button>
  );
}

function ArrayValue({ items, depth }: { items: unknown[]; depth: number }) {
  const [expanded, setExpanded] = useState(false);
  const [hexMode, setHexMode] = useState(false);
  const { t } = useTranslation();

  if (items.length === 0) {
    return <span className="detail-empty-array">[]</span>;
  }

  // Byte array special rendering
  if (isByteArray(items)) {
    const bytes = items as number[];
    const preview = hexMode
      ? formatBytesHex(bytes, MAX_BYTE_PREVIEW)
      : formatBytesDec(bytes, MAX_BYTE_PREVIEW);
    const truncated = items.length > MAX_BYTE_PREVIEW;
    return (
      <span className="detail-byte-array">
        <button
          className="byte-toggle-btn"
          onClick={() => setHexMode((v) => !v)}
          title={t('dashboard.toggleHexDec')}
        >
          {hexMode ? '0x' : '10'}
        </button>
        <span className="byte-preview">{preview}{truncated ? ` … (${items.length})` : ''}</span>
        {items.length > 0 && <CopyButton text={JSON.stringify(items)} label={t('common.copy')} />}
      </span>
    );
  }

  // RGB array special rendering
  if (items.length > 0 && items.every((item) => Array.isArray(item) && isRgbColor(item))) {
    return (
      <span className="detail-rgb-array">
        {items.slice(0, MAX_EXPANDED_ITEMS).map((item, idx) => (
          <ColorSwatch key={idx} hex={rgbToHex(item as number[])} />
        ))}
        {items.length > MAX_EXPANDED_ITEMS && <span className="detail-more"> +{items.length - MAX_EXPANDED_ITEMS}</span>}
      </span>
    );
  }

  if (depth >= MAX_DEPTH) {
    return <span className="detail-truncated">[…{items.length}]</span>;
  }

  const visibleCount = expanded ? Math.min(items.length, MAX_EXPANDED_ITEMS) : Math.min(items.length, MAX_VISIBLE_ITEMS);
  const hasMore = items.length > MAX_VISIBLE_ITEMS;

  return (
    <span className="detail-array">
      <span className="array-summary">{t('dashboard.arrayCount', { count: items.length })}</span>
      <span className="array-items">
        {items.slice(0, visibleCount).map((item, idx) => (
          <span key={idx} className="array-item">
            <DetailValue value={item} depth={depth + 1} />
            {idx < visibleCount - 1 ? ', ' : ''}
          </span>
        ))}
        {hasMore && !expanded && <span className="detail-more"> +{items.length - MAX_VISIBLE_ITEMS}</span>}
      </span>
      {hasMore && (
        <button className="detail-expand-btn" onClick={() => setExpanded((v) => !v)}>
          {expanded ? t('common.collapse') : t('common.expand')}
        </button>
      )}
    </span>
  );
}

function ObjectValue({ obj, depth }: { obj: Record<string, unknown>; depth: number }) {
  const [expanded, setExpanded] = useState(depth < 2);
  const { t } = useTranslation();
  const entries = Object.entries(obj).filter(([, v]) => v !== undefined);

  if (entries.length === 0) {
    return <span className="detail-empty-object">{'{}'}</span>;
  }

  if (depth >= MAX_DEPTH) {
    return <span className="detail-truncated">{'{…}'}</span>;
  }

  const visibleCount = expanded ? Math.min(entries.length, MAX_EXPANDED_ITEMS) : Math.min(entries.length, MAX_VISIBLE_ITEMS);
  const hasMore = entries.length > MAX_VISIBLE_ITEMS;

  return (
    <span className={`detail-object ${expanded ? 'expanded' : 'collapsed'}`}>
      {!expanded && <span className="object-summary">{t('dashboard.objectCount', { count: entries.length })}</span>}
      {expanded && (
        <dl className="object-fields">
          {entries.slice(0, visibleCount).map(([key, val]) => (
            <div key={key} className="object-field">
              <dt className="object-key">{key}</dt>
              <dd className="object-value">
                <DetailValue value={val} depth={depth + 1} />
              </dd>
            </div>
          ))}
          {hasMore && !expanded && <span className="detail-more"> +{entries.length - MAX_VISIBLE_ITEMS}</span>}
        </dl>
      )}
      {hasMore && (
        <button className="detail-expand-btn" onClick={() => setExpanded((v) => !v)}>
          {expanded ? t('common.collapse') : t('common.expand')}
        </button>
      )}
    </span>
  );
}
