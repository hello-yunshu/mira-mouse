// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { check } from '@tauri-apps/plugin-updater';
import type { AboutInfo } from './types';
import { notifyError } from './notify';

type UpdateState = 'idle' | 'checking' | 'up-to-date' | 'available';

const PREVIEW_INFO: AboutInfo = {
  name: 'Mira Mouse',
  version: '0.1.0-preview',
  identifier: 'app.mira.preview',
  platform: 'Web Preview',
  architecture: 'browser',
  rustVersion: 'N/A',
  buildDate: '本地预览',
  gitCommit: 'working-tree',
  bundledPlugins: [],
  contact: { github: 'https://github.com/hello-yunshu/mira-mouse' },
  updaterActive: false,
};

function extractChannel(releaseTag: string): string | null {
  const match = releaseTag.match(/-(test|beta|alpha|rc|dev|preview|canary)$/i);
  return match ? match[1].toLowerCase() : null;
}

export function AboutPage({ onBack, previewMode = false }: { onBack: () => void; previewMode?: boolean }) {
  const [info, setInfo] = useState<AboutInfo | null>(previewMode ? PREVIEW_INFO : null);
  const [error, setError] = useState<string>('');
  const [updateState, setUpdateState] = useState<UpdateState>('idle');
  const [diagnostics, setDiagnostics] = useState<string>('');

  useEffect(() => {
    if (previewMode) return;
    invoke<AboutInfo>('about_info')
      .then(setInfo)
      .catch((err) => setError(String(err)));
  }, [previewMode]);

  async function checkForUpdates() {
    if (!info?.updaterActive) return;
    setUpdateState('checking');
    try {
      const update = await check();
      if (update) {
        setUpdateState('available');
      } else {
        setUpdateState('up-to-date');
      }
    } catch (err) {
      setUpdateState('idle');
      notifyError('检查更新失败', String(err));
    }
  }

  function exportDiagnostics() {
    if (previewMode) {
      setDiagnostics(JSON.stringify({ mode: 'web-preview', privacy: 'sanitized' }, null, 2));
      return;
    }
    invoke<unknown>('export_diagnostics')
      .then((data) => setDiagnostics(JSON.stringify(data, null, 2)))
      .catch((err) => notifyError('导出失败', String(err)));
  }

  if (error) {
    return (
      <main className="about-page">
        <header>
          <button className="secondary" onClick={onBack}>返回</button>
        </header>
        <p className="setting-hint">加载关于信息失败：{error}</p>
      </main>
    );
  }

  if (!info) {
    return (
      <main className="about-page">
        <header>
          <button className="secondary" onClick={onBack}>返回</button>
        </header>
        <p className="setting-hint">加载中…</p>
      </main>
    );
  }

  const contact = info.contact;

  return (
    <main className="about-page">
      <header>
        <div>
          <p className="eyebrow">Mira Mouse</p>
          <h1>关于</h1>
        </div>
        <button className="secondary" onClick={onBack}>返回</button>
      </header>

      <section className="card about-section">
        <img className="about-logo" src="/app-icon.png" alt="" aria-hidden="true" />
        <h2>{info.name}</h2>
        <p className="eyebrow">非官方 · 兼容客户端</p>
        <p className="disclaimer">
          Mira 与任何厂商之间不存在授权、合作或背书关系——要对自己诚实嘛。
          未经真机验证的能力不会在界面中宣称稳定支持。
        </p>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>版本</h2></div>
        <dl className="info-list">
          <div><dt>应用名称</dt><dd>{info.name}</dd></div>
          <div><dt>版本</dt><dd>{info.version}</dd></div>
          <div><dt>构建日期</dt><dd>{info.buildDate}</dd></div>
          <div><dt>Git Commit</dt><dd><code>{info.gitCommit}</code></dd></div>
          <div><dt>Bundle Identifier</dt><dd>{info.identifier}</dd></div>
          <div><dt>平台</dt><dd>{info.platform}</dd></div>
          <div><dt>架构</dt><dd>{info.architecture}</dd></div>
          <div><dt>Rust 版本</dt><dd>{info.rustVersion}</dd></div>
          <div><dt>自动更新</dt><dd>{info.updaterActive ? '已启用' : '未启用'}</dd></div>
        </dl>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>内置插件</h2></div>
        {info.bundledPlugins.length === 0 ? (
          <p className="setting-hint">未发现内置插件。正式安装包默认携带 mira.amaster。</p>
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
                      {plugin.signatureVerified ? '签名已验证' : '签名未验证'}
                    </span>
                    {plugin.bundleByDefault && <span className="badge">默认内置</span>}
                  </div>
                  <dl className="plugin-detail">
                    <div><dt>SHA-256</dt><dd><code>{plugin.sha256}</code></dd></div>
                    <div><dt>发布者 Key ID</dt><dd><code>{plugin.publisherKeyId}</code></dd></div>
                    <div><dt>资产名</dt><dd>{plugin.asset}</dd></div>
                  </dl>
                </div>
              );
            })}
          </div>
        )}
      </section>

      {contact.github || contact.x || contact.telegram ? (
        <section className="card about-section">
          <div className="card-title"><h2>联系</h2></div>
          {contact.developerName && <p className="setting-hint">开发者：{contact.developerName}</p>}
          {contact.copyright && <p className="setting-hint">版权：{contact.copyright}</p>}
          <div className="contact-links">
            {contact.github && <a className="secondary" href={contact.github} target="_blank" rel="noopener noreferrer">GitHub</a>}
            {contact.x && <a className="secondary" href={contact.x} target="_blank" rel="noopener noreferrer">X</a>}
            {contact.telegram && <a className="secondary" href={contact.telegram} target="_blank" rel="noopener noreferrer">Telegram</a>}
          </div>
        </section>
      ) : null}

      <section className="card about-section">
        <div className="card-title"><h2>检查更新</h2></div>
        <p className="setting-hint">
          {info.updaterActive
            ? '自动更新已启用，可手动检查新版本。'
            : '自动更新未启用。请前往 GitHub Release 页面手动下载新版本。'}
        </p>
        {info.updaterActive && (
          <>
            <div className="contact-links">
              <button className="secondary" onClick={checkForUpdates} disabled={updateState === 'checking'}>
                {updateState === 'checking' ? '检查中…' : '检查更新'}
              </button>
            </div>
            {updateState === 'up-to-date' && <p className="setting-hint">当前已是最新版本。</p>}
            {updateState === 'available' && <p className="setting-hint">发现新版本，将在下次启动时提示安装。</p>}
          </>
        )}
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>开源许可</h2></div>
        <p className="setting-hint">
          Mira 使用 AGPL-3.0-or-later 许可证，原创文档使用 CC-BY-SA-4.0。第三方依赖保留各自原许可证。
        </p>
        <div className="contact-links">
          {contact.github ? (
            <a className="secondary" href={`${contact.github}/blob/main/LICENSE`} target="_blank" rel="noopener noreferrer">查看开源许可证</a>
          ) : (
            <button className="secondary" disabled>查看开源许可证（需配置 GitHub 链接）</button>
          )}
          {contact.github ? (
            <a className="secondary" href={`${contact.github}/tree/main/NOTICE`} target="_blank" rel="noopener noreferrer">查看第三方许可</a>
          ) : (
            <button className="secondary" disabled>查看第三方许可（需配置 GitHub 链接）</button>
          )}
        </div>
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>隐私说明</h2></div>
        <p className="setting-hint">
          Mira 不内置遥测、账户、广告或常驻网络服务。诊断导出已脱敏，不含设备序列号或 HID 负载。
          未经你确认不上传任何数据。
        </p>
        <div className="contact-links">
          <button className="secondary" onClick={exportDiagnostics}>导出诊断</button>
        </div>
        {diagnostics && (
          <pre className="diagnostics-output">{diagnostics}</pre>
        )}
      </section>

      <section className="card about-section">
        <div className="card-title"><h2>报告问题</h2></div>
        <p className="setting-hint">
          报告问题时请勿粘贴未脱敏的序列号或敏感日志。
        </p>
        <div className="contact-links">
          {contact.github ? (
            <a className="secondary" href={`${contact.github}/issues/new/choose`} target="_blank" rel="noopener noreferrer">报告问题</a>
          ) : (
            <button className="secondary" disabled>报告问题（需配置 GitHub 链接）</button>
          )}
        </div>
      </section>
    </main>
  );
}
