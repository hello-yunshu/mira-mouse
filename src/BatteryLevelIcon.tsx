// SPDX-License-Identifier: AGPL-3.0-or-later
import { useId } from 'react';

interface BatteryLevelIconProps {
  percentage?: number;
  charging?: boolean;
  className?: string;
}

function batteryFillPercentage(percentage?: number): number {
  if (percentage === undefined || Number.isNaN(percentage)) return 0;
  return Math.max(0, Math.min(100, percentage));
}

export function BatteryLevelIcon({ percentage, charging = false, className = '' }: BatteryLevelIconProps) {
  const fillPercentage = batteryFillPercentage(percentage);
  const tone = charging ? 'charging' : (percentage ?? 100) <= 20 ? 'low' : 'normal';
  const rawMaskId = useId();
  const maskId = `battery-level-mask-${rawMaskId.replace(/[^a-zA-Z0-9_-]/g, '')}`;
  const clipId = `${maskId}-clip`;
  const fillWidth = (16 * fillPercentage) / 100;

  return (
    <span
      className={`battery-level-icon ${tone} ${className}`.trim()}
      aria-hidden="true"
    >
      <svg className="battery-level-svg" viewBox="0 0 28 16" focusable="false">
        <defs>
          <clipPath id={clipId}>
            <rect x="4" y="5" width="16" height="6" rx="2" />
          </clipPath>
          <mask id={maskId} maskUnits="userSpaceOnUse">
            <rect x="0" y="0" width="28" height="16" fill="white" />
            {charging && (
              <path
                className="battery-level-bolt-gap"
                d="M21.1 .6l-5 6h6.2L9.8 15.5l3.2-6.9H6.4z"
                fill="black"
              />
            )}
          </mask>
        </defs>
        <g mask={`url(#${maskId})`}>
          <rect className="battery-level-body" x="1" y="2" width="22" height="12" rx="3.6" />
          <rect
            className="battery-level-fill"
            x="4"
            y="5"
            width={fillWidth}
            height="6"
            rx="2"
            clipPath={`url(#${clipId})`}
          />
          <rect className="battery-level-cap" x="24" y="5.5" width="3" height="5" rx="1.5" />
        </g>
        {charging && (
          <path
            className="battery-level-bolt"
            d="M20.1 1.8l-4.2 5.6h5.1l-9.7 6.8l2.8-6.2H8.3z"
          />
        )}
      </svg>
    </span>
  );
}
