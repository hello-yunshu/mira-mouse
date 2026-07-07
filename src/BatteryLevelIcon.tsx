// SPDX-License-Identifier: AGPL-3.0-or-later

interface BatteryLevelIconProps {
  percentage?: number;
  charging?: boolean;
  className?: string;
}

const BATTERY_LEVEL_CELL_COUNT = 4;

function batteryLevelCells(percentage?: number): number {
  if (percentage === undefined || Number.isNaN(percentage)) return 0;
  const clamped = Math.max(0, Math.min(100, percentage));
  if (clamped === 0) return 0;
  return Math.max(1, Math.ceil((clamped / 100) * BATTERY_LEVEL_CELL_COUNT));
}

export function BatteryLevelIcon({ percentage, charging = false, className = '' }: BatteryLevelIconProps) {
  const filledCells = batteryLevelCells(percentage);
  const tone = charging ? 'charging' : (percentage ?? 100) <= 20 ? 'low' : 'normal';

  return (
    <span
      className={`battery-level-icon ${tone} ${className}`.trim()}
      aria-hidden="true"
    >
      <span className="battery-level-body">
        {Array.from({ length: BATTERY_LEVEL_CELL_COUNT }, (_, index) => (
          <span
            key={index}
            className={`battery-level-cell ${index < filledCells ? 'filled' : ''}`}
          />
        ))}
      </span>
      <span className="battery-level-cap" />
    </span>
  );
}
