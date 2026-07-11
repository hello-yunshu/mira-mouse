// SPDX-License-Identifier: AGPL-3.0-or-later
import { useId } from 'react';
import { LIGHTNING_EXACT_MATCH_PATH } from './assets/lightningExactMatchPath';

interface BatteryLevelIconProps {
  percentage?: number;
  charging?: boolean;
  className?: string;
}

function batteryFillPercentage(percentage?: number): number {
  if (percentage === undefined || Number.isNaN(percentage)) return 0;
  return Math.max(0, Math.min(100, percentage));
}

const CHARGING_BOLT_PATH = LIGHTNING_EXACT_MATCH_PATH;
const CHARGING_BOLT_TRANSFORM = 'translate(8.6 .3) scale(.358)';

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
                d={CHARGING_BOLT_PATH}
                transform={CHARGING_BOLT_TRANSFORM}
                fill="black"
                stroke="black"
                strokeWidth="3.4"
                strokeLinejoin="round"
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
            d={CHARGING_BOLT_PATH}
            transform={CHARGING_BOLT_TRANSFORM}
            fill="currentColor"
          />
        )}
      </svg>
    </span>
  );
}
