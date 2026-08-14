// SPDX-License-Identifier: AGPL-3.0-or-later
import { useTranslation } from 'react-i18next';

export type RuntimeSettingsTab = 'general' | 'device' | 'plugins' | 'privacy' | 'about';

function SkeletonLines({ count = 3, shortLast = false }: { count?: number; shortLast?: boolean }) {
  return (
    <div className="runtime-skeleton-lines" aria-hidden="true">
      {Array.from({ length: count }, (_, index) => (
        <span
          key={index}
          className={`runtime-skeleton${shortLast && index === count - 1 ? ' runtime-skeleton-short' : ''}`}
        />
      ))}
    </div>
  );
}

function SkeletonCard({
  title,
  size,
  lines = 3,
  className = '',
}: {
  title: string;
  size: string;
  lines?: number;
  className?: string;
}) {
  return (
    <section className={`card about-section runtime-frame-card runtime-frame-${size}${className ? ` ${className}` : ''}`}>
      <div className="card-title"><h2>{title}</h2></div>
      <SkeletonLines count={lines} shortLast />
    </section>
  );
}

export function AboutPageSkeleton({ onBack, showLogo = true }: { onBack: () => void; showLogo?: boolean }) {
  const { t } = useTranslation();
  return (
    <main className="about-page">
      <header>
        <div>
          <p className="eyebrow">{t('about.eyebrow')}</p>
          <h1>{t('about.title')}</h1>
        </div>
        <button className="secondary" onClick={onBack}>{t('common.back')}</button>
      </header>
      <div className="settings-scroll-area" aria-busy="true" aria-label={t('about.loading')}>
        <div className="settings-scroll-content about-loading-content runtime-frame-list">
          <section className="card about-section about-intro-card runtime-frame-card runtime-frame-about-intro">
            {showLogo && (
              <span className="about-logo-frame" aria-hidden="true">
                <img className="about-logo about-logo-light" src="/app-icon.png" alt="" />
                <img className="about-logo about-logo-dark" src="/app-icon-dark.png" alt="" />
              </span>
            )}
            <span className="runtime-skeleton runtime-skeleton-title" aria-hidden="true" />
            <SkeletonLines count={3} shortLast />
          </section>
          <SkeletonCard title={t('about.section.version')} size="about-version" lines={8} />
          <SkeletonCard title={t('about.section.donate')} size="about-donate" lines={2} className="donate-card" />
          <SkeletonCard title={t('about.section.bundledPlugins')} size="about-plugins" lines={7} />
          <SkeletonCard title={t('about.section.contact')} size="about-contact" lines={2} />
          <SkeletonCard title={t('about.section.checkUpdate')} size="about-update" lines={2} />
          <SkeletonCard title={t('about.section.license')} size="about-license" lines={2} />
          <SkeletonCard title={t('about.section.privacy')} size="about-privacy" lines={2} />
          <SkeletonCard title={t('about.section.reportIssue')} size="about-issue" lines={2} />
          <span className="visually-hidden" role="status">{t('about.loading')}</span>
        </div>
      </div>
    </main>
  );
}

const SETTINGS_CARD_SPECS: Record<RuntimeSettingsTab, Array<{ key: string; size: string }>> = {
  general: [
    { key: 'settings.language.label', size: 'settings-general-row' },
    { key: 'settings.section.theme', size: 'settings-general-row' },
    { key: 'settings.section.startup', size: 'settings-startup' },
    { key: 'settings.section.update', size: 'settings-update' },
  ],
  device: [
    { key: 'settings.section.battery', size: 'settings-device-row' },
    { key: 'settings.section.batteryHistory', size: 'settings-history' },
    { key: 'settings.section.nightLight', size: 'settings-device-row' },
    { key: 'settings.section.config', size: 'settings-config' },
  ],
  plugins: [
    { key: 'settings.localAi.title', size: 'settings-local-ai' },
    { key: 'settings.section.plugins', size: 'settings-plugins' },
  ],
  privacy: [{ key: 'settings.section.privacy', size: 'settings-privacy' }],
  about: [
    { key: 'settings.section.about', size: 'settings-about' },
    { key: 'logs.title', size: 'settings-logs' },
    { key: 'about.section.donate', size: 'settings-donate' },
  ],
};

export function SettingsPageSkeleton({ tab = 'general' }: { tab?: RuntimeSettingsTab }) {
  const { t } = useTranslation();
  const tabs: Array<{ id: RuntimeSettingsTab; key: string }> = [
    { id: 'general', key: 'settings.tab.general' },
    { id: 'device', key: 'settings.tab.device' },
    { id: 'plugins', key: 'settings.tab.plugins' },
    { id: 'privacy', key: 'settings.tab.privacy' },
    { id: 'about', key: 'settings.tab.about' },
  ];
  return (
    <main className="settings-page runtime-settings-frame" aria-busy="true">
      <header>
        <div><p className="eyebrow">Mira Mouse</p><h1>{t('settings.title')}</h1></div>
      </header>
      <nav className="sub-nav" aria-hidden="true">
        {tabs.map((item) => (
          <span
            key={item.id}
            className={`sub-nav-link ${item.id === tab ? 'active' : ''}`}
          >
            {t(item.key)}
          </span>
        ))}
      </nav>
      <div className="settings-scroll-area" aria-label={t('about.loading')}>
        <div className="settings-scroll-content runtime-frame-list">
          {SETTINGS_CARD_SPECS[tab].map((spec) => (
            <section key={spec.key} className={`card settings-section runtime-frame-card runtime-frame-${spec.size}`}>
              <div className="card-title"><h2>{t(spec.key)}</h2></div>
              <SkeletonLines count={spec.size === 'settings-local-ai' ? 7 : spec.size === 'settings-startup' || spec.size === 'settings-history' ? 6 : 2} shortLast />
            </section>
          ))}
          <span className="visually-hidden" role="status">{t('about.loading')}</span>
        </div>
      </div>
    </main>
  );
}

export function LogPageSkeleton({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation();
  return (
    <main className="log-page runtime-log-frame" aria-busy="true">
      <header>
        <div className="log-title-block"><h1>{t('logs.title')}</h1></div>
        <button className="secondary" onClick={onBack}>{t('common.back')}</button>
      </header>
      <div className="log-toolbar runtime-log-toolbar" aria-hidden="true">
        <span className="runtime-skeleton" />
        <div className="runtime-log-controls">
          <span className="runtime-skeleton runtime-skeleton-short" />
          <span className="runtime-skeleton" />
        </div>
      </div>
      <div className="log-list-wrapper runtime-log-list" aria-hidden="true">
        {Array.from({ length: 7 }, (_, index) => (
          <div className="runtime-log-row" key={index}>
            <span className="runtime-skeleton runtime-log-time" />
            <span className="runtime-skeleton runtime-log-level" />
            <span className="runtime-skeleton runtime-log-message" />
          </div>
        ))}
      </div>
      <span className="visually-hidden" role="status">{t('about.loading')}</span>
    </main>
  );
}
