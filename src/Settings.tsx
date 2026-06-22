// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings, BundledPluginInfo, AboutInfo, DiscoveredDevice, ThemeMode } from './types';
import { Tooltip } from './Tooltip';
import { notifyError } from './notify';
import { extractChannel, exportDiagnostics } from './plugin-utils';

const DEFAULT_SETTINGS: AppSettings = {
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
  refreshIntervalSeconds: 5,
  telemetryDisabled: true,
};

type SettingsTab = 'general' | 'device' | 'plugins' | 'privacy' | 'about';

const TABS: { id: SettingsTab; label: string }[] = [
  { id: 'general', label: '通用' },
  { id: 'device', label: '设备' },
  { id: 'plugins', label: '插件' },
  { id: 'privacy', label: '隐私' },
  { id: 'about', label: '关于' },
];

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

export function SettingsPage({ onNavigateAbout, onThemeChange, onRefreshIntervalChange, previewMode = false }: { onNavigateAbout: () => void; onThemeChange: (theme: ThemeMode) => void; onRefreshIntervalChange: (seconds: number) => void; previewMode?: boolean }) {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [plugins, setPlugins] = useState<BundledPluginInfo[]>([]);
  const [diagnostics, setDiagnostics] = useState<string>('');
  const [discovered, setDiscovered] = useState<DiscoveredDevice[]>([]);
  const [saved, setSaved] = useState(false);
  const [tab, setTab] = useState<SettingsTab>('general');

  useEffect(() => {
    if (previewMode) return;
    invoke<AppSettings>('settings_get')
      .then((loaded) => {
        // 与默认值合并，避免后端字段缺失导致受控输入变为 undefined
        const merged: AppSettings = { ...DEFAULT_SETTINGS, ...loaded };
        setSettings(merged);
        onThemeChange(merged.theme as ThemeMode);
        onRefreshIntervalChange(merged.refreshIntervalSeconds);
      })
      .catch(() => setSettings(DEFAULT_SETTINGS));
    invoke<boolean>('autostart_state')
      .then(setAutostartEnabled)
      .catch(() => setAutostartEnabled(false));
    invoke<AboutInfo>('about_info')
      .then((info) => setPlugins(info.bundledPlugins ?? []))
      .catch(() => setPlugins([]));
  }, [onRefreshIntervalChange, onThemeChange, previewMode]);

  function update(patch: Partial<AppSettings>) {
    const next = { ...settings, ...patch };
    setSettings(next);
    if (patch.theme && onThemeChange) onThemeChange(patch.theme as ThemeMode);
    if (patch.refreshIntervalSeconds != null) onRefreshIntervalChange(patch.refreshIntervalSeconds);
    if (previewMode) {
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
      return;
    }
    invoke<AppSettings>('settings_set', { settings: next })
      .then((savedSettings) => {
        setSettings(savedSettings);
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      })
      .catch((error) => notifyError('保存失败', String(error)));
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
        notifyError('设置开机启动失败', String(error));
      });
  }

  async function handleExportDiagnostics() {
    const result = await exportDiagnostics();
    if (result !== undefined) setDiagnostics(result);
  }

  function scanDevices() {
    invoke<DiscoveredDevice[]>('discover_devices')
      .then(setDiscovered)
      .catch((err) => notifyError('扫描失败', String(err)));
  }

  return (
    <main className="settings-page">
      <header>
        <div>
          <p className="eyebrow">Mira Mouse</p>
          <h1>设置</h1>
        </div>
        {saved && <span className="save-badge">已保存</span>}
      </header>

      <nav className="sub-nav" aria-label="设置分类">
        {TABS.map((t) => (
          <button
            key={t.id}
            className={`sub-nav-link ${tab === t.id ? 'active' : ''}`}
            onClick={() => setTab(t.id)}
            aria-pressed={tab === t.id}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {tab === 'general' && (
        <>
          <section className="card settings-section">
            <div className="card-title"><h2>主题</h2></div>
            <SettingRow title="主题模式" hint="跟随系统会自动适配浅色或深色">
              <select value={settings.theme} onChange={(e) => update({ theme: e.target.value as ThemeMode })} aria-label="主题模式">
                <option value="system">跟随系统</option>
                <option value="light">浅色</option>
                <option value="dark">深色</option>
              </select>
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>开机与菜单栏</h2></div>
            <SettingRow title="开机自动启动" hint="登录系统时自动启动 Mira">
              <Toggle checked={autostartEnabled} onChange={toggleAutostart} label="开机自动启动" />
            </SettingRow>
            <SettingRow title="启动时隐藏窗口" hint="仅驻留菜单栏/托盘，不显示主窗口">
              <Toggle checked={settings.startHidden} onChange={(v) => update({ startHidden: v })} label="启动时隐藏窗口" />
            </SettingRow>
            <SettingRow title="显示电量百分比" hint="鼠标图标仍会按电量填充，这里只控制旁边的数字">
              <Toggle checked={settings.trayShowBatteryTitle} onChange={(v) => update({ trayShowBatteryTitle: v })} label="显示电量百分比" />
            </SettingRow>
            <SettingRow title="托盘图标颜色" hint="白色适合深色菜单栏背景，黑色适合浅色背景，跟随主题自动切换">
              <select value={settings.trayIconColor} onChange={(e) => update({ trayIconColor: e.target.value })} aria-label="托盘图标颜色">
                <option value="white">白色</option>
                <option value="black">黑色</option>
                <option value="auto">跟随主题</option>
              </select>
            </SettingRow>
            <SettingRow title="标题附带接收器电量" hint="托盘菜单中始终保留所有设备电量">
              <Toggle checked={settings.trayIncludeReceiverBattery} onChange={(v) => update({ trayIncludeReceiverBattery: v })} label="标题附带接收器电量" disabled={!settings.trayShowBatteryTitle} />
            </SettingRow>
            <SettingRow title="菜单显示连接状态" hint="在托盘菜单中显示连接方式和设备名称">
              <Toggle checked={settings.trayShowConnection} onChange={(v) => update({ trayShowConnection: v })} label="菜单显示连接状态" />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>轮询</h2></div>
            <SettingRow title={`刷新间隔：${settings.refreshIntervalSeconds} 秒`} hint="窗口可见时的兜底轮询间隔；事件触发时立即读取，窗口隐藏时自动降为 60 秒">
              <input
                type="range"
                min={1}
                max={60}
                value={settings.refreshIntervalSeconds}
                onChange={(e) => update({ refreshIntervalSeconds: Number(e.target.value) })}
                aria-label="刷新间隔"
              />
            </SettingRow>
          </section>
        </>
      )}

      {tab === 'device' && (
        <>
          <section className="card settings-section">
            <div className="card-title">
              <h2>电量提醒</h2>
              <Tooltip label="低电量提醒：仅在电量跨过阈值时提醒一次，不会反复弹窗。"><button className="icon-button" aria-label="低电量提醒说明">?</button></Tooltip>
            </div>
            <SettingRow title={`低电量阈值：${settings.lowBatteryThreshold}%`} hint="电量跨过阈值时通过系统通知提醒一次，充电期间不提醒">
              <input
                type="range"
                min={5}
                max={50}
                value={settings.lowBatteryThreshold}
                onChange={(e) => update({ lowBatteryThreshold: Number(e.target.value) })}
                aria-label="低电量阈值"
              />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title">
              <h2>安静灯光</h2>
              <Tooltip label="夜间模式：按时间降低或关闭灯光，并在关闭后可靠恢复原状态。"><button className="icon-button" aria-label="安静灯光说明">?</button></Tooltip>
            </div>
            <SettingRow title="启用夜间模式" hint="当前版本尚未实现灯光定时调度">
              <Toggle checked={false} onChange={() => {}} label="启用夜间模式" disabled />
            </SettingRow>
          </section>

          <section className="card settings-section">
            <div className="card-title"><h2>配置导入导出</h2></div>
            <p className="setting-hint">
              导出 .mira-profile 配置文件用于备份或迁移；导入时会检查插件和固件兼容性并展示差异。
            </p>
            <div className="contact-links">
              <button className="secondary" disabled>导出配置</button>
              <button className="secondary" disabled>导入配置</button>
            </div>
            <p className="setting-hint">配置导入导出需要连接真实设备后开放。</p>
          </section>
        </>
      )}

      {tab === 'plugins' && (
        <section className="card settings-section">
          <div className="card-title"><h2>已安装插件</h2></div>
          {plugins.length === 0 ? (
            <p className="setting-hint">未发现已安装插件。正式安装包默认携带 mira.amaster。</p>
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
                        {plugin.signatureVerified ? '签名已验证' : '签名未验证'}
                      </span>
                      {plugin.bundleByDefault && <span className="badge">默认内置</span>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      )}

      {tab === 'privacy' && (
        <section className="card settings-section">
          <div className="card-title"><h2>隐私</h2></div>
          <SettingRow title="禁用遥测" hint="Mira 不内置遥测，此选项始终开启">
            <Toggle checked={true} onChange={() => {}} label="禁用遥测" disabled />
          </SettingRow>
          <SettingRow title="扫描 HID 设备" hint="列出与已安装插件匹配的真实 HID 设备（硬件测试用）">
            <button className="secondary" onClick={scanDevices}>扫描</button>
          </SettingRow>
          <SettingRow title="导出诊断" hint="诊断数据已脱敏，不含序列号或 HID 负载">
            <button className="secondary" onClick={handleExportDiagnostics}>导出诊断</button>
          </SettingRow>
          {discovered.length > 0 && (
            <div className="plugin-list">
              {discovered.map((d) => (
                <div key={d.path} className="plugin-item">
                  <div>
                    <strong>{d.pluginId} · {d.family}</strong>
                    <span className="setting-hint">VID {d.vendorId.toString(16).toUpperCase().padStart(4, '0')} · PID {d.productId.toString(16).toUpperCase().padStart(4, '0')} · usage {d.usagePage}/{d.usage}</span>
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
        <section className="card settings-section">
          <div className="card-title"><h2>关于 Mira</h2></div>
          <SettingRow title="查看关于页" hint="版本、插件、联系方式、许可证和免责声明">
            <button className="secondary" onClick={onNavigateAbout}>打开关于页</button>
          </SettingRow>
        </section>
      )}
    </main>
  );
}
