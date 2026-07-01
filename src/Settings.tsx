// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { AppSettings, BundledPluginInfo, AboutInfo, DiscoveredDevice, PluginInstallResult, PluginUpdateInfo, ThemeMode } from './types';
import { Tooltip } from './Tooltip';
import { notifyError, notifyInfo } from './notify';
import { extractChannel, exportDiagnostics } from './plugin-utils';
import { applyLanguage, type AppLanguage } from './i18n';
import { save, open } from '@tauri-apps/plugin-dialog';
import { ExternalLink } from './ExternalLink';
import { startAutomaticAppUpdateCheck } from './updater';

const DEFAULT_SETTINGS: AppSettings = {
  language: 'auto',
  theme: 'system',
  autostart: false,
  startHidden: false,
  trayShowBatteryTitle: true,
  trayIncludeReceiverBattery: false,
  trayShowConnection: true,
  trayIconColor: 'white',
  lowBatteryThreshold: 20,
  nightModeEnabled: false,
  nightModeStart: '22:00',
  nightModeEnd: '07:00',
  nightModeTriggerTime: true,
  nightModeTriggerTheme: false,
  nightModeThemeDark: true,
  nightModeTriggerCharging: false,
  nightModeTriggerLowBattery: false,
  nightModeTargetMouse: true,
  nightModeTargetReceiver: false,
  refreshIntervalSeconds: 5,
  telemetryDisabled: true,
  automaticUpdateChecks: true,
  automaticUpdateInstall: false,
  automaticPluginUpdateChecks: true,
};

type SettingsTab = 'general' | 'device' | 'plugins' | 'privacy' | 'about';

function SettingRow({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-label">
        <strong>{title}</strong>
        {hint && <span className="setting-hint">{hint}</span>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

function Toggle({ checked, onChange, label, disabled = false }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      className={`toggle ${checked ? 'on' : ''}`}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  );
}

// 与 App.tsx 中 isWindowsPlatform 一致：兼容 ?platform=windows 网页预览
function isWindowsPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'windows' || navigator.userAgent.includes('Windows');
}

export function SettingsPage({ onNavigateAbout, onThemeChange, previewMode = false, supportsAnyLighting = false, supportsReceiverLighting = false, focusPluginUpdateToken = 0 }: { onNavigateAbout: () => void; onThemeChange: (theme: ThemeMode) => void; previewMode?: boolean; supportsAnyLighting?: boolean; supportsReceiverLighting?: boolean; focusPluginUpdateToken?: number }) {
  const { t } = useTranslation();
  const windowsPlatform = isWindowsPlatform();
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [plugins, setPlugins] = useState<BundledPluginInfo[]>([]);
  const [pluginUpdates, setPluginUpdates] = useState<PluginUpdateInfo[]>([]);
  const [pluginUpdatesChecking, setPluginUpdatesChecking] = useState(false);
  const [pluginInstalling, setPluginInstalling] = useState<string>();
  const [diagnostics, setDiagnostics] = useState<string>('');
  const [discovered, setDiscovered] = useState<DiscoveredDevice[]>([]);
  const [saved, setSaved] = useState(false);
  const [tabState, setTabState] = useState<{ tab: SettingsTab; focusToken: number }>(() => ({
    tab: focusPluginUpdateToken > 0 ? 'plugins' : 'general',
    focusToken: focusPluginUpdateToken,
  }));
  const pendingPluginFocus = useRef(false);
  const tab = focusPluginUpdateToken > tabState.focusToken ? 'plugins' : tabState.tab;

  // 点击「插件更新可用」通知后，先切到 plugins 标签，待渲染后再滚动聚焦。
  useEffect(() => {
    if (focusPluginUpdateToken === 0) return;
    pendingPluginFocus.current = true;
  }, [focusPluginUpdateToken]);

  useEffect(() => {
    if (!pendingPluginFocus.current || tab !== 'plugins') return;
    pendingPluginFocus.current = false;
    const target = document.getElementById('settings-plugin-update-section');
    target?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
    target?.focus?.({ preventScroll: true });
  }, [tab, focusPluginUpdateToken]);

  const TABS: { id: SettingsTab; label: string }[] = [
    { id: 'general', label: t('settings.tab.general') },
    { id: 'device', label: t('settings.tab.device') },
    { id: 'plugins', label: t('settings.tab.plugins') },
    { id: 'privacy', label: t('settings.tab.privacy') },
    { id: 'about', label: t('settings.tab.about') },
  ];

  useEffect(() => {
    if (previewMode) return;
    invoke<AppSettings>('settings_get')
      .then((loaded) => {
        // 与默认值合并，避免后端字段缺失导致受控输入变为 undefined
        const merged: AppSettings = { ...DEFAULT_SETTINGS, ...loaded };
        setSettings(merged);
        onThemeChange(merged.theme as ThemeMode);
      })
      .catch(() => setSettings(DEFAULT_SETTINGS));
    invoke<boolean>('autostart_state')
      .then(setAutostartEnabled)
      .catch(() => setAutostartEnabled(false));
    invoke<AboutInfo>('about_info')
      .then((info) => setPlugins(info.bundledPlugins ?? []))
      .catch(() => setPlugins([]));
  }, [onThemeChange, previewMode]);

  function update(patch: Partial<AppSettings>) {
    const next = { ...settings, ...patch };
    const automaticUpdateChanged = patch.automaticUpdateChecks !== undefined || patch.automaticUpdateInstall !== undefined;
    setSettings(next);
    if (patch.theme && onThemeChange) onThemeChange(patch.theme as ThemeMode);
    if (previewMode) {
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
      return;
    }
    invoke<AppSettings>('settings_set', { settings: next })
      .then((savedSettings) => {
        setSettings(savedSettings);
        if (automaticUpdateChanged) {
          syncAutomaticAppUpdateChecks(savedSettings);
        }
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      })
      .catch((error) => notifyError(t('notification.saveFailed'), String(error)));
  }

  function syncAutomaticAppUpdateChecks(nextSettings: AppSettings) {
    if (!nextSettings.automaticUpdateChecks) {
      void startAutomaticAppUpdateCheck(false);
      return;
    }
    void invoke<AboutInfo>('about_info')
      .then((info) => {
        if (info.updaterActive) {
          return startAutomaticAppUpdateCheck(true, nextSettings.automaticUpdateInstall);
        }
      })
      .catch(() => { /* Pre-release and offline builds skip automatic application checks. */ });
  }

  function toggleAutostart(enabled: boolean) {
    if (previewMode) {
      setAutostartEnabled(enabled);
      update({ autostart: enabled });
      return;
    }
    invoke('set_autostart', { enabled })
      .then(() => {
        setAutostartEnabled(enabled);
        update({ autostart: enabled });
      })
      .catch((error) => {
        // 保持开关状态不变（未启用就是 false），并提示用户失败原因
        setAutostartEnabled(!enabled);
        notifyError(t('notification.autostartFailed'), String(error));
      });
  }

  async function handleExportDiagnostics() {
    const result = await exportDiagnostics();
    if (result !== undefined) setDiagnostics(result);
  }

  // #11 配置导出：通过系统文件选择器指定保存路径。
  async function handleExportConfig() {
    if (previewMode) return;
    try {
      const path = await save({
        defaultPath: 'device-config.json',
        filters: [{ name: t('settings.config.filterName'), extensions: ['json'] }],
      });
      if (!path) return;
      await invoke('device_config_export', { path });
      notifyInfo(t('notification.exportSuccess'), t('notification.exportSuccessBody', { path }));
    } catch (error) {
      notifyError(t('notification.exportFailed'), String(error));
    }
  }

  // #11 配置导入：通过系统文件选择器选择配置文件。
  async function handleImportConfig() {
    if (previewMode) return;
    try {
      const selected = await open({
        filters: [{ name: t('settings.config.filterName'), extensions: ['json'] }],
        multiple: false,
      });
      if (!selected || typeof selected !== 'string') return;
      await invoke('device_config_import', { path: selected });
      notifyInfo(t('notification.importSuccess'), t('notification.importSuccessBody'));
    } catch (error) {
      notifyError(t('notification.importFailed'), String(error));
    }
  }

  function scanDevices() {
    invoke<DiscoveredDevice[]>('discover_devices')
      .then(setDiscovered)
      .catch((err) => notifyError(t('notification.scanFailed'), String(err)));
  }

  async function checkPluginUpdates() {
    if (previewMode) return;
    setPluginUpdatesChecking(true);
    try {
      setPluginUpdates(await invoke<PluginUpdateInfo[]>('plugin_updates_check'));
    } catch (error) {
      notifyError(t('notification.checkPluginUpdateFailed'), String(error));
    } finally {
      setPluginUpdatesChecking(false);
    }
  }

  async function installPluginUpdate(pluginId: string) {
    setPluginInstalling(pluginId);
    try {
      const result = await invoke<PluginInstallResult>('plugin_update_install', { pluginId });
      setPlugins((current) => current.map((plugin) => plugin.pluginId === result.pluginId
        ? { ...plugin, version: result.version, source: 'installed', signatureVerified: true }
        : plugin));
      await checkPluginUpdates();
    } catch (error) {
      notifyError(t('notification.installPluginUpdateFailed'), String(error));
    } finally {
      setPluginInstalling(undefined);
    }
  }

  return (
    <main className="settings-page">
      <header>
        <div>
          <p className="eyebrow">Mira Mouse</p>
          <h1>{t('settings.title')}</h1>
        </div>
        {saved && <span className="save-badge">{t('common.saved')}</span>}
      </header>

      <nav className="sub-nav" aria-label={t('settings.category')}>
        {TABS.map((tabItem) => (
          <button
            key={tabItem.id}
            className={`sub-nav-link ${tab === tabItem.id ? 'active' : ''}`}
            onClick={() => setTabState({ tab: tabItem.id, focusToken: focusPluginUpdateToken })}
            aria-pressed={tab === tabItem.id}
          >
            {tabItem.label}
          </button>
        ))}
      </nav>

      {tab === 'general' && (
        <>
          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.language.label')}</h2></div>
            <SettingRow title={t('settings.language.label')} hint={t('settings.language.hint')}>
              <select
                value={settings.language}
                onChange={(e) => {
                  const lang = e.target.value as AppLanguage;
                  applyLanguage(lang);
                  update({ language: lang });
                }}
                aria-label={t('settings.language.label')}
              >
                <option value="auto">{t('settings.language.auto')}</option>
                <option value="zh-CN">{t('settings.language.zhCN')}</option>
                <option value="en">{t('settings.language.en')}</option>
              </select>
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.theme')}</h2></div>
            <SettingRow title={t('settings.theme.label')} hint={t('settings.theme.hint')}>
              <select value={settings.theme} onChange={(e) => update({ theme: e.target.value as ThemeMode })} aria-label={t('settings.theme.label')}>
                <option value="system">{t('settings.theme.system')}</option>
                <option value="light">{t('settings.theme.light')}</option>
                <option value="dark">{t('settings.theme.dark')}</option>
              </select>
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.startup')}</h2></div>
            <SettingRow title={t('settings.autostart.label')} hint={t('settings.autostart.hint')}>
              <Toggle checked={autostartEnabled} onChange={toggleAutostart} label={t('settings.autostart.label')} />
            </SettingRow>
            <SettingRow title={t('settings.startHidden.label')} hint={t('settings.startHidden.hint')}>
              <Toggle checked={settings.startHidden} onChange={(v) => update({ startHidden: v })} label={t('settings.startHidden.label')} />
            </SettingRow>
            <SettingRow title={t('settings.trayBattery.label')} hint={t('settings.trayBattery.hint')}>
              {windowsPlatform ? (
                <Tooltip label={t('settings.trayBattery.disabledHint')}>
                  <Toggle checked={settings.trayShowBatteryTitle} onChange={(v) => update({ trayShowBatteryTitle: v })} label={t('settings.trayBattery.label')} disabled />
                </Tooltip>
              ) : (
                <Toggle checked={settings.trayShowBatteryTitle} onChange={(v) => update({ trayShowBatteryTitle: v })} label={t('settings.trayBattery.label')} />
              )}
            </SettingRow>
            <SettingRow title={t('settings.trayIconColor.label')} hint={t('settings.trayIconColor.hint')}>
              <select value={settings.trayIconColor} onChange={(e) => update({ trayIconColor: e.target.value })} aria-label={t('settings.trayIconColor.label')}>
                <option value="white">{t('settings.trayIconColor.white')}</option>
                <option value="black">{t('settings.trayIconColor.black')}</option>
                <option value="auto">{t('settings.trayIconColor.auto')}</option>
              </select>
            </SettingRow>
            <SettingRow title={t('settings.receiverBattery.label')} hint={t('settings.receiverBattery.hint')}>
              {windowsPlatform ? (
                <Tooltip label={t('settings.trayBattery.disabledHint')}>
                  <Toggle checked={settings.trayIncludeReceiverBattery} onChange={(v) => update({ trayIncludeReceiverBattery: v })} label={t('settings.receiverBattery.label')} disabled />
                </Tooltip>
              ) : (
                <Toggle checked={settings.trayIncludeReceiverBattery} onChange={(v) => update({ trayIncludeReceiverBattery: v })} label={t('settings.receiverBattery.label')} disabled={!settings.trayShowBatteryTitle} />
              )}
            </SettingRow>
            <SettingRow title={t('settings.trayConnection.label')} hint={t('settings.trayConnection.hint')}>
              <Toggle checked={settings.trayShowConnection} onChange={(v) => update({ trayShowConnection: v })} label={t('settings.trayConnection.label')} />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.polling')}</h2></div>
            <SettingRow title={t('settings.refreshInterval.label', { seconds: settings.refreshIntervalSeconds })} hint={t('settings.refreshInterval.hint')}>
              <input
                type="range"
                min={1}
                max={60}
                value={settings.refreshIntervalSeconds}
                onChange={(e) => update({ refreshIntervalSeconds: Number(e.target.value) })}
                aria-label={t('settings.refreshInterval.label', { seconds: settings.refreshIntervalSeconds })}
              />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.update')}</h2></div>
            <SettingRow title={t('settings.updateCheck.label')} hint={t('settings.updateCheck.hint')}>
              <Toggle checked={settings.automaticUpdateChecks} onChange={(v) => update({ automaticUpdateChecks: v })} label={t('settings.updateCheck.label')} />
            </SettingRow>
            <SettingRow title={t('settings.updateInstall.label')} hint={t('settings.updateInstall.hint')}>
              <Toggle
                checked={settings.automaticUpdateInstall}
                onChange={(v) => update({ automaticUpdateInstall: v })}
                label={t('settings.updateInstall.label')}
                disabled={!settings.automaticUpdateChecks}
              />
            </SettingRow>
          </section>
        </>
      )}

      {tab === 'device' && (
        <>
          <section className="card settings-section">
            <div className="card-title">
              <h2>{t('settings.section.battery')}</h2>
              <Tooltip label={t('settings.lowBattery.tooltip')}><button className="icon-button" aria-label={t('settings.section.battery')}>?</button></Tooltip>
            </div>
            <SettingRow title={t('settings.lowBattery.label', { value: settings.lowBatteryThreshold })} hint={t('settings.lowBattery.hint')}>
              <input
                type="range"
                min={5}
                max={50}
                value={settings.lowBatteryThreshold}
                onChange={(e) => update({ lowBatteryThreshold: Number(e.target.value) })}
                aria-label={t('settings.lowBattery.label', { value: settings.lowBatteryThreshold })}
              />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title">
              <h2>{t('settings.section.nightLight')}</h2>
              <Tooltip label={t('settings.nightMode.tooltip')}><button className="icon-button" aria-label={t('settings.section.nightLight')}>?</button></Tooltip>
            </div>
            <SettingRow title={t('settings.nightMode.label')} hint={t('settings.nightMode.hint')}>
              <Toggle checked={settings.nightModeEnabled} onChange={(v) => update({ nightModeEnabled: v })} label={t('settings.nightMode.label')} disabled={!supportsAnyLighting} />
            </SettingRow>
            {settings.nightModeEnabled && (
              <>
                <p className="setting-hint" style={{ paddingTop: 4 }}>{t('settings.nightMode.triggerSection')}</p>
                <SettingRow title={t('settings.nightMode.triggerTime')} hint={t('settings.nightMode.triggerTimeHint')}>
                  <Toggle
                    checked={settings.nightModeTriggerTime}
                    onChange={(v) => update({ nightModeTriggerTime: v, ...(v ? { nightModeTriggerTheme: false } : {}) })}
                    label={t('settings.nightMode.triggerTime')}
                  />
                </SettingRow>
                {settings.nightModeTriggerTime && (
                  <>
                    <SettingRow title={t('settings.nightMode.startLabel')} hint={t('settings.nightMode.startHint')}>
                      <input type="time" value={settings.nightModeStart} onChange={(e) => update({ nightModeStart: e.target.value })} aria-label={t('settings.nightMode.startLabel')} />
                    </SettingRow>
                    <SettingRow title={t('settings.nightMode.endLabel')} hint={t('settings.nightMode.endHint')}>
                      <input type="time" value={settings.nightModeEnd} onChange={(e) => update({ nightModeEnd: e.target.value })} aria-label={t('settings.nightMode.endLabel')} />
                    </SettingRow>
                  </>
                )}
                <SettingRow title={t('settings.nightMode.triggerTheme')} hint={t('settings.nightMode.triggerThemeHint')}>
                  <Toggle
                    checked={settings.nightModeTriggerTheme}
                    onChange={(v) => update({ nightModeTriggerTheme: v, ...(v ? { nightModeTriggerTime: false } : {}) })}
                    label={t('settings.nightMode.triggerTheme')}
                  />
                </SettingRow>
                {settings.nightModeTriggerTheme && (
                  <SettingRow title={t('settings.nightMode.triggerTheme')} hint={t('settings.nightMode.triggerThemeHint')}>
                    <select
                      value={settings.nightModeThemeDark ? 'dark' : 'light'}
                      onChange={(e) => update({ nightModeThemeDark: e.target.value === 'dark' })}
                      aria-label={t('settings.nightMode.triggerTheme')}
                    >
                      <option value="dark">{t('settings.nightMode.themeDark')}</option>
                      <option value="light">{t('settings.nightMode.themeLight')}</option>
                    </select>
                  </SettingRow>
                )}
                <SettingRow title={t('settings.nightMode.triggerCharging')} hint={t('settings.nightMode.triggerChargingHint')}>
                  <Toggle checked={settings.nightModeTriggerCharging} onChange={(v) => update({ nightModeTriggerCharging: v })} label={t('settings.nightMode.triggerCharging')} />
                </SettingRow>
                <SettingRow title={t('settings.nightMode.triggerLowBattery')} hint={t('settings.nightMode.triggerLowBatteryHint', { value: settings.lowBatteryThreshold })}>
                  <Toggle checked={settings.nightModeTriggerLowBattery} onChange={(v) => update({ nightModeTriggerLowBattery: v })} label={t('settings.nightMode.triggerLowBattery')} />
                </SettingRow>
                <p className="setting-hint" style={{ paddingTop: 4 }}>{t('settings.nightMode.targetSection')}</p>
                <SettingRow title={t('settings.nightMode.targetMouse')} hint={t('settings.nightMode.targetMouseHint')}>
                  <Toggle checked={settings.nightModeTargetMouse} onChange={(v) => update({ nightModeTargetMouse: v })} label={t('settings.nightMode.targetMouse')} />
                </SettingRow>
                <SettingRow title={t('settings.nightMode.targetReceiver')} hint={t('settings.nightMode.targetReceiverHint')}>
                  <Toggle
                    checked={settings.nightModeTargetReceiver}
                    onChange={(v) => update({ nightModeTargetReceiver: v })}
                    label={t('settings.nightMode.targetReceiver')}
                    disabled={!supportsReceiverLighting}
                  />
                </SettingRow>
              </>
            )}
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.config')}</h2></div>
            <p className="setting-hint">
              {t('settings.config.hint')}
            </p>
            <div className="contact-links">
              <button className="secondary" onClick={() => void handleExportConfig()} disabled={previewMode}>{t('settings.config.export')}</button>
              <button className="secondary" onClick={() => void handleImportConfig()} disabled={previewMode}>{t('settings.config.import')}</button>
            </div>
          </section>
        </>
      )}

      {tab === 'plugins' && (
        <section id="settings-plugin-update-section" className="card settings-section" tabIndex={-1}>
          <div className="card-title"><h2>{t('settings.section.plugins')}</h2></div>
          <SettingRow title={t('settings.pluginUpdateCheck.label')} hint={t('settings.pluginUpdateCheck.hint')}>
            <Toggle checked={settings.automaticPluginUpdateChecks} onChange={(v) => update({ automaticPluginUpdateChecks: v })} label={t('settings.pluginUpdateCheck.label')} />
          </SettingRow>
          <div className="contact-links plugin-update-actions">
            <button className="secondary" onClick={() => void checkPluginUpdates()} disabled={previewMode || pluginUpdatesChecking || Boolean(pluginInstalling)}>
              {pluginUpdatesChecking ? t('settings.pluginUpdate.checking') : t('settings.pluginUpdate.check')}
            </button>
            {pluginUpdates.length > 0 && pluginUpdates.every((item) => !item.updateAvailable) && <span className="save-badge">{t('settings.pluginUpdate.allLatest')}</span>}
          </div>
          {plugins.length === 0 ? (
            <p className="setting-hint">{t('settings.pluginUpdate.noPlugins')}</p>
          ) : (
            <div className="plugin-list">
              {plugins.map((plugin) => {
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
                        {plugin.signatureVerified ? t('settings.pluginUpdate.signatureVerified') : t('settings.pluginUpdate.signatureUnverified')}
                      </span>
                      {plugin.bundleByDefault && <span className="badge">{t('settings.pluginUpdate.defaultBundled')}</span>}
                    </div>
                    {pluginUpdates.find((item) => item.pluginId === plugin.pluginId)?.updateAvailable && (
                      <div className="plugin-update-row">
                        <span className="setting-hint">{t('settings.pluginUpdate.updatable', { version: pluginUpdates.find((item) => item.pluginId === plugin.pluginId)?.availableVersion })}</span>
                        <button
                          className="primary"
                          disabled={Boolean(pluginInstalling)}
                          onClick={() => void installPluginUpdate(plugin.pluginId)}
                        >
                          {pluginInstalling === plugin.pluginId ? t('settings.pluginUpdate.updating') : t('settings.pluginUpdate.update')}
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </section>
      )}

      {tab === 'privacy' && (
        <section className="card settings-section">
          <div className="card-title"><h2>{t('settings.section.privacy')}</h2></div>
          <SettingRow title={t('settings.privacy.telemetryLabel')} hint={t('settings.privacy.telemetryHint')}>
            <Toggle checked={true} onChange={() => {}} label={t('settings.privacy.telemetryLabel')} disabled />
          </SettingRow>
          <SettingRow title={t('settings.privacy.scanLabel')} hint={t('settings.privacy.scanHint')}>
            <button className="secondary" onClick={scanDevices} disabled={previewMode}>{t('settings.privacy.scanButton')}</button>
          </SettingRow>
          <SettingRow title={t('settings.privacy.diagnosticsLabel')} hint={t('settings.privacy.diagnosticsHint')}>
            <button className="secondary" onClick={handleExportDiagnostics} disabled={previewMode}>{t('settings.privacy.diagnosticsButton')}</button>
          </SettingRow>
          {discovered.length > 0 && (
            <div className="plugin-list">
              {discovered.map((d) => (
                <div key={d.path} className="plugin-item">
                  <div>
                    <strong>{d.pluginId} · {d.family}</strong>
                    <span className="setting-hint">VID {d.vendorId.toString(16).toUpperCase().padStart(4, '0')} · PID {d.productId.toString(16).toUpperCase().padStart(4, '0')} · usage {d.usagePage}/{d.usage}</span>
                    {d.lastError && (
                      <span className="setting-hint">Last read: {d.lastErrorKind ?? 'error'} - {d.lastError}</span>
                    )}
                  </div>
                  <div className="plugin-meta">
                    <span className="badge">{d.connection}</span>
                    <span className="badge">{d.evidence}</span>
                  </div>
                </div>
              ))}
            </div>
          )}
          {diagnostics && (
            <pre className="diagnostics-output">{diagnostics}</pre>
          )}
        </section>
      )}

      {tab === 'about' && (
        <>
          <section className="card settings-section">
            <div className="card-title"><h2>{t('settings.section.about')}</h2></div>
            <SettingRow title={t('settings.about.label')} hint={t('settings.about.hint')}>
              <button className="secondary" onClick={onNavigateAbout}>{t('settings.about.button')}</button>
            </SettingRow>
          </section>

          <section className="card settings-section donate-card">
            <div className="card-title"><h2>{t('about.section.donate')}</h2></div>
            <p className="setting-hint donate-hint">{t('about.donate.hint')}</p>
            <div className="contact-links">
              <ExternalLink className="primary" href="https://hey.run/donate/" errorTitle={t('notification.openExternalFailed')}>{t('about.donate.button')}</ExternalLink>
            </div>
          </section>
        </>
      )}
    </main>
  );
}
