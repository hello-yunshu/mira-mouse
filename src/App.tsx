// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import { MOCK_DEVICE } from './mock';
import { applyTheme } from './theme';
import { Tooltip } from './Tooltip';
import type { DeviceState, ThemeMode } from './types';
import './styles.css';

function Icon({ name }: { name: 'battery' | 'pointer' | 'frequency' | 'profile' | 'connection' }) {
  const paths = { battery: 'M4 8h14v8H4zM20 10v4', pointer: 'M5 3l12 10-6 1 3 6-3 1-3-6-4 4z', frequency: 'M4 12h3m3 0h4m3 0h3M8 7v10m8-13v16', profile: 'M5 5h14v14H5zM9 9h6m-6 4h6', connection: 'M8 7l4-4 4 4-4 4zm4 4v10m-4-4 4 4 4-4' };
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d={paths[name]} /></svg>;
}

function EmptyState({ onRefresh, onDemo }: { onRefresh: () => void; onDemo: () => void }) {
  return <main className="empty"><div className="orb" aria-hidden="true" /><p className="eyebrow">Mira Mouse</p><h1>未发现受支持的鼠标</h1><p>连接设备后，Mira 会安静地在这里显示可用设置。未知设备不会被猜测识别。</p><div className="actions"><button onClick={onRefresh}>刷新</button><button className="secondary">导入插件</button><button className="secondary">导出未知设备报告</button></div><button className="demo" onClick={onDemo}>打开 Fixture 演示</button></main>;
}

function Dashboard({ device }: { device: DeviceState }) {
  const stages = device.dpiStages.filter((stage) => stage.enabled);
  const current = stages.find((stage) => stage.active);
  return <main className="dashboard">
    <header><div><p className="eyebrow">{device.connection} · 已连接</p><h1>{device.name}</h1></div><span className="evidence">{device.evidence}</span></header>
    <section className="stats" aria-label="设备状态">
      {device.battery !== undefined && <div><Icon name="battery" /><strong>{device.battery}%{device.charging ? ' · 充电中' : ''}</strong><span>电量</span></div>}
      {current && <div><Icon name="pointer" /><strong>{current.value} DPI</strong><span>指针速度</span></div>}
      {device.pollingRate && <div><Icon name="frequency" /><strong>{device.pollingRate} Hz</strong><span>回报率</span></div>}
      {device.profile && <div><Icon name="profile" /><strong>{device.profile}</strong><span>配置</span></div>}
    </section>
    <section className="card"><div className="card-title"><div><p className="eyebrow">DPI 档位</p><h2>指针速度</h2></div><Tooltip label="DPI：控制指针移动速度，数值越高，移动越快。"><button className="icon-button">?</button></Tooltip></div>
      <div className="dpi-track">{stages.map((stage) => <button key={stage.value} className={stage.active ? 'active' : ''} aria-pressed={stage.active}><i style={{ background: stage.color }} /><span>{stage.value}</span></button>)}</div>
      <button className="secondary">调整当前档</button>
    </section>
    {device.lighting && <section className="card light-card"><div className="light-preview" style={{ '--light-color': device.lighting.color } as React.CSSProperties} /><div className="card-title"><div><p className="eyebrow">灯光</p><h2>{device.lighting.enabled ? device.lighting.mode : '已关闭'}</h2></div><button className="secondary">调整</button></div>
      {device.lighting.receiverLinked && <p className="hint"><Icon name="connection" /> 与鼠标联动由 Mira 连续更新鼠标和接收器，不是接收器原生功能。</p>}
    </section>}
    <footer>最后更新：{device.updatedAt} · Fixture 演示不会访问 HID 设备</footer>
  </main>;
}

export default function App() {
  const [device, setDevice] = useState<DeviceState>();
  const [theme, setTheme] = useState<ThemeMode>('system');
  useEffect(() => applyTheme(theme, device?.lighting?.enabled ? device.lighting.color : undefined), [theme, device]);
  return <div className="app-shell"><nav><span className="brand">Mira</span><label>外观<select value={theme} onChange={(event) => setTheme(event.target.value as ThemeMode)}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></label></nav>{device ? <Dashboard device={device} /> : <EmptyState onRefresh={() => setDevice(undefined)} onDemo={() => setDevice(MOCK_DEVICE)} />}</div>;
}

