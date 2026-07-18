// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useEffectEvent, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ChartBar } from '@phosphor-icons/react';
import type { AppSettings, BundledPluginInfo, AboutInfo, DiscoveredDevice, LocalAiInstallResult, LocalAiStatus, LocalAiUpdateInfo, PluginCapability, ThemeMode } from './types';
import { Tooltip } from './Tooltip';
import { notifyError, notifyInfo } from './notify';
import { extractChannel, exportDiagnostics } from './plugin-utils';
import { resolveLightingMutations, resolveLightingRoles } from './pluginAdapter';
import { applyLanguage, type AppLanguage } from './i18n';
import { save, open } from '@tauri-apps/plugin-dialog';
import { ExternalLink } from './ExternalLink';
import { startAutomaticAppUpdateCheck } from './updater';
import { checkForPluginUpdates, installPluginUpdate, onPluginUpdateState, pluginUpdateState, startAutomaticPluginUpdateCheck, type PluginUpdateState } from './plugin-updater';
import { DEFAULT_LOCAL_AI_FEATURES, LOCAL_AI_FEATURE, localAiFeatureEnabled, setLocalAiFeature } from './localAi';
import { LogPage } from './logs/LogPage';

const DEFAULT_SETTINGS: AppSettings = {
  language: 'auto',
  theme: 'system',
  autostart: false,
  startHidden: false,
  trayShowBatteryTitle: true,
  trayIncludeReceiverBattery: false,
  trayShowConnection: true,
  trayIconColor: 'auto',
  trayRenderMode: 'auto',
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
  telemetryDisabled: true,
  automaticUpdateChecks: true,
  automaticUpdateInstall: false,
  automaticPluginUpdateChecks: true,
  localAiAnalysisEnabled: false,
  localAiFeatures: { ...DEFAULT_LOCAL_AI_FEATURES },
  batteryHistoryEnabled: true,
  batteryHistoryRetentionDays: 30,
  unusualDrainAlerts: false,
};

function mergeSettingsSnapshot(
  loaded: Partial<AppSettings>,
  patch: Partial<AppSettings> = {},
): AppSettings {
  return {
    ...DEFAULT_SETTINGS,
    ...loaded,
    ...patch,
    localAiFeatures: {
      ...DEFAULT_LOCAL_AI_FEATURES,
      ...loaded.localAiFeatures,
      ...patch.localAiFeatures,
    },
  };
}

const EMPTY_LOCAL_AI_STATUS: LocalAiStatus = {
  ready: false,
  rollbackAvailable: false,
};

type SettingsTab = 'general' | 'device' | 'plugins' | 'privacy' | 'about';

type PendingSettingsSave = {
  settings: AppSettings;
  sequence: number;
  automaticUpdateChanged: boolean;
  automaticPluginUpdateChanged: boolean;
};

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

function Toggle({ checked, onChange, label, disabled = false, showOnWhenDisabled = false }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean; showOnWhenDisabled?: boolean }) {
  // 被禁用且无法设置的开关统一显示为关闭，避免「打开但不可操作」的误导；
  // showOnWhenDisabled 用于 telemetry 等需要保持显示状态的特例。
  const effectiveChecked = disabled && !showOnWhenDisabled ? false : checked;
  return (
    <button
      className={`toggle ${effectiveChecked ? 'on' : ''}`}
      role="switch"
      aria-checked={effectiveChecked}
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

function isMacPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'macos'
    || (previewPlatform === null && /Macintosh|Mac OS X/.test(navigator.userAgent));
}

export function SettingsPage({ onNavigateAbout, onOpenBatteryUsage = () => {}, onBatteryUsageSettingsChange, onThemeChange, previewMode = false, pluginCapabilities = [], writableMutations = [], focusPluginUpdateToken = 0 }: { onNavigateAbout: () => void; onOpenBatteryUsage?: () => void; onBatteryUsageSettingsChange?: (settings: { batteryHistoryEnabled: boolean; aiAnalysisEnabled: boolean }) => void; onThemeChange: (theme: ThemeMode) => void; previewMode?: boolean; pluginCapabilities?: PluginCapability[]; writableMutations?: string[]; focusPluginUpdateToken?: number }) {
  const { t } = useTranslation();
  const windowsPlatform = isWindowsPlatform();
  const macPlatform = isMacPlatform();
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  // Settings writes are full-object replacements. Keep one authoritative
  // optimistic snapshot and coalesce serialized saves so rapid UI changes
  // cannot race, overwrite a newer response, contend for settings.json.tmp,
  // or build a long write backlog while dragging a range control.
  const settingsRef = useRef<AppSettings>(DEFAULT_SETTINGS);
  const settingsHydrated = useRef(previewMode);
  const pendingHydrationPatch = useRef<Partial<AppSettings>>({});
  const settingsSaveSequence = useRef(0);
  const pendingSettingsSave = useRef<PendingSettingsSave | undefined>(undefined);
  const settingsSaveInFlight = useRef(false);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const autostartTouched = useRef(false);
  const [plugins, setPlugins] = useState<BundledPluginInfo[]>([]);
  const [pluginUpdate, setPluginUpdate] = useState<PluginUpdateState>(pluginUpdateState());
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus>(EMPTY_LOCAL_AI_STATUS);
  const [localAiUpdates, setLocalAiUpdates] = useState<LocalAiUpdateInfo[]>([]);
  const [localAiChecking, setLocalAiChecking] = useState(false);
  const [localAiInstalling, setLocalAiInstalling] = useState<'bundle' | null>(null);
  const [diagnostics, setDiagnostics] = useState<string>('');
  const [discovered, setDiscovered] = useState<DiscoveredDevice[]>([]);
  const [saved, setSaved] = useState(false);
  const [confirmingClearBattery, setConfirmingClearBattery] = useState(false);
  const [subview, setSubview] = useState<'main' | 'logs'>('main');
  const [tabState, setTabState] = useState<{ tab: SettingsTab; focusToken: number }>(() => ({
    tab: focusPluginUpdateToken > 0 ? 'plugins' : 'general',
    focusToken: focusPluginUpdateToken,
  }));
  const pendingPluginFocus = useRef(false);
  const tab = focusPluginUpdateToken > tabState.focusToken ? 'plugins' : tabState.tab;
  const pluginUpdates = pluginUpdate.updates;
  const pluginUpdatesChecking = pluginUpdate.phase === 'checking';
  const pluginInstalling = pluginUpdate.installingPluginId;
  const batteryAiAnalysisEnabled = localAiFeatureEnabled(settings, LOCAL_AI_FEATURE.batteryUsage);

  // 通过 resolveLightingMutations 从插件 capability 与可写 mutation 计算灯光支持情况，
  // 替代已移除的 supportsAnyLighting/supportsLightingMutation 旧导出。
  const availablePluginCapabilities = pluginCapabilities.filter((capability) => capability.available !== false);
  const lightingMutations = resolveLightingMutations(availablePluginCapabilities, writableMutations);
  const supportsAnyLighting = lightingMutations.length > 0;
  // 灯光角色可用性由插件 zone 声明驱动，UI 不再硬编码 mutation 名。
  const { mouse: supportsMouseLighting, receiver: supportsReceiverLighting } = resolveLightingRoles(availablePluginCapabilities, writableMutations);

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

  async function flushSettingsSaveQueue() {
    if (settingsSaveInFlight.current) return;
    const pending = pendingSettingsSave.current;
    if (!pending) return;
    pendingSettingsSave.current = undefined;
    settingsSaveInFlight.current = true;
    try {
      const savedSettings = await invoke<AppSettings>('settings_set', { settings: pending.settings });
      // A newer optimistic edit may already be queued. Only the newest save
      // is allowed to replace the visible state with its normalized response.
      if (pending.sequence === settingsSaveSequence.current) {
        settingsRef.current = savedSettings;
        setSettings(savedSettings);
      }
      if (pending.automaticUpdateChanged) {
        syncAutomaticAppUpdateChecks(savedSettings);
      }
      if (pending.automaticPluginUpdateChanged) {
        void startAutomaticPluginUpdateCheck(savedSettings.automaticPluginUpdateChecks);
      }
      if (pending.sequence === settingsSaveSequence.current) {
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      }
    } catch (error) {
      notifyError(t('notification.saveFailed'), String(error));
      // A newer full-object save already includes this optimistic change. If
      // this attempt fails, carry its runtime side-effect flags forward so the
      // successful replacement save still resynchronizes update schedulers.
      const queuedAfterFailure = pendingSettingsSave.current as PendingSettingsSave | undefined;
      if (queuedAfterFailure) {
        queuedAfterFailure.automaticUpdateChanged ||= pending.automaticUpdateChanged;
        queuedAfterFailure.automaticPluginUpdateChanged ||= pending.automaticPluginUpdateChanged;
      }
      // If this was the latest edit, restore the backend's canonical state so
      // an optimistic toggle does not remain visibly enabled after a failure.
      if (pending.sequence === settingsSaveSequence.current) {
        try {
          const persisted = await invoke<AppSettings>('settings_get');
          const recovered = mergeSettingsSnapshot(persisted);
          settingsRef.current = recovered;
          setSettings(recovered);
          applyLanguage(recovered.language);
          onThemeChange(recovered.theme as ThemeMode);
        } catch {
          // Keep the optimistic state if the canonical settings cannot be read.
        }
      }
    } finally {
      settingsSaveInFlight.current = false;
      if (pendingSettingsSave.current) void flushSettingsSaveQueue();
    }
  }

  function queueSettingsSave(next: AppSettings, patch: Partial<AppSettings>) {
    const automaticUpdateChanged = patch.automaticUpdateChecks !== undefined || patch.automaticUpdateInstall !== undefined;
    const automaticPluginUpdateChanged = patch.automaticPluginUpdateChecks !== undefined;
    const saveSequence = ++settingsSaveSequence.current;
    const queued = pendingSettingsSave.current;
    pendingSettingsSave.current = {
      settings: next,
      sequence: saveSequence,
      automaticUpdateChanged: automaticUpdateChanged || queued?.automaticUpdateChanged === true,
      automaticPluginUpdateChanged: automaticPluginUpdateChanged || queued?.automaticPluginUpdateChanged === true,
    };
    void flushSettingsSaveQueue();
  }

  const queueSettingsSaveFromEffect = useEffectEvent(queueSettingsSave);

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
        // 与默认值合并，避免后端字段缺失导致受控输入变为 undefined。
        // 首次 IPC 返回前发生的极早用户操作作为补丁叠加，不能用默认
        // 整对象覆盖已经持久化的其他偏好。
        const hydrationPatch = pendingHydrationPatch.current;
        const merged = mergeSettingsSnapshot(loaded, hydrationPatch);
        pendingHydrationPatch.current = {};
        settingsHydrated.current = true;
        settingsRef.current = merged;
        setSettings(merged);
        onThemeChange(merged.theme as ThemeMode);
        if (Object.keys(hydrationPatch).length > 0) {
          queueSettingsSaveFromEffect(merged, hydrationPatch);
        }
      })
      .catch(() => {
        const hydrationPatch = pendingHydrationPatch.current;
        const fallback = mergeSettingsSnapshot(DEFAULT_SETTINGS, hydrationPatch);
        pendingHydrationPatch.current = {};
        settingsHydrated.current = true;
        settingsRef.current = fallback;
        setSettings(fallback);
        onThemeChange(fallback.theme as ThemeMode);
        if (Object.keys(hydrationPatch).length > 0) {
          queueSettingsSaveFromEffect(fallback, hydrationPatch);
        }
      });
    invoke<boolean>('autostart_state')
      .then((enabled) => {
        if (!autostartTouched.current) setAutostartEnabled(enabled);
      })
      .catch(() => {
        if (!autostartTouched.current) setAutostartEnabled(false);
      });
    invoke<AboutInfo>('about_info')
      .then((info) => setPlugins(info.bundledPlugins ?? []))
      .catch(() => setPlugins([]));
    invoke<LocalAiStatus>('local_ai_status')
      .then((status) => status && setLocalAiStatus(status))
      .catch(() => setLocalAiStatus(EMPTY_LOCAL_AI_STATUS));
  }, [onThemeChange, previewMode]);

  useEffect(() => onPluginUpdateState(setPluginUpdate), []);

  useEffect(() => {
    onBatteryUsageSettingsChange?.({
      batteryHistoryEnabled: settings.batteryHistoryEnabled,
      aiAnalysisEnabled: batteryAiAnalysisEnabled,
    });
  }, [batteryAiAnalysisEnabled, onBatteryUsageSettingsChange, settings.batteryHistoryEnabled]);

  function update(patch: Partial<AppSettings>) {
    const next = { ...settingsRef.current, ...patch };
    settingsRef.current = next;
    setSettings(next);
    if (patch.theme && onThemeChange) onThemeChange(patch.theme as ThemeMode);
    if (previewMode) {
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
      return;
    }
    if (!settingsHydrated.current) {
      pendingHydrationPatch.current = { ...pendingHydrationPatch.current, ...patch };
      return;
    }
    queueSettingsSave(next, patch);
  }

  function toggleAutostart(enabled: boolean) {
    autostartTouched.current = true;
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

  // 电量历史清除
  async function handleClearBatteryHistory() {
    if (previewMode) {
      notifyInfo(t('batteryUsage.clearDone'), '');
      setConfirmingClearBattery(false);
      return;
    }
    try {
      await invoke('battery_history_clear', { deviceKey: null });
      notifyInfo(t('batteryUsage.clearDone'), '');
      setConfirmingClearBattery(false);
    } catch (err) {
      notifyError(t('batteryUsage.clearHistory'), String(err));
      setConfirmingClearBattery(false);
    }
  }

  // 电量历史导出
  async function handleExportBatteryHistory(format: 'json' | 'csv') {
    if (previewMode) return;
    try {
      const ext = format === 'csv' ? 'csv' : 'json';
      const path = await save({
        defaultPath: `battery-history.${ext}`,
        filters: [{ name: format.toUpperCase(), extensions: [ext] }],
      });
      if (!path) return;
      await invoke('battery_history_export', { format, path });
      notifyInfo(t('batteryUsage.exportDone'), '');
    } catch (err) {
      notifyError(t('batteryUsage.exportFailed'), String(err));
    }
  }

  async function checkPluginUpdates() {
    if (previewMode) return;
    try {
      await checkForPluginUpdates();
    } catch (error) {
      notifyError(t('notification.checkPluginUpdateFailed'), String(error));
    }
  }

  async function handlePluginUpdateInstall(pluginId: string) {
    try {
      const result = await installPluginUpdate(pluginId);
      setPlugins((current) => current.map((plugin) => plugin.pluginId === result.pluginId
        ? { ...plugin, version: result.version, source: 'installed', signatureVerified: true }
        : plugin));
    } catch (error) {
      notifyError(t('notification.installPluginUpdateFailed'), String(error));
    }
  }

  async function checkLocalAiUpdates() {
    if (previewMode) return;
    setLocalAiChecking(true);
    try {
      const updates = await invoke<LocalAiUpdateInfo[]>('local_ai_updates_check');
      setLocalAiUpdates(updates ?? []);
    } catch (error) {
      notifyError(t('notification.checkLocalAiUpdateFailed'), String(error));
    } finally {
      setLocalAiChecking(false);
    }
  }

  async function installLocalAiUpdate() {
    if (previewMode) return;
    setLocalAiInstalling('bundle');
    try {
      await invoke<LocalAiInstallResult>('local_ai_update_install', { component: 'bundle' });
      const nextStatus = await invoke<LocalAiStatus>('local_ai_status');
      if (nextStatus) setLocalAiStatus(nextStatus);
      setLocalAiUpdates((updates) => updates.map((updateInfo) => updateInfo.component === 'bundle'
        ? { ...updateInfo, currentVersion: updateInfo.availableVersion, updateAvailable: false }
        : updateInfo));
    } catch (error) {
      notifyError(t('notification.installLocalAiUpdateFailed'), String(error));
    } finally {
      setLocalAiInstalling(null);
    }
  }

  async function rollbackLocalAi() {
    if (previewMode) return;
    setLocalAiInstalling('bundle');
    try {
      const nextStatus = await invoke<LocalAiStatus>('local_ai_update_rollback', { component: 'bundle' });
      if (nextStatus) setLocalAiStatus(nextStatus);
      setLocalAiUpdates([]);
    } catch (error) {
      notifyError(t('notification.rollbackLocalAiFailed'), String(error));
    } finally {
      setLocalAiInstalling(null);
    }
  }

  if (subview === 'logs') {
    return <LogPage onBack={() => setSubview('main')} />;
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
            <SettingRow title={t('settings.trayIconColor.label')} hint={t(macPlatform ? 'settings.trayIconColor.hintMac' : 'settings.trayIconColor.hint')}>
              <select value={settings.trayIconColor} onChange={(e) => update({ trayIconColor: e.target.value })} aria-label={t('settings.trayIconColor.label')}>
                <option value="auto">{t(macPlatform ? 'settings.trayIconColor.autoMac' : 'settings.trayIconColor.auto')}</option>
                <option value="white">{t('settings.trayIconColor.white')}</option>
                <option value="black">{t('settings.trayIconColor.black')}</option>
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

          <section className="card settings-section" id="settings-battery-history-section">
            <div className="card-title">
              <div className="settings-title-group">
                <h2>{t('settings.section.batteryHistory')}</h2>
                {batteryAiAnalysisEnabled && (
                  <span className="settings-feature-badge">{t('batteryUsage.aiBadge')}</span>
                )}
              </div>
              <button type="button" className="settings-inline-action" onClick={onOpenBatteryUsage}>
                <ChartBar weight="regular" />
                <span>{t('batteryUsage.viewTrend')}</span>
              </button>
            </div>
            <SettingRow title={t('batteryUsage.recordEnabled')} hint={t('batteryUsage.recordEnabledHint')}>
              <Toggle
                checked={settings.batteryHistoryEnabled}
                onChange={(v) => update({ batteryHistoryEnabled: v })}
                label={t('batteryUsage.recordEnabled')}
              />
            </SettingRow>
            <SettingRow title={t('batteryUsage.aiAnalysisToggle')} hint={t('batteryUsage.aiAnalysisToggleHint')}>
              <Toggle
                checked={batteryAiAnalysisEnabled}
                onChange={(v) => update({
                  localAiAnalysisEnabled: v ? true : settings.localAiAnalysisEnabled,
                  localAiFeatures: setLocalAiFeature(settings.localAiFeatures, LOCAL_AI_FEATURE.batteryUsage, v),
                  ...(v ? { batteryHistoryEnabled: true } : {}),
                })}
                label={t('batteryUsage.aiAnalysisToggle')}
              />
            </SettingRow>
            {settings.batteryHistoryEnabled && (
              <>
                <SettingRow title={t('batteryUsage.retentionDays')} hint={t('batteryUsage.retentionDaysHint')}>
                  <select
                    value={settings.batteryHistoryRetentionDays}
                    onChange={(e) => update({ batteryHistoryRetentionDays: Number(e.target.value) })}
                    aria-label={t('batteryUsage.retentionDays')}
                  >
                    {[3, 7, 10, 14, 30, 60, 90].map((d) => (
                      <option key={d} value={d}>{d}</option>
                    ))}
                  </select>
                </SettingRow>
                <SettingRow title={t('batteryUsage.unusualDrainAlerts')} hint={t('batteryUsage.unusualDrainAlertsHint')}>
                  <Toggle
                    checked={settings.unusualDrainAlerts}
                    onChange={(v) => update({ unusualDrainAlerts: v })}
                    label={t('batteryUsage.unusualDrainAlerts')}
                  />
                </SettingRow>
              </>
            )}
            <div className="battery-history-actions">
              {confirmingClearBattery ? (
                <div className="clear-confirm-bar">
                  <span>{t('batteryUsage.clearConfirm')}</span>
                  <button className="danger" onClick={handleClearBatteryHistory}>{t('batteryUsage.clearHistoryConfirm')}</button>
                  <button onClick={() => setConfirmingClearBattery(false)}>{t('common.cancel')}</button>
                </div>
              ) : (
                <>
                  <button className="action-btn" onClick={() => setConfirmingClearBattery(true)}>
                    {t('batteryUsage.clearHistory')}
                  </button>
                  <button className="action-btn" onClick={() => handleExportBatteryHistory('json')}>
                    {t('batteryUsage.exportJson')}
                  </button>
                  <button className="action-btn" onClick={() => handleExportBatteryHistory('csv')}>
                    {t('batteryUsage.exportCsv')}
                  </button>
                </>
              )}
            </div>
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
                <h3 className="settings-subsection-title">{t('settings.nightMode.triggerSection')}</h3>
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
                <h3 className="settings-subsection-title">{t('settings.nightMode.targetSection')}</h3>
                <SettingRow title={t('settings.nightMode.targetMouse')} hint={t('settings.nightMode.targetMouseHint')}>
                  <Toggle checked={settings.nightModeTargetMouse} onChange={(v) => update({ nightModeTargetMouse: v })} label={t('settings.nightMode.targetMouse')} disabled={!supportsMouseLighting} />
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

          <section className="card settings-section settings-action-card">
            <div className="card-title"><h2>{t('settings.section.config')}</h2></div>
            <div className="settings-action-body">
              <p className="setting-hint">{t('settings.config.hint')}</p>
              <div className="contact-links align-end">
                <button className="secondary" onClick={() => void handleExportConfig()} disabled={previewMode}>{t('settings.config.export')}</button>
                <button className="secondary" onClick={() => void handleImportConfig()} disabled={previewMode}>{t('settings.config.import')}</button>
              </div>
            </div>
          </section>
        </>
      )}

      {tab === 'plugins' && (
        <>
        <section id="settings-local-ai-section" className="card settings-section">
          <div className="card-title">
            <h2>{t('settings.localAi.title')}</h2>
            <span className={`badge ${settings.localAiAnalysisEnabled ? 'badge-ok' : ''}`}>
              {settings.localAiAnalysisEnabled ? t('settings.localAi.enabled') : t('settings.localAi.disabled')}
            </span>
          </div>
          <SettingRow title={t('settings.localAi.toggle')} hint={t('settings.localAi.hint')}>
            <Toggle
              checked={settings.localAiAnalysisEnabled}
              onChange={(v) => update({ localAiAnalysisEnabled: v })}
              label={t('settings.localAi.toggle')}
            />
          </SettingRow>
          <div className="contact-links plugin-update-actions">
            <button
              className="secondary"
              onClick={() => void checkLocalAiUpdates()}
              disabled={previewMode || localAiChecking || Boolean(localAiInstalling)}
            >
              {localAiChecking ? t('settings.localAi.checking') : t('settings.localAi.checkUpdates')}
            </button>
            {localAiStatus.ready && <span className="save-badge">{t('settings.localAi.runtimeReady')}</span>}
          </div>
          <div className="plugin-list">
            <div className="plugin-item">
              <div>
                <strong>{t('settings.localAi.bundle')}</strong>
                <span className="setting-hint">
                  {localAiStatus.runtimeVersion ? `v${localAiStatus.runtimeVersion}` : t('settings.localAi.notInstalled')}
                </span>
              </div>
              <div className="plugin-meta">
                <span className="badge badge-ok">{t('settings.localAi.signedBundle')}</span>
                {!localAiStatus.rollbackAvailable && <span className="badge">{t('settings.localAi.defaultBundled')}</span>}
              </div>
              {localAiUpdates.find((item) => item.component === 'bundle')?.updateAvailable && (
                <div className="plugin-update-row">
                  <span className="setting-hint">{t('settings.localAi.updatable', { version: localAiUpdates.find((item) => item.component === 'bundle')?.availableVersion })}</span>
                  <button className="primary" disabled={Boolean(localAiInstalling)} onClick={() => void installLocalAiUpdate()}>
                    {localAiInstalling === 'bundle' ? t('settings.localAi.updating') : t('settings.localAi.updateBundle')}
                  </button>
                </div>
              )}
              {localAiStatus.rollbackAvailable && (
                <button className="secondary" disabled={Boolean(localAiInstalling)} onClick={() => void rollbackLocalAi()}>
                  {t('settings.localAi.rollbackBundle')}
                </button>
              )}
            </div>
          </div>
          {!localAiStatus.ready && localAiStatus.error && (
            <p className="setting-hint">{t(`settings.localAi.status.${localAiStatus.error}`, { defaultValue: t('settings.localAi.runtimeUnavailable') })}</p>
          )}
        </section>
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
                          onClick={() => void handlePluginUpdateInstall(plugin.pluginId)}
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
        </>
      )}

      {tab === 'privacy' && (
        <section className="card settings-section">
          <div className="card-title"><h2>{t('settings.section.privacy')}</h2></div>
          <SettingRow title={t('settings.privacy.telemetryLabel')} hint={t('settings.privacy.telemetryHint')}>
            <Toggle checked={true} onChange={() => {}} label={t('settings.privacy.telemetryLabel')} disabled showOnWhenDisabled />
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
          <section className="card settings-section settings-action-card">
            <div className="card-title"><h2>{t('settings.section.about')}</h2></div>
            <div className="settings-action-body">
              <div className="setting-label">
                <strong>{t('settings.about.label')}</strong>
                <span className="setting-hint">{t('settings.about.hint')}</span>
              </div>
              <button className="secondary" onClick={onNavigateAbout}>{t('settings.about.button')}</button>
            </div>
          </section>

          <section className="card settings-section settings-action-card logs-card">
            <div className="card-title"><h2>{t('logs.title')}</h2></div>
            <div className="settings-action-body">
              <div className="settings-action-copy">
                <p className="setting-hint">{t('logs.cardHint')}</p>
                <p className="setting-hint">{t('logs.cardPrivacy')}</p>
              </div>
              <button className="primary" onClick={() => setSubview('logs')}>{t('logs.openButton')}</button>
            </div>
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
