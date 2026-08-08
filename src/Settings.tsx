// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useEffectEvent, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ChartBar } from '@phosphor-icons/react';
import type { AppSettings, BundledPluginInfo, AboutInfo, DiscoveredDevice, LocalAiComponent, LocalAiStatus, PluginCapability, ThemeMode } from './types';
import { Tooltip } from './Tooltip';
import { notifyError, notifyInfo } from './notify';
import { useScrollFadeState } from './useScrollOverflow';
import { extractChannel, exportDiagnostics } from './plugin-utils';
import { resolveLightingMutations, resolveLightingRoles } from './pluginAdapter';
import { applyLanguage, type AppLanguage } from './i18n';
import { save, open } from '@tauri-apps/plugin-dialog';
import { ExternalLink } from './ExternalLink';
import { startAutomaticAppUpdateCheck } from './updater';
import { checkForPluginUpdates, installPluginUpdate, onPluginUpdateState, pluginUpdateState, startAutomaticPluginUpdateCheck, type PluginUpdateState } from './plugin-updater';
import {
  checkForLocalAiUpdates,
  installLocalAiUpdate,
  localAiComponentLabel,
  onLocalAiUpdateState,
  localAiUpdateState,
  rollbackLocalAiUpdate,
  type LocalAiUpdateState,
} from './local-ai-updater';
import { friendlyUpdateError } from './update-errors';
import { DEFAULT_LOCAL_AI_FEATURES, LOCAL_AI_FEATURE, localAiFeatureEnabled, setLocalAiFeature } from './localAi';
import { MiraActivityButton, announceAfterOrbExit } from './activity';
import {
  ATTENTION_PRIORITY,
  AttentionBeamLayer,
  AttentionSurface,
  attentionDesaturatedAccent,
  attentionLocalAiInstalledKey,
  attentionLocalAiUpdateKey,
  attentionPluginInstalledKey,
  attentionPluginUpdateKey,
  attentionSectionFocusKey,
  resolveUpdateAttentionTarget,
  useAttentionFeedback,
  type AttentionBeamRequest,
} from './attention';

const DEFAULT_SETTINGS: AppSettings = {
  language: 'auto',
  theme: 'system',
  autostart: false,
  startHidden: false,
  trayShowBatteryTitle: true,
  trayIncludeReceiverBattery: false,
  trayShowConnection: true,
  trayShowBatteryIcon: false,
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
  automaticLocalAiUpdateChecks: true,
  localAiAnalysisEnabled: false,
  localAiFeatures: { ...DEFAULT_LOCAL_AI_FEATURES },
  batteryHistoryEnabled: true,
  batteryHistoryRetentionDays: 30,
  unusualDrainAlerts: false,
  wakeOnUnlock: false,
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

// 演示模式下模拟内置 AI 引擎状态：runtime 为最新版 1.0.0，未更新过（无 rollback）。
const PREVIEW_LOCAL_AI_STATUS: LocalAiStatus = {
  ready: true,
  runtimeVersion: '1.0.0',
  bundleVersion: '1.0.0',
  modelPackId: 'mira-battery-model',
  modelPackVersion: '0.8.3',
  handlerId: 'mira.battery.handler',
  handlerVersion: '0.8.5',
  handlerApiVersion: 1,
  rollbackAvailable: false,
};

export type SettingsTab = 'general' | 'device' | 'plugins' | 'privacy' | 'about';

// 本地 AI 引擎的三个独立发布工件：Rill 运行时 / Mira 模型 / Mira 处理器。
// 三者独立检查更新与通知，但安装/回退仍作为一次事务性部署整体进行。
const LOCAL_AI_COMPONENTS: LocalAiComponent[] = ['runtime', 'model', 'handler'];

function localAiStatusVersion(status: LocalAiStatus, component: LocalAiComponent): string | undefined {
  switch (component) {
    case 'runtime':
      return status.runtimeVersion;
    case 'model':
      return status.modelPackVersion;
    case 'handler':
      return status.handlerVersion;
  }
}

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

// 必须与 App.tsx 同名函数保持一致
function isWindowsPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'windows' || navigator.userAgent.includes('Windows');
}

function isMacPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'macos'
    || (previewPlatform === null && /Macintosh|Mac OS X/.test(navigator.userAgent));
}

export function SettingsPage({ onNavigateAbout, onNavigateLogs = () => {}, onOpenBatteryUsage = () => {}, onBatteryUsageSettingsChange, onThemeChange, previewMode = false, pluginCapabilities = [], writableMutations = [], focusPluginUpdateToken = 0, focusLocalAiUpdateToken = 0, initialTab = 'general', onTabChange, onSettingsExit }: { onNavigateAbout: () => void; onNavigateLogs?: () => void; onOpenBatteryUsage?: () => void; onBatteryUsageSettingsChange?: (settings: { batteryHistoryEnabled: boolean; aiAnalysisEnabled: boolean }) => void; onThemeChange: (theme: ThemeMode) => void; previewMode?: boolean; pluginCapabilities?: PluginCapability[]; writableMutations?: string[]; focusPluginUpdateToken?: number; focusLocalAiUpdateToken?: number; initialTab?: SettingsTab; onTabChange?: (tab: SettingsTab) => void; onSettingsExit?: () => void }) {
  const { t } = useTranslation();
  const windowsPlatform = isWindowsPlatform();
  const macPlatform = isMacPlatform();
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const { canScrollUp, canScrollDown } = useScrollFadeState(scrollRef, contentRef);
  // 乐观快照 + 串行化保存合并，避免拖动 range 时竞态、写盘队列堆积、settings.json.tmp 争用。
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
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus>(previewMode ? PREVIEW_LOCAL_AI_STATUS : EMPTY_LOCAL_AI_STATUS);
  const [localAiUpdate, setLocalAiUpdate] = useState<LocalAiUpdateState>(localAiUpdateState());
  const [diagnostics, setDiagnostics] = useState<string>('');
  const [discovered, setDiscovered] = useState<DiscoveredDevice[]>([]);
  const [saved, setSaved] = useState(false);
  const [confirmingClearBattery, setConfirmingClearBattery] = useState(false);
  const [deviceScanBusy, setDeviceScanBusy] = useState(false);
  const [diagnosticsExportBusy, setDiagnosticsExportBusy] = useState(false);
  const [configExportBusy, setConfigExportBusy] = useState(false);
  const [configImportBusy, setConfigImportBusy] = useState(false);
  const [batteryExportBusy, setBatteryExportBusy] = useState<'json' | 'csv' | null>(null);
  const [localAiRollbackBusy, setLocalAiRollbackBusy] = useState(false);
  // 手动检查的局部 busy：与自动后台检查解耦，避免自动检查时页面出现 Orb。
  const [pluginCheckBusy, setPluginCheckBusy] = useState(false);
  const [localAiCheckBusy, setLocalAiCheckBusy] = useState(false);
  const [tabState, setTabState] = useState<{ tab: SettingsTab; focusToken: number }>(() => ({
    tab: focusPluginUpdateToken > 0 || focusLocalAiUpdateToken > 0 ? 'plugins' : initialTab,
    focusToken: focusPluginUpdateToken,
  }));
  // tab 切换过渡：targetTab 控制高亮，displayedTab 控制实际渲染的内容。
  // 切换时先在旧内容上播放退出动画（is-exiting），动画结束后才切换 displayedTab，
  // 新内容因 key 变化重新挂载自动触发依次淡入。
  const [displayedTab, setDisplayedTab] = useState<SettingsTab>(tabState.tab);
  const [exiting, setExiting] = useState(false);
  const exitTimer = useRef<number | undefined>(undefined);
  const pendingPluginFocus = useRef(false);
  const pendingLocalAiFocus = useRef(false);
  // 任一焦点 token 增长时强制切到 plugins 标签，待渲染后由专属 effect 滚动聚焦。
  const tab = focusPluginUpdateToken > tabState.focusToken || focusLocalAiUpdateToken > tabState.focusToken
    ? 'plugins'
    : tabState.tab;
  const pluginUpdates = pluginUpdate.updates;
  const pluginUpdatesChecking = pluginUpdate.phase === 'checking';
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

  // 区块级强调：从通知跳转定位到「插件更新」/「本地 AI」整块时，用 Beam 替代
  // 已移除的 focus outline。独立 scope，避免与固定行的 row-level Beam 混淆。
  const pluginSectionAttention = useAttentionFeedback('settings-plugin-section');
  const localAiSectionAttention = useAttentionFeedback('settings-local-ai-section');
  const pluginSectionAnnounce = pluginSectionAttention.announce;
  const localAiSectionAnnounce = localAiSectionAttention.announce;
  const pluginSectionFocusSeq = useRef(0);
  const localAiSectionFocusSeq = useRef(0);

  // 点击「插件更新可用」通知后，先切到 plugins 标签，待渲染后再滚动聚焦。
  useEffect(() => {
    if (focusPluginUpdateToken === 0) return;
    pendingPluginFocus.current = true;
  }, [focusPluginUpdateToken]);

  useEffect(() => {
    if (!pendingPluginFocus.current || displayedTab !== 'plugins') return;
    pendingPluginFocus.current = false;
    const target = document.getElementById('settings-plugin-update-section');
    target?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
    target?.focus?.({ preventScroll: true });
    // 从通知跳转定位到整块：用 Beam 替代已移除的 focus outline 做视觉强调。
    const seq = ++pluginSectionFocusSeq.current;
    pluginSectionAnnounce({
      eventKey: attentionSectionFocusKey('settings-plugin', seq),
      scope: 'settings-plugin-section',
      variant: 'line',
      color: attentionDesaturatedAccent(),
      durationMs: 1500,
      strength: 0.2,
      cycles: 1,
      priority: ATTENTION_PRIORITY['update-available'],
    });
  }, [displayedTab, focusPluginUpdateToken, pluginSectionAnnounce]);

  // 点击「本地 AI 更新可用」通知后，先切到 plugins 标签，再滚动到 AI 引擎卡片。
  useEffect(() => {
    if (focusLocalAiUpdateToken === 0) return;
    pendingLocalAiFocus.current = true;
  }, [focusLocalAiUpdateToken]);

  useEffect(() => {
    if (!pendingLocalAiFocus.current || displayedTab !== 'plugins') return;
    pendingLocalAiFocus.current = false;
    const target = document.getElementById('settings-local-ai-section');
    target?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
    target?.focus?.({ preventScroll: true });
    // 从通知跳转定位到整块：用 Beam 替代已移除的 focus outline 做视觉强调。
    const seq = ++localAiSectionFocusSeq.current;
    localAiSectionAnnounce({
      eventKey: attentionSectionFocusKey('settings-local-ai', seq),
      scope: 'settings-local-ai-section',
      variant: 'line',
      color: attentionDesaturatedAccent(),
      durationMs: 1500,
      strength: 0.2,
      cycles: 1,
      priority: ATTENTION_PRIORITY['update-available'],
    });
  }, [displayedTab, focusLocalAiUpdateToken, localAiSectionAnnounce]);

  // 设置页卸载时通知父组件清除更新聚焦 token，避免下次进入时重复跳转和重复显示"已更新至"标签。
  useEffect(() => () => onSettingsExit?.(), [onSettingsExit]);

  // 把当前「实际渲染」的标签上抛给父组件（不是用户点击后的目标标签）：
  // 标签切换有约 120ms 过渡，displayedTab 只在退出动画结束后才变化；
  // 父级据此做更新事件的可见性仲裁，避免过渡期间误判目标标签已可见。
  // 同时使设置页在卸载/重建（例如进入关于页再返回）后能恢复到用户先前
  // 所在的标签，而不是每次都落回首个标签。
  useEffect(() => {
    onTabChange?.(displayedTab);
  }, [displayedTab, onTabChange]);

  // tab 切换过渡：tab（目标）变化时先播放退出动画，动画结束后切换 displayedTab。
  // 新内容因 key 变化重新挂载，自动触发依次淡入。
  useEffect(() => {
    if (tab === displayedTab) return;
    window.clearTimeout(exitTimer.current);
    // 用 rAF 延迟一帧设置 exiting，确保浏览器先应用 key 变化前的旧 DOM 状态，
    // 再添加 is-exiting class 触发退出动画。
    const raf = requestAnimationFrame(() => setExiting(true));
    exitTimer.current = window.setTimeout(() => {
      setDisplayedTab(tab);
      setExiting(false);
    }, 120);
    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(exitTimer.current);
    };
  }, [tab, displayedTab]);

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
    if (previewMode) return;
    invoke<LocalAiStatus>('local_ai_status')
      .then((status) => status && setLocalAiStatus(status))
      .catch(() => setLocalAiStatus(EMPTY_LOCAL_AI_STATUS));
  }, [onThemeChange, previewMode]);

  useEffect(() => onPluginUpdateState(setPluginUpdate), []);

  // 订阅 local-ai-updater 状态，并在安装/回滚完成时刷新后端 status。
  useEffect(() => onLocalAiUpdateState((next) => {
    setLocalAiUpdate(next);
    if (next.phase === 'installed') {
      invoke<LocalAiStatus>('local_ai_status')
        .then((status) => status && setLocalAiStatus(status))
        .catch(() => {});
    }
  }), []);

  // 本地 AI 出错后保留回退入口；恢复正常运行一段时间后清除错误状态，回退按钮随之消失。
  useEffect(() => {
    if (localAiUpdate.phase !== 'error') return;
    const timer = setTimeout(() => {
      setLocalAiUpdate((current) => (current.phase === 'error' ? { ...current, phase: 'idle', error: undefined } : current));
    }, 10 * 60 * 1000);
    return () => clearTimeout(timer);
  }, [localAiUpdate.phase, localAiUpdate.error]);

  // ── Attention Beam：插件 / 本地 AI 固定更新行（§5.7、§5.8、§11） ──────
  // 只在真实的 updateAvailable 迁移（或安装完成）时播放一次；初次挂载时
  // 已有状态不触发；多个插件可更新时只强调第一个新出现的更新行。
  // 固定行只在 plugins 标签实际可见时才有权 announce；其余情况交给通知浮层
  // 消费（仲裁见 attentionTypes.resolveUpdateAttentionTarget，P0-2）。
  const pluginRowAttention = useAttentionFeedback('settings-plugin');
  const localAiRowAttention = useAttentionFeedback('settings-local-ai');
  const pluginRowAnnounce = pluginRowAttention.announce;
  const localAiRowAnnounce = localAiRowAttention.announce;

  const prevAvailablePluginsRef = useRef<string[] | undefined>(undefined);
  const prevAvailableLocalAiRef = useRef<Map<LocalAiComponent, string> | undefined>(undefined);
  const prevPluginPhaseRef = useRef<PluginUpdateState['phase'] | undefined>(undefined);
  const prevLocalAiPhaseRef = useRef<LocalAiUpdateState['phase'] | undefined>(undefined);

  useEffect(() => {
    const available = pluginUpdate.updates
      .filter((item) => item.updateAvailable && item.availableVersion)
      .map((item) => `${item.pluginId}@${item.availableVersion}`);
    if (prevAvailablePluginsRef.current === undefined) {
      prevAvailablePluginsRef.current = available;
      return;
    }
    const prev = prevAvailablePluginsRef.current;
    prevAvailablePluginsRef.current = available;
    const fresh = available.find((key) => !prev.includes(key));
    if (fresh && resolveUpdateAttentionTarget('plugin', { view: 'settings', settingsTab: displayedTab }) === 'settings-plugin') {
      const [pluginId, version] = fresh.split('@');
      announceAfterOrbExit('settings-plugin', pluginRowAnnounce, {
        eventKey: attentionPluginUpdateKey(pluginId, String(version)),
        scope: 'settings-plugin',
        variant: 'line',
        color: attentionDesaturatedAccent(),
        durationMs: 1600,
        strength: 0.18,
        cycles: 1,
        priority: ATTENTION_PRIORITY['update-available'],
      });
    }
  }, [pluginUpdate, pluginRowAnnounce, displayedTab]);

  useEffect(() => {
    const currentAvailable = new Map<LocalAiComponent, string>();
    for (const item of localAiUpdate.updates) {
      if (item.updateAvailable && item.availableVersion) {
        currentAvailable.set(item.component, item.availableVersion);
      }
    }
    const prev = prevAvailableLocalAiRef.current;
    prevAvailableLocalAiRef.current = currentAvailable;
    if (!prev) return;
    if (resolveUpdateAttentionTarget('local-ai', { view: 'settings', settingsTab: displayedTab }) !== 'settings-local-ai') return;
    // 每个独立的组件新出现可用更新时，都播放一次光束；组件内版本迁移也触发。
    for (const [component, version] of currentAvailable) {
      if (prev.get(component) !== version) {
        announceAfterOrbExit('settings-local-ai', localAiRowAnnounce, {
          eventKey: attentionLocalAiUpdateKey(component, version),
          scope: 'settings-local-ai',
          variant: 'line',
          color: attentionDesaturatedAccent(),
          durationMs: 1600,
          strength: 0.18,
          cycles: 1,
          priority: ATTENTION_PRIORITY['update-available'],
        });
      }
    }
  }, [localAiUpdate, localAiRowAnnounce, displayedTab]);

  // 插件安装完成 Flash：只在 plugins 标签实际可见时提交（§7.1）。
  // phase 已处于 installed 后不再重复尝试（其他依赖变化不触发重播）；
  // 安装完成时不可见 → 不提交、不消费 eventKey、不占全局队列。
  useEffect(() => {
    const prev = prevPluginPhaseRef.current;
    prevPluginPhaseRef.current = pluginUpdate.phase;
    if (prev === undefined || prev === 'installed') return;
    if (pluginUpdate.phase !== 'installed') return;
    const target = resolveUpdateAttentionTarget('plugin', { view: 'settings', settingsTab: displayedTab });
    if (target !== 'settings-plugin') return;
    if (pluginUpdate.lastInstalledPluginId && pluginUpdate.lastInstalledVersion) {
      announceAfterOrbExit('settings-plugin', pluginRowAnnounce, {
        eventKey: attentionPluginInstalledKey(pluginUpdate.lastInstalledPluginId, pluginUpdate.lastInstalledVersion),
        scope: 'settings-plugin',
        variant: 'flash',
        color: attentionDesaturatedAccent(),
        durationMs: 950,
        strength: 0.2,
        cycles: 1,
        priority: ATTENTION_PRIORITY['update-installed'],
      });
    }
  }, [pluginUpdate, pluginRowAnnounce, displayedTab]);

  // Local AI 安装完成 Flash：与插件相同门控（§7.2）。
  useEffect(() => {
    const prev = prevLocalAiPhaseRef.current;
    prevLocalAiPhaseRef.current = localAiUpdate.phase;
    if (prev === undefined || prev === 'installed') return;
    if (localAiUpdate.phase !== 'installed') return;
    const target = resolveUpdateAttentionTarget('local-ai', { view: 'settings', settingsTab: displayedTab });
    if (target !== 'settings-local-ai') return;
    // 对本次实际更新的每个组件播放一次安装完成短闪。
    for (const component of localAiUpdate.updatedComponents ?? []) {
      const version = localAiUpdate.updates.find((item) => item.component === component)?.currentVersion;
      if (version) {
        announceAfterOrbExit('settings-local-ai', localAiRowAnnounce, {
          eventKey: attentionLocalAiInstalledKey(component, version),
          scope: 'settings-local-ai',
          variant: 'flash',
          color: attentionDesaturatedAccent(),
          durationMs: 950,
          strength: 0.2,
          cycles: 1,
          priority: ATTENTION_PRIORITY['update-installed'],
        });
      }
    }
  }, [localAiUpdate, localAiRowAnnounce, displayedTab]);

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
    setDiagnosticsExportBusy(true);
    try {
      const result = await exportDiagnostics();
      if (result !== undefined) setDiagnostics(result);
    } finally {
      setDiagnosticsExportBusy(false);
    }
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
      setConfigExportBusy(true);
      await invoke('device_config_export', { path });
      notifyInfo(t('notification.exportSuccess'), t('notification.exportSuccessBody', { path }));
    } catch (error) {
      notifyError(t('notification.exportFailed'), String(error));
    } finally {
      setConfigExportBusy(false);
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
      setConfigImportBusy(true);
      await invoke('device_config_import', { path: selected });
      notifyInfo(t('notification.importSuccess'), t('notification.importSuccessBody'));
    } catch (error) {
      notifyError(t('notification.importFailed'), String(error));
    } finally {
      setConfigImportBusy(false);
    }
  }

  async function scanDevices() {
    setDeviceScanBusy(true);
    try {
      setDiscovered(await invoke<DiscoveredDevice[]>('discover_devices'));
    } catch (err) {
      notifyError(t('notification.scanFailed'), String(err));
    } finally {
      setDeviceScanBusy(false);
    }
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
      setBatteryExportBusy(format);
      await invoke('battery_history_export', { format, path });
      notifyInfo(t('batteryUsage.exportDone'), '');
    } catch (err) {
      notifyError(t('batteryUsage.exportFailed'), String(err));
    } finally {
      setBatteryExportBusy(null);
    }
  }

  async function checkPluginUpdates() {
    if (previewMode) return;
    setPluginCheckBusy(true);
    try {
      await checkForPluginUpdates();
    } catch (error) {
      notifyError(t('notification.checkPluginUpdateFailed'), friendlyUpdateError(error));
    } finally {
      setPluginCheckBusy(false);
    }
  }

  async function handlePluginUpdateInstall(pluginId: string) {
    try {
      const result = await installPluginUpdate(pluginId);
      setPlugins((current) => current.map((plugin) => plugin.pluginId === result.pluginId
        ? { ...plugin, version: result.version, source: 'installed', signatureVerified: true }
        : plugin));
    } catch (error) {
      notifyError(t('notification.installPluginUpdateFailed'), friendlyUpdateError(error));
    }
  }

  async function checkLocalAiUpdates() {
    if (previewMode) return;
    setLocalAiCheckBusy(true);
    try {
      await checkForLocalAiUpdates();
    } catch (error) {
      notifyError(t('notification.checkLocalAiUpdateFailed'), friendlyUpdateError(error));
    } finally {
      setLocalAiCheckBusy(false);
    }
  }

  async function handleLocalAiInstall() {
    if (previewMode) return;
    try {
      await installLocalAiUpdate();
    } catch (error) {
      notifyError(t('notification.installLocalAiUpdateFailed'), friendlyUpdateError(error));
    }
  }

  async function handleLocalAiRollback() {
    if (previewMode) return;
    setLocalAiRollbackBusy(true);
    try {
      const nextStatus = await rollbackLocalAiUpdate();
      if (nextStatus) setLocalAiStatus(nextStatus);
    } catch (error) {
      notifyError(t('notification.rollbackLocalAiFailed'), friendlyUpdateError(error));
    } finally {
      setLocalAiRollbackBusy(false);
    }
  }

  // available 绕行光只属于更新提示行；installed 完成闪光属于始终挂载的
  // 组件卡片。不能把两者都挂到条件渲染的更新行，否则安装成功后该行卸载，
  // 总线虽已消费事件，DOM 中却没有可见的 Beam。
  const localAiAvailableRowBeamFor = (component: LocalAiComponent): AttentionBeamRequest | null => {
    const beam = localAiRowAttention.beam;
    if (!beam) return null;
    const updateItem = localAiUpdate.updates.find((item) => item.component === component);
    if (updateItem?.updateAvailable && updateItem.availableVersion
      && beam.eventKey === attentionLocalAiUpdateKey(component, updateItem.availableVersion)) {
      return beam;
    }
    return null;
  };
  const localAiInstalledItemBeamFor = (component: LocalAiComponent): AttentionBeamRequest | null => {
    const beam = localAiRowAttention.beam;
    if (!beam) return null;
    if ((localAiUpdate.updatedComponents ?? []).includes(component)) {
      const currentVersion = localAiUpdate.updates.find((item) => item.component === component)?.currentVersion;
      if (currentVersion && beam.eventKey === attentionLocalAiInstalledKey(component, currentVersion)) {
        return beam;
      }
    }
    return null;
  };
  const pluginAvailableRowBeamFor = (pluginId: string): AttentionBeamRequest | null => {
    const beam = pluginRowAttention.beam;
    if (!beam) return null;
    const updateItem = pluginUpdate.updates.find((item) => item.pluginId === pluginId);
    if (updateItem?.updateAvailable && updateItem.availableVersion
      && beam.eventKey === attentionPluginUpdateKey(pluginId, updateItem.availableVersion)) {
      return beam;
    }
    return null;
  };
  const pluginInstalledItemBeamFor = (pluginId: string): AttentionBeamRequest | null => {
    const beam = pluginRowAttention.beam;
    if (!beam) return null;
    if (pluginUpdate.lastInstalledPluginId === pluginId && pluginUpdate.lastInstalledVersion
      && beam.eventKey === attentionPluginInstalledKey(pluginId, pluginUpdate.lastInstalledVersion)) {
      return beam;
    }
    return null;
  };

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
            onClick={() => setTabState({ tab: tabItem.id, focusToken: Math.max(focusPluginUpdateToken, focusLocalAiUpdateToken) })}
            aria-pressed={tab === tabItem.id}
          >
            {tabItem.label}
          </button>
        ))}
      </nav>

      <div ref={scrollRef} className={`settings-scroll-area${canScrollUp ? ' scroll-fade-top' : ''}${canScrollDown ? ' scroll-fade-bottom' : ''}`}>
      <div ref={contentRef} key={displayedTab} className={`settings-scroll-content${exiting ? ' is-exiting' : ''}`}>
      {displayedTab === 'general' && (
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
            <SettingRow title={t('settings.trayBattery.label')} hint={t(windowsPlatform ? 'settings.trayBattery.hintWindows' : 'settings.trayBattery.hint')}>
              {windowsPlatform ? (
                <Toggle checked={settings.trayShowBatteryIcon} onChange={(v) => update({ trayShowBatteryIcon: v })} label={t('settings.trayBattery.label')} />
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
            <SettingRow title={t('settings.wakeOnUnlock.label')} hint={t('settings.wakeOnUnlock.hint')}>
              <Toggle checked={settings.wakeOnUnlock} onChange={(v) => update({ wakeOnUnlock: v })} label={t('settings.wakeOnUnlock.label')} />
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

      {displayedTab === 'device' && (
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
                  <MiraActivityButton
                    className="action-btn"
                    active={batteryExportBusy === 'json'}
                    activity="exporting-battery-history"
                    onClick={() => handleExportBatteryHistory('json')}
                    disabled={batteryExportBusy !== null}
                  >
                    {t('batteryUsage.exportJson')}
                  </MiraActivityButton>
                  <MiraActivityButton
                    className="action-btn"
                    active={batteryExportBusy === 'csv'}
                    activity="exporting-battery-history"
                    onClick={() => handleExportBatteryHistory('csv')}
                    disabled={batteryExportBusy !== null}
                  >
                    {t('batteryUsage.exportCsv')}
                  </MiraActivityButton>
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
                <div className="settings-subsection">
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
                </div>
                <div className="settings-subsection">
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
                </div>
              </>
            )}
          </section>

          <section className="card settings-section settings-action-card">
            <div className="card-title"><h2>{t('settings.section.config')}</h2></div>
            <div className="settings-action-body">
              <p className="setting-hint">{t('settings.config.hint')}</p>
              <div className="contact-links align-end">
                <MiraActivityButton
                  className="secondary"
                  active={configExportBusy}
                  activity="exporting-device-config"
                  onClick={() => void handleExportConfig()}
                  disabled={previewMode || configImportBusy}
                >
                  {t('settings.config.export')}
                </MiraActivityButton>
                <MiraActivityButton
                  className="secondary"
                  active={configImportBusy}
                  activity="importing-device-config"
                  onClick={() => void handleImportConfig()}
                  disabled={previewMode || configExportBusy}
                >
                  {t('settings.config.import')}
                </MiraActivityButton>
              </div>
            </div>
          </section>
        </>
      )}

      {displayedTab === 'plugins' && (
        <>
        <section id="settings-local-ai-section" className="card settings-section" tabIndex={-1} style={localAiSectionAttention.beam ? { position: 'relative' } : undefined}>
          {localAiSectionAttention.beam && <AttentionBeamLayer active request={localAiSectionAttention.beam} />}
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
          <SettingRow title={t('settings.localAi.automaticCheckLabel')} hint={t('settings.localAi.automaticCheckHint')}>
            <Toggle
              checked={settings.automaticLocalAiUpdateChecks}
              onChange={(v) => update({ automaticLocalAiUpdateChecks: v })}
              label={t('settings.localAi.automaticCheckLabel')}
            />
          </SettingRow>
          <div className="contact-links plugin-update-actions align-end">
            <MiraActivityButton
              className="secondary"
              active={localAiCheckBusy}
              activity="checking-local-ai-updates"
              onClick={() => void checkLocalAiUpdates()}
              disabled={previewMode || localAiUpdate.phase === 'checking' || localAiUpdate.phase === 'rolling-back' || localAiUpdate.phase === 'downloading'}
            >
              {t('settings.localAi.checkUpdates')}
            </MiraActivityButton>
            {localAiStatus.ready && <span className="save-badge">{t('settings.localAi.runtimeReady')}</span>}
          </div>
          <div className="plugin-list">
            {LOCAL_AI_COMPONENTS.map((component) => {
              const updateItem = localAiUpdate.updates.find((item) => item.component === component);
              const version = localAiStatusVersion(localAiStatus, component);
              const wasUpdated = localAiUpdate.phase === 'installed'
                && (localAiUpdate.updatedComponents ?? []).includes(component);
              return (
                <AttentionSurface key={component} className="plugin-item" beam={localAiInstalledItemBeamFor(component)}>
                  <div>
                    <strong>{localAiComponentLabel(component)}</strong>
                    <span className="setting-hint">{version ? `v${version}` : t('settings.localAi.notInstalled')}</span>
                  </div>
                  <div className="plugin-meta">
                    <span className="badge badge-ok">{t('settings.localAi.signedBundle')}</span>
                    {wasUpdated && (
                      <span className="badge badge-ok">{t('settings.localAi.updated', { version: updateItem?.currentVersion ?? version })}</span>
                    )}
                  </div>
                  {updateItem?.updateAvailable && (
                    <AttentionSurface className="plugin-update-row" beam={localAiAvailableRowBeamFor(component)}>
                      <span className="setting-hint">{t('settings.localAi.updatable', { version: updateItem.availableVersion })}</span>
                      <button className="primary" disabled={localAiUpdate.phase === 'downloading'} onClick={() => void handleLocalAiInstall()}>
                        {localAiUpdate.phase === 'downloading' ? t('settings.localAi.updating') : t('settings.localAi.updateBundle')}
                      </button>
                    </AttentionSurface>
                  )}
                </AttentionSurface>
              );
            })}
          </div>
          {localAiUpdate.phase === 'downloading' && (
            <div className="update-progress" aria-live="polite">
              <progress value={localAiUpdate.downloadedBytes} max={localAiUpdate.totalBytes || undefined} />
              <span>
                {localAiUpdate.stage
                  ? t('about.downloadedPercentWithStage', {
                      percent: localAiUpdate.totalBytes
                        ? Math.min(100, Math.round((localAiUpdate.downloadedBytes / localAiUpdate.totalBytes) * 100))
                        : 0,
                      stage: t(`settings.localAi.stage.${localAiUpdate.stage}`),
                    })
                  : localAiUpdate.totalBytes
                    ? t('about.downloadedPercent', { percent: Math.min(100, Math.round((localAiUpdate.downloadedBytes / localAiUpdate.totalBytes) * 100)) })
                    : t('about.downloadedMib', { mib: (localAiUpdate.downloadedBytes / 1024 / 1024).toFixed(1) })}
              </span>
            </div>
          )}
          {localAiStatus.rollbackAvailable && (localAiUpdate.phase === 'error' || localAiUpdate.phase === 'rolling-back') && (
            <div className="plugin-update-row">
              {localAiStatus.previousVersion && localAiUpdate.phase !== 'rolling-back' && (
                <div className="plugin-version-label">
                  <strong>{t('settings.localAi.previousVersion')}</strong>
                  <span className="setting-hint">v{localAiStatus.previousVersion}</span>
                </div>
              )}
              <MiraActivityButton
                className="secondary"
                active={localAiRollbackBusy}
                activity="restoring-local-ai"
                disabled={localAiUpdate.phase === 'rolling-back'}
                onClick={() => void handleLocalAiRollback()}
              >
                {t('settings.localAi.rollbackBundle')}
              </MiraActivityButton>
            </div>
          )}
          {!localAiStatus.ready && localAiStatus.error && (
            <p className="setting-hint">{t(`settings.localAi.status.${localAiStatus.error}`, { defaultValue: t('settings.localAi.runtimeUnavailable') })}</p>
          )}
          {localAiUpdate.phase === 'error' && localAiUpdate.error && (
            <p className="setting-hint update-error">{localAiUpdate.error}</p>
          )}
        </section>
        <section id="settings-plugin-update-section" className="card settings-section" tabIndex={-1} style={pluginSectionAttention.beam ? { position: 'relative' } : undefined}>
          {pluginSectionAttention.beam && <AttentionBeamLayer active request={pluginSectionAttention.beam} />}
          <div className="card-title"><h2>{t('settings.section.plugins')}</h2></div>
          <SettingRow title={t('settings.pluginUpdateCheck.label')} hint={t('settings.pluginUpdateCheck.hint')}>
            <Toggle checked={settings.automaticPluginUpdateChecks} onChange={(v) => update({ automaticPluginUpdateChecks: v })} label={t('settings.pluginUpdateCheck.label')} />
          </SettingRow>
          <div className="contact-links plugin-update-actions align-end">
            <MiraActivityButton
              className="secondary"
              active={pluginCheckBusy}
              activity="checking-plugin-updates"
              onClick={() => void checkPluginUpdates()}
              disabled={previewMode || pluginUpdatesChecking || pluginUpdate.phase === 'downloading'}
            >
              {t('settings.pluginUpdate.check')}
            </MiraActivityButton>
            {pluginUpdates.length > 0 && pluginUpdates.every((item) => !item.updateAvailable) && <span className="save-badge">{t('settings.pluginUpdate.allLatest')}</span>}
          </div>
          {plugins.length === 0 ? (
            <p className="setting-hint">{t('settings.pluginUpdate.noPlugins')}</p>
          ) : (
            <div className="plugin-list">
              {plugins.map((plugin) => {
                const channel = extractChannel(plugin.releaseTag);
                const isInstallingThis = pluginUpdate.phase === 'downloading' && pluginUpdate.installingPluginId === plugin.pluginId;
                return (
                  <AttentionSurface key={plugin.pluginId} className="plugin-item" beam={pluginInstalledItemBeamFor(plugin.pluginId)}>
                    <div>
                      <strong>{plugin.pluginId}</strong>
                      <span className="setting-hint">v{plugin.version}</span>
                    </div>
                    <div className="plugin-meta">
                      {channel && <span className="badge">{channel}</span>}
                      <span className={`badge ${plugin.signatureVerified ? 'badge-ok' : 'badge-warn'}`}>
                        {plugin.signatureVerified ? t('settings.pluginUpdate.signatureVerified') : t('settings.pluginUpdate.signatureUnverified')}
                      </span>
                      {focusPluginUpdateToken > 0 && pluginUpdate.phase === 'installed' && pluginUpdate.lastInstalledPluginId === plugin.pluginId && (
                        <span className="badge badge-ok">{t('settings.pluginUpdate.updated', { version: pluginUpdate.lastInstalledVersion })}</span>
                      )}
                      {plugin.bundleByDefault && <span className="badge">{t('settings.pluginUpdate.defaultBundled')}</span>}
                    </div>
                    {pluginUpdates.find((item) => item.pluginId === plugin.pluginId)?.updateAvailable && (
                      <AttentionSurface className="plugin-update-row" beam={pluginAvailableRowBeamFor(plugin.pluginId)}>
                        <span className="setting-hint">{t('settings.pluginUpdate.updatable', { version: pluginUpdates.find((item) => item.pluginId === plugin.pluginId)?.availableVersion })}</span>
                        <button
                          className="primary"
                          disabled={pluginUpdate.phase === 'downloading'}
                          onClick={() => void handlePluginUpdateInstall(plugin.pluginId)}
                        >
                          {isInstallingThis ? t('settings.pluginUpdate.updating') : t('settings.pluginUpdate.update')}
                        </button>
                      </AttentionSurface>
                    )}
                    {isInstallingThis && (
                      <div className="update-progress" aria-live="polite">
                        <progress value={pluginUpdate.downloadedBytes} max={pluginUpdate.totalBytes || undefined} />
                        <span>
                          {pluginUpdate.stage
                            ? t('about.downloadedPercentWithStage', {
                                percent: pluginUpdate.totalBytes
                                  ? Math.min(100, Math.round((pluginUpdate.downloadedBytes / pluginUpdate.totalBytes) * 100))
                                  : 0,
                                stage: t(`settings.pluginUpdate.stage.${pluginUpdate.stage}`),
                              })
                            : pluginUpdate.totalBytes
                              ? t('about.downloadedPercent', { percent: Math.min(100, Math.round((pluginUpdate.downloadedBytes / pluginUpdate.totalBytes) * 100)) })
                              : t('about.downloadedMib', { mib: (pluginUpdate.downloadedBytes / 1024 / 1024).toFixed(1) })}
                        </span>
                      </div>
                    )}
                  </AttentionSurface>
                );
              })}
            </div>
          )}
          {pluginUpdate.phase === 'error' && pluginUpdate.error && (
            <p className="setting-hint update-error">{pluginUpdate.error}</p>
          )}
        </section>
        </>
      )}

      {displayedTab === 'privacy' && (
        <section className="card settings-section">
          <div className="card-title"><h2>{t('settings.section.privacy')}</h2></div>
          <SettingRow title={t('settings.privacy.telemetryLabel')} hint={t('settings.privacy.telemetryHint')}>
            <Toggle checked={true} onChange={() => {}} label={t('settings.privacy.telemetryLabel')} disabled showOnWhenDisabled />
          </SettingRow>
          <SettingRow title={t('settings.privacy.scanLabel')} hint={t('settings.privacy.scanHint')}>
            <MiraActivityButton
              className="secondary"
              active={deviceScanBusy}
              activity="scanning-devices"
              onClick={() => void scanDevices()}
              disabled={previewMode}
            >
              {t('settings.privacy.scanButton')}
            </MiraActivityButton>
          </SettingRow>
          <SettingRow title={t('settings.privacy.diagnosticsLabel')} hint={t('settings.privacy.diagnosticsHint')}>
            <MiraActivityButton
              className="secondary"
              active={diagnosticsExportBusy}
              activity="exporting-diagnostics"
              onClick={() => void handleExportDiagnostics()}
              disabled={previewMode}
            >
              {t('settings.privacy.diagnosticsButton')}
            </MiraActivityButton>
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

      {displayedTab === 'about' && (
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
              </div>
              <button className="primary" onClick={onNavigateLogs}>{t('logs.openButton')}</button>
            </div>
          </section>

          <section className="card settings-section donate-card">
            <span className="donate-border-beam" aria-hidden="true" />
            <div className="card-title"><h2>{t('about.section.donate')}</h2></div>
            <p className="setting-hint donate-hint">{t('about.donate.hint')}</p>
            <div className="contact-links">
              <ExternalLink className="primary" href="https://hey.run/donate/" errorTitle={t('notification.openExternalFailed')}>{t('about.donate.button')}</ExternalLink>
            </div>
          </section>
        </>
      )}
      </div>
      </div>
    </main>
  );
}
