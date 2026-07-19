// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ExternalLink } from './ExternalLink';
import type { AboutInfo } from './types';
import { notifyError } from './notify';
import { extractChannel } from './plugin-utils';
import {
  appUpdateState,
  checkForAppUpdate,
  installAppUpdate,
  onAppUpdateState,
  relaunchAfterUpdate,
  type AppUpdateState,
} from './updater';

export function AboutPage({ onBack, previewMode = false, focusUpdateToken = 0 }: { onBack: () => void; previewMode?: boolean; focusUpdateToken?: number }) {
  const { t } = useTranslation();
  const PREVIEW_INFO: AboutInfo = {
    name: 'Mira Mouse',
    version: '0.1.0-preview',
    identifier: 'app.mira.preview',
    platform: 'Web Preview',
    architecture: 'browser',
    rustVersion: 'N/A',
    buildDate: t('about.buildDatePreview'),
    gitCommit: 'working-tree',
    bundledPlugins: [],
    contact: {
      github: 'https://github.com/hello-yunshu',
      repository: 'https://github.com/hello-yunshu/mira-mouse',
      x: 'https://x.com/yunyunyshu',
      telegram: 'https://t.me/yunyunshu',
      developerName: '云云舒',
      copyright: '云云舒',
    },
    updaterActive: false,
  };
  const [info, setInfo] = useState<AboutInfo | null>(previewMode ? PREVIEW_INFO : null);
  const [error, setError] = useState<string>('');
  const [update, setUpdate] = useState<AppUpdateState>(appUpdateState());

  useEffect(() => {
    if (previewMode) return;
    invoke<AboutInfo>('about_info')
      .then(setInfo)
      .catch((err) => {
        const message = String(err);
        notifyError(t('notification.loadAboutFailed'), message);
        setError(message);
      });
  }, [previewMode, t]);

  useEffect(() => onAppUpdateState(setUpdate), []);

  useEffect(() => {
    if (focusUpdateToken === 0) return;
    const target = document.getElementById('about-update-section');
    target?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
    target?.focus?.({ preventScroll: true });
  }, [focusUpdateToken, info]);

  async function checkForUpdates() {
    if (!info?.updaterActive) return;
    try {
      await checkForAppUpdate();
    } catch (err) {
      notifyError(t('notification.checkUpdateFailed'), String(err));
    }
  }

  async function installUpdate() {
    try {
      await installAppUpdate();
    } catch (err) {
      notifyError(t('notification.installUpdateFailed'), String(err));
    }
  }

  if (error) {
    return (
      <main className="about-page">
        <header>
          <button className="secondary" onClick={onBack}>{t('common.back')}</button>
        </header>
        <p className="setting-hint">{t('about.loadFailed', { error })}</p>
      </main>
    );
  }

  if (!info) {
    return (
      <main className="about-page">
        <header>
          <button className="secondary" onClick={onBack}>{t('common.back')}</button>
        </header>
        <p className="setting-hint">{t('about.loading')}</p>
      </main>
    );
  }

  const contact = info.contact;
  const repositoryUrl = contact.repository ?? contact.github;
  const linkError = t('notification.openExternalFailed');

  return (
    <main className="about-page">
      <header>
        <div>
          <p className="eyebrow">{t('about.eyebrow')}</p>
          <h1>{t('about.title')}</h1>
        </div>
        <button className="secondary" onClick={onBack}>{t('common.back')}</button>
      </header>

      <section className="card about-section about-intro-card">
        <span className="about-logo-frame" aria-hidden="true">
          <img className="about-logo about-logo-light" src="/app-icon.png" alt="" />
          <img className="about-logo about-logo-dark" src="/app-icon-dark.png" alt="" />
        </span>
        <h2>{info.name}</h2>
        <p className="eyebrow">{t('about.unofficial')}</p>
        <p className="disclaimer">
          {t('about.disclaimer')}
        </p>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>{t('about.section.version')}</h2></div>
        <dl className="info-list">
          <div><dt>{t('about.label.appName')}</dt><dd>{info.name}</dd></div>
          <div><dt>{t('about.label.version')}</dt><dd>{info.version}</dd></div>
          <div><dt>{t('about.label.buildDate')}</dt><dd>{info.buildDate}</dd></div>
          <div><dt>{t('about.label.gitCommit')}</dt><dd><code>{info.gitCommit}</code></dd></div>
          <div><dt>{t('about.label.platform')}</dt><dd>{info.platform}</dd></div>
          <div><dt>{t('about.label.architecture')}</dt><dd>{info.architecture}</dd></div>
          <div><dt>{t('about.label.rustVersion')}</dt><dd>{info.rustVersion}</dd></div>
          <div><dt>{t('about.label.autoUpdate')}</dt><dd>{info.updaterActive ? t('about.autoUpdateEnabled') : t('about.autoUpdateDisabled')}</dd></div>
        </dl>
      </section>

      <section className="card about-section donate-card">
        <div className="card-title"><h2>{t('about.section.donate')}</h2></div>
        <p className="setting-hint donate-hint">{t('about.donate.hint')}</p>
        <div className="contact-links">
          <ExternalLink className="primary" href="https://hey.run/donate/" errorTitle={linkError}>{t('about.donate.button')}</ExternalLink>
        </div>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>{t('about.section.bundledPlugins')}</h2></div>
        {info.bundledPlugins.length === 0 ? (
          <p className="setting-hint">{t('about.noBundledPlugins')}</p>
        ) : (
          <div className="plugin-list">
            {info.bundledPlugins.map((plugin) => {
              const channel = extractChannel(plugin.releaseTag);
              return (
                <div key={plugin.pluginId} className="plugin-item">
                  <div>
                    <strong>{plugin.pluginId}</strong>
                    <span className="setting-hint">v{plugin.version}</span>
                  </div>
                  <div className="plugin-meta">
                    {channel && <span className="badge">{channel}</span>}
                    <span className={`badge ${plugin.signatureVerified ? 'badge-ok' : 'badge-warn'}`}>
                      {plugin.signatureVerified ? t('about.signatureVerified') : t('about.signatureUnverified')}
                    </span>
                    {plugin.bundleByDefault && <span className="badge">{t('about.defaultBundled')}</span>}
                    {plugin.source === 'installed' && <span className="badge badge-ok">{t('about.userUpdated')}</span>}
                  </div>
                  <dl className="plugin-detail">
                    <div><dt>{t('about.label.sha256')}</dt><dd><code>{plugin.sha256}</code></dd></div>
                    <div><dt>{t('about.label.publisherKey')}</dt><dd><code>{plugin.publisherKeyId}</code></dd></div>
                    <div><dt>{t('about.label.asset')}</dt><dd>{plugin.asset}</dd></div>
                  </dl>
                </div>
              );
            })}
          </div>
        )}
      </section>

      {contact.github || contact.x || contact.telegram ? (
        <section className="card about-section">
          <div className="card-title"><h2>{t('about.section.contact')}</h2></div>
          <div className="settings-action-body">
            <div className="settings-action-copy">
              {contact.developerName && <p className="setting-hint">{t('about.developer', { name: contact.developerName })}</p>}
              {contact.copyright && <p className="setting-hint">{t('about.copyright', { name: contact.copyright })}</p>}
            </div>
            <div className="contact-links align-end">
              {contact.github && <ExternalLink className="secondary" href={contact.github} errorTitle={linkError}>GitHub</ExternalLink>}
              {contact.x && <ExternalLink className="secondary" href={contact.x} errorTitle={linkError}>X</ExternalLink>}
              {contact.telegram && <ExternalLink className="secondary" href={contact.telegram} errorTitle={linkError}>Telegram</ExternalLink>}
            </div>
          </div>
        </section>
      ) : null}

      <section id="about-update-section" className="card about-section" tabIndex={-1}>
        <div className="card-title"><h2>{t('about.section.checkUpdate')}</h2></div>
        <div className="settings-action-body">
          <p className="setting-hint">
            {info.updaterActive
              ? t('about.updateEnabledHint')
              : t('about.updateDisabledHint')}
          </p>
          {info.updaterActive && (
            <div className="contact-links align-end">
              <button className="secondary" onClick={checkForUpdates} disabled={update.phase === 'checking' || update.phase === 'downloading'}>
                {update.phase === 'checking' ? t('about.updateChecking') : t('about.updateCheck')}
              </button>
              {update.phase === 'up-to-date' && <span className="save-badge">{t('about.updateUpToDate')}</span>}
              {update.phase === 'available' && (
                <button className="primary" onClick={installUpdate}>
                  {t('about.downloadInstall', { version: update.version })}
                </button>
              )}
              {update.phase === 'installed' && <button className="primary" onClick={() => void relaunchAfterUpdate()}>{t('about.relaunch')}</button>}
            </div>
          )}
        </div>
        {info.updaterActive && (
          <>
            {update.phase === 'available' && (
              <div className="update-details">
                {update.date && <span className="setting-hint">{t('about.releaseDate', { date: new Date(update.date).toLocaleDateString() })}</span>}
                {update.notes && <p>{update.notes}</p>}
              </div>
            )}
            {update.phase === 'downloading' && (
              <div className="update-progress" aria-live="polite">
                <progress value={update.downloadedBytes} max={update.totalBytes || undefined} />
                <span>{update.totalBytes
                  ? t('about.downloadedPercent', { percent: Math.min(100, Math.round((update.downloadedBytes / update.totalBytes) * 100)) })
                  : t('about.downloadedMib', { mib: (update.downloadedBytes / 1024 / 1024).toFixed(1) })}</span>
              </div>
            )}
            {update.phase === 'error' && <p className="setting-hint update-error">{update.error}</p>}
          </>
        )}
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>{t('about.section.license')}</h2></div>
        <div className="settings-action-body">
          <p className="setting-hint">
            {t('about.licenseHint')}
          </p>
          <div className="contact-links align-end">
            {repositoryUrl ? (
              <ExternalLink className="secondary" href={`${repositoryUrl}/blob/main/LICENSE`} errorTitle={linkError}>{t('about.viewLicense')}</ExternalLink>
            ) : (
              <button className="secondary" disabled>{t('about.viewLicenseDisabled')}</button>
            )}
            {repositoryUrl ? (
              <ExternalLink className="secondary" href={`${repositoryUrl}/tree/main/NOTICE`} errorTitle={linkError}>{t('about.viewThirdParty')}</ExternalLink>
            ) : (
              <button className="secondary" disabled>{t('about.viewThirdPartyDisabled')}</button>
            )}
          </div>
        </div>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>{t('about.section.privacy')}</h2></div>
        <p className="setting-hint">{t('about.privacyHint')}</p>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>{t('about.section.reportIssue')}</h2></div>
        <div className="settings-action-body">
          <p className="setting-hint">
            {t('about.reportIssueHint')}
          </p>
          <div className="contact-links align-end">
            {repositoryUrl ? (
              <ExternalLink className="secondary" href={`${repositoryUrl}/issues/new/choose`} errorTitle={linkError}>{t('about.reportIssue')}</ExternalLink>
            ) : (
              <button className="secondary" disabled>{t('about.reportIssueDisabled')}</button>
            )}
          </div>
        </div>
      </section>
    </main>
  );
}
