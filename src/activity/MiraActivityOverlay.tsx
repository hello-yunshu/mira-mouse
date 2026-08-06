// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from 'react';
import { ThinkingOrb } from 'thinking-orbs';
import { useTranslation } from 'react-i18next';
import { OverlayPortal } from '../overlay';
import {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
  miraActivityLabel,
  miraActivitySpec,
  resolveGlobalMiraActivity,
  type MiraActivityKind,
} from './activityCatalog';

function useDetectedGlobalActivity(): MiraActivityKind | null {
  const [activity, setActivity] = useState<MiraActivityKind | null>(() => (
    typeof document === 'undefined'
      ? null
      : resolveGlobalMiraActivity(document)
  ));

  useEffect(() => {
    if (typeof document === 'undefined' || !document.body) return;
    if (typeof MutationObserver === 'undefined') return;

    let frame = 0;
    const sync = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const next = resolveGlobalMiraActivity(document);
        setActivity((current) => current === next ? current : next);
      });
    };

    const observer = new MutationObserver(sync);
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['class'],
    });
    sync();

    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, []);

  return activity;
}

/**
 * 全局 64px Orb 的可见状态：延迟出现、最短可见、活动切换即时替换。
 * 所有 setState 都发生在定时器回调里，避免 set-state-in-effect。
 */
function useDisplayedGlobalActivity(
  detected: MiraActivityKind | null,
): MiraActivityKind | null {
  const [displayed, setDisplayed] = useState<MiraActivityKind | null>(detected);
  const visibleSinceRef = useRef(0);

  useEffect(() => {
    let timer = 0;

    if (detected) {
      if (!displayed) {
        timer = window.setTimeout(() => {
          visibleSinceRef.current = performance.now();
          setDisplayed(detected);
        }, Math.max(0, MIRA_ACTIVITY_SHOW_DELAY_MS));
      } else if (displayed !== detected) {
        timer = window.setTimeout(() => setDisplayed(detected), 0);
      }
    } else if (displayed) {
      const elapsed = performance.now() - visibleSinceRef.current;
      timer = window.setTimeout(
        () => setDisplayed(null),
        Math.max(0, MIRA_ACTIVITY_MIN_VISIBLE_MS - elapsed),
      );
    }

    return () => window.clearTimeout(timer);
  }, [detected, displayed]);

  return displayed;
}

export function MiraActivityOverlay() {
  const { i18n } = useTranslation();
  const detected = useDetectedGlobalActivity();
  const displayed = useDisplayedGlobalActivity(detected);

  if (!displayed) return null;

  const spec = miraActivitySpec(displayed);
  const label = miraActivityLabel(
    displayed,
    i18n.resolvedLanguage ?? i18n.language,
  );

  return (
    <OverlayPortal>
      <div
        className="mira-activity-overlay"
        data-mira-activity={displayed}
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        <div className="mira-activity-card">
          <ThinkingOrb
            state={spec.state}
            size={spec.size}
            speed={spec.speed}
            theme="auto"
            aria-hidden="true"
          />
          <span>{label}</span>
        </div>
      </div>
    </OverlayPortal>
  );
}
