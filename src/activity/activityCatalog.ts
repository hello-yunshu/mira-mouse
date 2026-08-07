// SPDX-License-Identifier: AGPL-3.0-or-later

export type MiraActivityKind =
  | 'awaiting-mouse'
  | 'device-initializing'
  | 'battery-analysis'
  | 'applying-settings'
  | 'scanning-devices'
  | 'checking-app-update'
  | 'checking-plugin-updates'
  | 'checking-local-ai-updates'
  | 'refreshing-device-details'
  | 'copying-readings'
  | 'copying-device-diagnostics'
  | 'exporting-battery-history'
  | 'exporting-device-config'
  | 'importing-device-config'
  | 'exporting-diagnostics'
  | 'restoring-local-ai';

export type MiraOrbState =
  | 'working'
  | 'searching'
  | 'solving'
  | 'connecting'
  | 'weaving'
  | 'composing';

export interface MiraActivitySpec {
  state: MiraOrbState;
  size: 20 | 64;
  speed: number;
  layer: 'global' | 'inline';
}

const ACTIVITY_SPECS: Record<MiraActivityKind, MiraActivitySpec> = {
  'awaiting-mouse': { state: 'connecting', size: 64, speed: 1, layer: 'global' },
  'device-initializing': { state: 'connecting', size: 64, speed: 1, layer: 'global' },
  'battery-analysis': { state: 'solving', size: 64, speed: 0.9, layer: 'global' },
  'applying-settings': { state: 'working', size: 20, speed: 1, layer: 'inline' },
  'scanning-devices': { state: 'searching', size: 20, speed: 0.95, layer: 'inline' },
  'checking-app-update': { state: 'searching', size: 20, speed: 0.95, layer: 'inline' },
  'checking-plugin-updates': { state: 'searching', size: 20, speed: 0.95, layer: 'inline' },
  'checking-local-ai-updates': { state: 'searching', size: 20, speed: 0.95, layer: 'inline' },
  'refreshing-device-details': { state: 'weaving', size: 20, speed: 0.9, layer: 'inline' },
  'copying-readings': { state: 'composing', size: 20, speed: 0.9, layer: 'inline' },
  'copying-device-diagnostics': { state: 'composing', size: 20, speed: 0.9, layer: 'inline' },
  'exporting-battery-history': { state: 'composing', size: 20, speed: 0.9, layer: 'inline' },
  'exporting-device-config': { state: 'composing', size: 20, speed: 0.9, layer: 'inline' },
  'importing-device-config': { state: 'weaving', size: 20, speed: 0.9, layer: 'inline' },
  'exporting-diagnostics': { state: 'composing', size: 20, speed: 0.9, layer: 'inline' },
  'restoring-local-ai': { state: 'connecting', size: 20, speed: 0.9, layer: 'inline' },
};

const ZH_LABELS: Record<MiraActivityKind | 'battery-analysis-with-ai', string> = {
  'awaiting-mouse': '正在等待鼠标就位…',
  'device-initializing': '正在识别并读取鼠标…',
  'battery-analysis': '正在整理电量记录…',
  'battery-analysis-with-ai': '正在整理电量记录并生成本地分析…',
  'applying-settings': '正在应用设备设置…',
  'scanning-devices': '正在扫描设备…',
  'checking-app-update': '正在检查应用更新…',
  'checking-plugin-updates': '正在检查插件更新…',
  'checking-local-ai-updates': '正在检查本地 AI 更新…',
  'refreshing-device-details': '正在重新读取全部参数…',
  'copying-readings': '正在整理全部读数…',
  'copying-device-diagnostics': '正在整理设备诊断…',
  'exporting-battery-history': '正在导出电量历史…',
  'exporting-device-config': '正在导出设备配置…',
  'importing-device-config': '正在导入设备配置…',
  'exporting-diagnostics': '正在导出诊断信息…',
  'restoring-local-ai': '正在恢复本地 AI 组件…',
};

const EN_LABELS: Record<MiraActivityKind | 'battery-analysis-with-ai', string> = {
  'awaiting-mouse': 'Waiting for the mouse…',
  'device-initializing': 'Discovering and reading the mouse…',
  'battery-analysis': 'Preparing battery history…',
  'battery-analysis-with-ai': 'Preparing battery history and local analysis…',
  'applying-settings': 'Applying device settings…',
  'scanning-devices': 'Scanning for devices…',
  'checking-app-update': 'Checking for application updates…',
  'checking-plugin-updates': 'Checking for plugin updates…',
  'checking-local-ai-updates': 'Checking for local AI updates…',
  'refreshing-device-details': 'Refreshing all device readings…',
  'copying-readings': 'Preparing all device readings…',
  'copying-device-diagnostics': 'Preparing device diagnostics…',
  'exporting-battery-history': 'Exporting battery history…',
  'exporting-device-config': 'Exporting device configuration…',
  'importing-device-config': 'Importing device configuration…',
  'exporting-diagnostics': 'Exporting diagnostics…',
  'restoring-local-ai': 'Restoring local AI components…',
};

export const MIRA_ACTIVITY_SHOW_DELAY_MS = 300;
export const MIRA_ACTIVITY_MIN_VISIBLE_MS = 420;

export function miraActivitySpec(activity: MiraActivityKind): MiraActivitySpec {
  return ACTIVITY_SPECS[activity];
}

export function miraActivityLabel(
  activity: MiraActivityKind,
  language: string | undefined,
  options: { aiAnalysisEnabled?: boolean } = {},
): string {
  const isZh = (language ?? '').toLowerCase().startsWith('zh');
  const key = activity === 'battery-analysis' && options.aiAnalysisEnabled
    ? 'battery-analysis-with-ai'
    : activity;
  return isZh ? ZH_LABELS[key] : EN_LABELS[key];
}

/** 全局 64px Orb 的业务状态集合，由 App 显式计算并传入。 */
export type MiraGlobalActivity =
  | 'awaiting-mouse'
  | 'device-initializing'
  | 'battery-analysis'
  | null;

/**
 * 兼容兜底：从 Mira 已有的、稳定的业务 DOM 标志推导全局状态。
 * 显式业务状态为第一优先级；仅在未提供显式状态时使用本函数。
 * 电量弹窗覆盖 Dashboard，因此优先判定电量流程；弹窗出现状态条或空态后，
 * 说明业务结果已就绪，不能继续显示 Orb。
 */
export function resolveGlobalMiraActivity(
  root: ParentNode = document,
): MiraGlobalActivity {
  const batteryModal = root.querySelector<HTMLElement>('.battery-usage-modal');
  if (batteryModal) {
    const ready = batteryModal.querySelector(
      '.battery-status-strip-shell, .battery-usage-empty',
    );
    return ready ? null : 'battery-analysis';
  }

  return root.querySelector('.dashboard.is-initializing')
    ? 'device-initializing'
    : null;
}
