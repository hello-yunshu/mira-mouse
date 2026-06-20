// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  BatteryHigh,
  Gauge,
  Info,
  Lightbulb,
  Minus,
  ReadCvLogo,
  SlidersHorizontal,
  Square,
  UserCircle,
  WaveSine,
  X,
} from '@phosphor-icons/react';
import { MOCK_DEVICE } from './mock';
import { applyTheme } from './theme';
import { SettingsPage } from './Settings';
import { AboutPage } from './About';
import type { AppSettings, DeviceBattery, DeviceCapabilities, DeviceSnapshot, DeviceState, ThemeMode } from './types';
import './styles.css';

type View = 'dashboard' | 'settings' | 'about';
type ControlMode = 'dpi' | 'polling' | 'lighting';

function isWindowsPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'windows' || navigator.userAgent.includes('Windows');
}

function isWindowsWebPreview(): boolean {
  return new URLSearchParams(window.location.search).get('platform') === 'windows'
    && !navigator.userAgent.includes('Windows');
}

function WindowsPreviewControls() {
  return (
    <div className="windows-preview-controls" aria-label="Windows 窗口控件">
      <button type="button" aria-label="最小化窗口"><Minus weight="regular" /></button>
      <button type="button" aria-label="最大化窗口"><Square weight="regular" /></button>
      <button type="button" className="windows-close" aria-label="关闭窗口"><X weight="regular" /></button>
    </div>
  );
}

const CONNECTION_LABEL = {
  usb: 'USB',
  wireless: '无线',
  bluetooth: '蓝牙',
  virtual: '虚拟',
} as const satisfies Record<DeviceSnapshot['connection'], DeviceState['connection']>;

// 界面不硬编码品牌灯效名称。灯效名称由插件 parsers.json 的 derived.lookup 提供（effectName/optionName）。
// 当插件未提供名称时，界面只显示通用占位符，避免将品牌数据耦合到 UI 层。
function lightingCapability(capabilities: DeviceCapabilities | undefined, group: 'mouseEffect' | 'receiverLighting'): Record<string, unknown> | undefined {
  return capabilities?.[group] ?? (group === 'mouseEffect' ? capabilities?.lighting : undefined);
}

function getLightingEffectName(capabilities?: DeviceCapabilities, group: 'mouseEffect' | 'receiverLighting' = 'mouseEffect'): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return '硬件同步';
  // 仅使用插件提供的 effectName（来自 parsers.json derived.lookup）
  if (typeof lighting.effectName === 'string' && lighting.effectName) return lighting.effectName;
  const effect = lighting.effect;
  if (typeof effect !== 'number') return '硬件同步';
  if (effect === 0) return '已关闭';
  return `灯效 ${effect}`;
}

function getLightingColorMode(capabilities?: DeviceCapabilities, group: 'mouseEffect' | 'receiverLighting' = 'mouseEffect'): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return '未报告';
  // 仅使用插件提供的 optionName（来自 parsers.json derived.lookup）
  if (typeof lighting.optionName === 'string' && lighting.optionName) return lighting.optionName;
  const option = lighting.option;
  if (typeof option !== 'number') return '未报告';
  return `模式 ${option}`;
}

function rgbToHex(rgb: unknown): string | undefined {
  if (typeof rgb === 'string' && /^#[0-9a-f]{6}$/i.test(rgb)) return rgb;
  if (!Array.isArray(rgb) || rgb.length < 3) return undefined;
  const [r, g, b] = rgb.map((v) => Number(v));
  if ([r, g, b].some((v) => Number.isNaN(v))) return undefined;
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

function snapshotToState(snapshot: DeviceSnapshot): DeviceState {
  const now = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  const stages = snapshot.dpiStages?.length
    ? snapshot.dpiStages
    : snapshot.dpi
      ? [{ value: snapshot.dpi, color: '#9a8bd0', enabled: true, active: true }]
      : [{ value: 800, color: '#9a8bd0', enabled: true, active: true }];
  const caps = snapshot.capabilities ?? {};
  const mouseEffect = lightingCapability(caps, 'mouseEffect');
  const receiverLighting = lightingCapability(caps, 'receiverLighting');
  const settings = caps.settings;
  const mouseLightEnabled = typeof settings?.mouseLightEnabled === 'boolean'
    ? settings.mouseLightEnabled
    : typeof mouseEffect?.enabled === 'boolean' ? mouseEffect.enabled : undefined;
  const mouseLightColor = rgbToHex(settings?.mouseLightStartColor)
    ?? (typeof mouseEffect?.color === 'string' ? mouseEffect.color : snapshot.confirmedLightColor);
  const mouseLightEndColor = rgbToHex(settings?.mouseLightEndColor);
  const fallbackBatteries: DeviceBattery[] = snapshot.batteryPercent === undefined ? [] : [{
    id: 'mouse', label: '鼠标', percentage: snapshot.batteryPercent, charging: snapshot.charging,
  }];
  return {
    name: snapshot.displayName,
    connection: CONNECTION_LABEL[snapshot.connection],
    battery: snapshot.batteryPercent,
    charging: snapshot.charging,
    batteries: snapshot.batteries?.length ? snapshot.batteries : fallbackBatteries,
    pollingRate: snapshot.pollingRateHz,
    profile: snapshot.profile,
    evidence: snapshot.evidence,
    updatedAt: now,
    dpiStages: stages,
    lighting: mouseLightColor || mouseEffect || receiverLighting
      ? {
          enabled: mouseLightEnabled !== false,
          mode: mouseEffect ? getLightingEffectName(caps, 'mouseEffect') : mouseLightEnabled === false ? '已关闭' : '已开启',
          color: mouseLightColor,
          supportsSpeed: typeof mouseEffect?.speed === 'number',
          supportsBrightness: typeof mouseEffect?.brightness === 'number',
          receiverLinked: snapshot.connection === 'wireless',
          mouseLightEnabled,
          mouseLightColor,
          mouseLightEndColor,
          receiverLightEnabled: typeof receiverLighting?.enabled === 'boolean' ? receiverLighting.enabled : undefined,
          receiverLightMode: receiverLighting ? getLightingEffectName(caps, 'receiverLighting') : undefined,
          receiverLightColor: typeof receiverLighting?.color === 'string' ? receiverLighting.color : undefined,
        }
      : undefined,
    capabilities: caps,
  };
}

function DeviceAura({ color }: { color?: string }) {
  return (
    <div className="device-aura" data-animation="realtime-deformation" style={{ '--device-color': color ?? '#b87ab0' } as React.CSSProperties} aria-hidden="true">
      <div className="aura-cloud aura-cloud-1" />
      <div className="aura-cloud aura-cloud-2" />
      <div className="aura-cloud aura-cloud-3" />
      <div className="aura-cloud aura-cloud-4" />
      <div className="aura-cloud aura-cloud-5" />
      <div className="aura-star aura-star-1" />
      <div className="aura-star aura-star-2" />
      <div className="aura-star aura-star-3" />
      <div className="aura-star aura-star-4" />
      <div className="aura-star aura-star-5" />
      <div className="aura-star aura-star-6" />
    </div>
  );
}

function EmptyState({ onRefresh, onDemo, onOpenSettings }: { onRefresh: () => void; onDemo: () => void; onOpenSettings: () => void }) {
  return (
    <main className="empty">
      <DeviceAura />
      <p className="eyebrow">Mira Mouse</p>
      <h1>未发现受支持的鼠标</h1>
      <p>连接设备后，Mira 会安静地在这里显示可用设置。未知设备不会被猜测识别。</p>
      <div className="actions">
        <button onClick={onRefresh}>刷新</button>
        <button className="secondary" onClick={onOpenSettings}>设备与诊断</button>
      </div>
      <button className="demo" onClick={onDemo}>打开 Fixture 演示</button>
    </main>
  );
}

const CAPABILITY_GROUP_LABELS: Record<string, string> = {
  battery: '电池',
  dpi: 'DPI 档位',
  settings: '传感器与连接',
  lighting: '主灯光（旧插件）',
  mouseEffect: '鼠标灯效',
  receiverLighting: '接收器灯光',
  fps: '传感器帧率',
  dpiButton: 'DPI 快切',
  firmwareUsb: '鼠标 USB 固件',
  firmwareSoc: '鼠标主控固件',
  receiverFirmwareUsb: '接收器 USB 固件',
  receiverFirmwareSoc: '接收器主控固件',
  receiverFirmwareLed: '接收器灯光固件',
  buttonMappings: '按键映射',
};

const CAPABILITY_FIELD_LABELS: Record<string, string> = {
  percentage: '电量', charging: '充电中', valid: '数据有效', profile: '配置编号', currentStage: '当前档位', stageCount: '档位数量',
  dpiX: 'X 轴 DPI', dpiY: 'Y 轴 DPI', stageColors: '档位颜色', pollingRaw: '回报率原始值', pollingRate: '回报率',
  usbDebounce: 'USB 防抖', wirelessDebounce: '2.4G 防抖', bluetoothDebounce: '蓝牙防抖', rippleCorrection: '波纹修正',
  buttonChangeTime: '按键切换时间', wheelToButton: '滚轮转按键', buttonToWheel: '按键转滚轮', bluetoothSleepValue: '蓝牙休眠值',
  wirelessSleepValue: '2.4G 休眠值', liftCutOff: '抬升高度', angleSnap: '角度吸附', motionSync: '运动同步',
  mouseLightStartColor: '鼠标灯光颜色', mouseLightEndColor: '鼠标灯光结束色', mouseLightEnabled: '鼠标灯光启用', effect: '灯效', speed: '速度',
  brightness: '亮度', option: '颜色模式', color: '灯光颜色', enabled: '启用', versionRaw: '固件原始版本值',
  effectName: '灯效名称', optionName: '颜色模式名称', speedLabel: '速度等级', brightnessLabel: '亮度等级',
};

function capabilityValue(value: unknown, key: string): string {
  if (typeof value === 'boolean') return value ? '开启' : '关闭';
  if (typeof value === 'number') {
    // 灯效/颜色模式的友好名称由插件 derived.lookup 提供（effectName/optionName 字段），
    // 此处仅显示原始数值，避免在界面硬编码品牌映射表。
    if (key === 'percentage' || key === 'brightness') return `${value}%`;
    if (key === 'pollingRate') return `${value} Hz`;
    return String(value);
  }
  if (Array.isArray(value)) {
    if (key.startsWith('0x') && value.every((item) => typeof item === 'number')) {
      return value.map((item) => Number(item).toString(16).toUpperCase().padStart(2, '0')).join(' ');
    }
    return value.join(' · ');
  }
  if (value === null || value === undefined || value === '') return '未报告';
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

function readCapability(capabilities: DeviceCapabilities, group: string, field: string): unknown {
  return capabilities[group]?.[field];
}

function preferredCapability(capabilities: DeviceCapabilities, group: string, preferred: string, fallback: string): string {
  const preferredValue = readCapability(capabilities, group, preferred);
  return preferredValue === undefined
    ? capabilityValue(readCapability(capabilities, group, fallback), fallback)
    : capabilityValue(preferredValue, preferred);
}

function DeviceDetails({ capabilities, onClose }: { capabilities: DeviceCapabilities; onClose: () => void }) {
  const groups = Object.entries(capabilities).filter(([, fields]) => fields && Object.keys(fields).length > 0);
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose(); };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [onClose]);
  return (
    <div className="details-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="device-details" role="dialog" aria-modal="true" aria-labelledby="device-details-title">
        <header>
          <div><p className="eyebrow">只读设备报告</p><h2 id="device-details-title">全部读取信息</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="关闭设备详情"><X weight="regular" /></button>
        </header>
        <p className="details-note">以下字段由已签名设备插件直接读取。当前插件未开放写入，因此不会显示虚假的可编辑控件。</p>
        <div className="capability-groups">
          {groups.length ? groups.map(([group, fields]) => (
            <section className="capability-group" key={group}>
              <h3>{CAPABILITY_GROUP_LABELS[group] ?? group}</h3>
              <dl>
                {Object.entries(fields).map(([key, value]) => (
                  <div key={key}>
                    <dt>{CAPABILITY_FIELD_LABELS[key] ?? key}</dt>
                    <dd>{capabilityValue(value, key)}</dd>
                  </div>
                ))}
              </dl>
            </section>
          )) : <p className="setting-hint">设备没有报告扩展能力字段。</p>}
        </div>
      </section>
    </div>
  );
}

function Dashboard({ device }: { device: DeviceState }) {
  const stages = device.dpiStages.filter((stage) => stage.enabled);
  const current = stages.find((stage) => stage.active);
  const initialDpi = current?.value ?? stages[0]?.value ?? 0;
  const [mode, setMode] = useState<ControlMode>('dpi');
  const [lightingView, setLightingView] = useState<'mouse' | 'receiver'>('mouse');
  const [previewMessage, setPreviewMessage] = useState('');
  const [showDetails, setShowDetails] = useState(false);
  const [showBatteries, setShowBatteries] = useState(false);
  const activeDpi = initialDpi;

  const controls: { id: ControlMode; label: string; icon: typeof Gauge }[] = [
    { id: 'dpi', label: 'DPI', icon: Gauge },
    { id: 'polling', label: '回报率', icon: WaveSine },
    { id: 'lighting', label: '灯光', icon: Lightbulb },
  ];

  return (
    <main className="dashboard">
      <section className="device-hero" aria-label="已连接设备">
        <div className="device-column">
          <h2 className="app-title">Mira</h2>
          <div className="device-copy">
            <p className="connection-state"><span />{device.connection} · 已连接</p>
            <h1>{device.name}</h1>
            {device.batteries.length > 0 && (
            <div className={`battery-control ${showBatteries ? 'open' : ''}`}>
              <button
                className="battery-state"
                aria-expanded={showBatteries}
                aria-controls="device-batteries"
                onClick={() => setShowBatteries((visible) => !visible)}
              >
                <BatteryHigh weight="regular" />
                {device.batteries[0].percentage}%
                {device.batteries[0].charging ? ' · 充电中' : ''}
                <span className="battery-count">{device.batteries.length} 台设备</span>
              </button>
              <section id="device-batteries" className="battery-popover" aria-label="设备电量">
                <p>设备电量</p>
                {device.batteries.map((battery) => (
                  <div key={battery.id} className="battery-device">
                    <span><BatteryHigh weight="regular" />{battery.label}</span>
                    <strong>{battery.percentage}%{battery.charging ? ' · 充电中' : ''}</strong>
                  </div>
                ))}
              </section>
            </div>
            )}
          </div>
        </div>
        <DeviceAura color={device.lighting?.mouseLightColor ?? device.lighting?.color} />
      </section>

      <div className="control-tabs" role="tablist" aria-label="设备控制">
        {controls.map(({ id, label, icon: ControlIcon }) => (
          <button
            key={id}
            role="tab"
            aria-selected={mode === id}
            className={mode === id ? 'active' : ''}
            onClick={() => { setMode(id); setPreviewMessage(''); }}
          >
            <ControlIcon weight="regular" />
            <span>{label}</span>
          </button>
        ))}
      </div>

      <section className="control-stage" aria-live="polite">
        {mode === 'dpi' && (
          <div className="control-reading dpi-reading">
            <div className="primary-reading" aria-label={`当前 DPI：${activeDpi}`}>
              <strong>{activeDpi}</strong><em>DPI</em>
            </div>
            <div className="dpi-scale" aria-label="DPI 档位" style={{ '--stage-count': Math.max(stages.length, 1) } as React.CSSProperties}>
              {stages.map((stage) => (
                <button
                  key={stage.value}
                  className={stage.active ? 'active' : ''}
                  aria-pressed={stage.active}
                  onClick={() => setPreviewMessage(`档位 ${stage.value} DPI；当前插件只读，硬件当前值仍为 ${activeDpi} DPI。`)}
                >
                  <i style={{ '--stage-source-color': stage.color } as React.CSSProperties} />
                  <span>{stage.value}</span>
                </button>
              ))}
            </div>
            <button className="adjust-button" onClick={() => setPreviewMessage('当前为只读预览，硬件写入尚未开放。')}>
              <SlidersHorizontal weight="regular" />调整当前档
            </button>
          </div>
        )}

        {mode === 'polling' && (
          <div className="control-reading mode-reading">
            <WaveSine weight="regular" />
            <span>当前回报率</span>
            <strong>{device.pollingRate ? `${device.pollingRate} Hz` : '未报告'}</strong>
            <div className="capability-summary" aria-label="传感器设置摘要">
              <span>运动同步<strong>{capabilityValue(readCapability(device.capabilities, 'settings', 'motionSync'), 'motionSync')}</strong></span>
              <span>角度吸附<strong>{capabilityValue(readCapability(device.capabilities, 'settings', 'angleSnap'), 'angleSnap')}</strong></span>
              <span>抬升高度<strong>{capabilityValue(readCapability(device.capabilities, 'settings', 'liftCutOff'), 'liftCutOff')}</strong></span>
            </div>
            <p>显示设备当前报告的状态；当前插件为只读。</p>
          </div>
        )}

        {mode === 'lighting' && (
          <div className="control-reading mode-reading lighting-reading">
            <div className="lighting-sub-tabs" role="tablist" aria-label="灯光对象">
              <button
                role="tab"
                aria-selected={lightingView === 'mouse'}
                className={lightingView === 'mouse' ? 'active' : ''}
                onClick={() => setLightingView('mouse')}
              >鼠标灯光</button>
              <button
                role="tab"
                aria-selected={lightingView === 'receiver'}
                className={lightingView === 'receiver' ? 'active' : ''}
                onClick={() => setLightingView('receiver')}
              >接收器灯光</button>
            </div>
            <div className="lighting-swatch" style={{ '--light-color': (
              lightingView === 'mouse' ? device.lighting?.mouseLightColor : device.lighting?.receiverLightColor
            ) ?? '#b87ab0' } as React.CSSProperties} />
            <div className="lighting-sections" aria-label="灯光分组">
              {lightingView === 'mouse' && (
                <div className="lighting-group lighting-group-mouse">
                  <p className="lighting-group-title">鼠标灯光</p>
                  <div className="capability-summary">
                    <span>状态<strong>{device.lighting?.mouseLightEnabled === false ? '关闭' : '开启'}</strong></span>
                    <span>颜色<strong>{device.lighting?.mouseLightColor ?? '未报告'}</strong></span>
                    {device.lighting?.mouseLightEndColor && device.lighting.mouseLightEndColor !== device.lighting.mouseLightColor && (
                      <span>结束色<strong>{device.lighting.mouseLightEndColor}</strong></span>
                    )}
                  </div>
                </div>
              )}
              {lightingView === 'receiver' && (
                <div className="lighting-group lighting-group-dongle">
                  <p className="lighting-group-title">接收器灯光</p>
                  <div className="capability-summary">
                    <span>灯效<strong>{device.lighting?.receiverLightMode ?? '未报告'}</strong></span>
                    <span>颜色<strong>{getLightingColorMode(device.capabilities, 'receiverLighting')}</strong></span>
                    <span>速度<strong>{preferredCapability(device.capabilities, 'receiverLighting', 'speedLabel', 'speed')}</strong></span>
                    <span>亮度<strong>{preferredCapability(device.capabilities, 'receiverLighting', 'brightnessLabel', 'brightness')}</strong></span>
                  </div>
                </div>
              )}
            </div>
            {device.lighting?.receiverLinked && <p>鼠标与接收器灯光分别读取，互不混用。</p>}
          </div>
        )}
        {previewMessage && <p className="preview-message">{previewMessage}</p>}
      </section>

      <section className="status-strip" aria-label="设备状态">
        <div><WaveSine weight="regular" /><span>回报率<strong>{device.pollingRate ? `${device.pollingRate} Hz` : '未报告'}</strong></span></div>
        <div><UserCircle weight="regular" /><span>配置文件<strong>{device.profile ?? '未报告'}</strong></span></div>
        <div><Lightbulb weight="regular" /><span>鼠标灯光<strong>{device.lighting?.mouseLightEnabled === false ? '已关闭' : device.lighting?.mouseLightColor ?? '未报告'}</strong></span><i style={{ '--light-color': device.lighting?.mouseLightColor ?? '#b87ab0' } as React.CSSProperties} /></div>
      </section>
      <div className="dashboard-meta">
        <span>最后更新：{device.updatedAt}</span>
        <button className="details-button" onClick={() => setShowDetails(true)}><ReadCvLogo weight="regular" />全部读取信息</button>
      </div>
      {showDetails && <DeviceDetails capabilities={device.capabilities} onClose={() => setShowDetails(false)} />}
    </main>
  );
}

export default function App() {
  const [device, setDevice] = useState<DeviceState>();
  const [theme, setTheme] = useState<ThemeMode>('system');
  const [view, setView] = useState<View>('dashboard');
  const [demoMode, setDemoMode] = useState(false);
  const [, setRefreshIntervalSeconds] = useState(5);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const windowsPlatform = isWindowsPlatform();
  const windowsWebPreview = isWindowsWebPreview();

  // 从后端加载已保存的主题设置
  useEffect(() => {
    invoke<AppSettings>('settings_get')
      .then((settings) => {
        setTheme(settings.theme as ThemeMode);
        setRefreshIntervalSeconds(Math.min(60, Math.max(1, settings.refreshIntervalSeconds || 5)));
      })
      .catch(() => {});
  }, []);

  // 周期性从后端读取真实设备状态
  useEffect(() => {
    if (demoMode) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    // 启动时立即读取一次缓存
    invoke<DeviceSnapshot | null>('device_snapshot')
      .then((snapshot) => {
        if (!cancelled) {
          setDevice(snapshot ? snapshotToState(snapshot) : undefined);
        }
      })
      .catch(() => {
        if (!cancelled) setDevice(undefined);
      });

    // 监听后台线程发出的 device-updated 事件，无需轮询
    listen<DeviceSnapshot | null>('device-updated', (event) => {
      if (cancelled) return;
      const snapshot = event.payload;
      setDevice(snapshot ? snapshotToState(snapshot) : undefined);
    }).then((un) => {
      unlisten = un;
    }).catch(() => {});

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [demoMode, refreshNonce]);

  useEffect(() => applyTheme(theme, device?.lighting?.mouseLightColor ?? device?.lighting?.color), [theme, device]);

  return <div className={`app-shell ${windowsPlatform ? 'platform-windows' : ''} ${windowsWebPreview ? 'windows-web-preview' : ''}`}>
    <nav className="top-nav" data-tauri-drag-region>
      <div className="nav-links">
        <button className={`nav-link ${view === 'dashboard' ? 'active' : ''}`} onClick={() => setView('dashboard')}>设备</button>
        <button className={`nav-link ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')}>设置</button>
        <button className={`nav-link nav-about ${view === 'about' ? 'active' : ''}`} onClick={() => setView('about')} aria-label="关于 Mira"><Info weight="regular" /></button>
      </div>
      {windowsWebPreview && <WindowsPreviewControls />}
    </nav>
    {view === 'dashboard' && (device ? <Dashboard device={device} /> : <EmptyState onRefresh={() => { setDemoMode(false); setDevice(undefined); setRefreshNonce((value) => value + 1); }} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); }} onOpenSettings={() => setView('settings')} />)}
    {view === 'settings' && <SettingsPage onNavigateAbout={() => setView('about')} onThemeChange={setTheme} onRefreshIntervalChange={setRefreshIntervalSeconds} />}
    {view === 'about' && <AboutPage onBack={() => setView('settings')} />}
  </div>;
}
