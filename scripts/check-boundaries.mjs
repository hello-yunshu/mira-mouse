// SPDX-License-Identifier: AGPL-3.0-or-later
// Architecture lint: 主仓库生产代码不得包含任何当前品牌插件的协议知识。
// 依据：Mira_All_Plugins_Development_Requirements_Complete_Decoupling_Trae_Prompt_v7
// 第一章 1.1、第三章 3.1/3.2/3.3、第十一章、第十六章。
//
// 本脚本必须可重复执行，并输出 file:line:context 形式的违规清单。
// 白名单必须显式声明原因与范围，禁止宽泛目录跳过。
import { readdir, readFile, stat } from 'node:fs/promises';
import { join, relative, sep } from 'node:path';

const root = new URL('..', import.meta.url).pathname;

// 顶层忽略目录（与品牌 lint 无关的产物目录）。
const ignoredDirs = new Set([
  '.git',
  '.trae',
  'node_modules',
  'dist',
  'target',
  // 反编译证据目录只用于本地研究，本身允许出现品牌名；CI 不会看到它。
  'AMasterDriver_v1.0.6_unpacked_reverse_bundle',
  // bundled 插件包是已发布的产物，不在源码 lint 范围内。
  'src-tauri/resources/plugins',
  // vendored 第三方依赖不参与架构 lint。
  'vendor',
]);

// 允许出现品牌关键词的文件白名单。每条必须给出原因。
// 严禁用宽泛目录跳过；只能列具体文件 + 明确原因。
const allowedFiles = new Map([
  ['scripts/check-boundaries.mjs', 'lint 自身需要列出品牌关键词作为检测规则'],
  ['docs/audits/logitech-hidpp-latest-code-audit.md', '审计文档，本身记录品牌耦合'],
  ['docs/logitech-public-source-matrix.md', 'Logitech 公开源码矩阵研究文档'],
  ['docs/local-ai-analysis-plan.md', 'AI 分析计划文档，仅描述不实现'],
  ['mira-razer-comprehensive-read-support-ai-ide-prompt-v3-subagents.md', '历史 AI 提示词文档'],
  ['CHANGELOG.md', '历史变更日志，按时间记录品牌相关工作'],
  ['README.md', '项目 README 中提及支持的设备品牌属于事实陈述'],
  ['README.en.md', '项目 README 英文版'],
  ['ROADMAP.md', '路线图文档'],
  ['SUPPORT.md', '支持文档'],
  ['CONTRIBUTING.md', '贡献指南'],
]);

// 测试夹具与迁移文档允许出现品牌名，但只允许在 docs/ 与 *.md 中。
// 生产运行时代码（crates/、src-tauri/src/、src/、xtask/、scripts/）不允许。
// 但 examples/ 目录是示例文档，不是运行时生产代码。
// 前端测试文件（*.test.tsx?）和 locales（UI 文案）也不属于生产运行时逻辑。
function isProductionCode(rel) {
  const normalized = rel.split(sep).join('/');
  // examples 目录是示例，不属于运行时生产代码
  if (normalized.includes('/examples/')) return false;
  // 前端测试文件允许出现品牌名（测试夹具）
  if (/\.test\.[tc]sx?$/.test(normalized)) return false;
  // locales 目录是 UI 文案，允许出现品牌名（面向用户显示）
  if (normalized.startsWith('src/locales/')) return false;
  return (
    normalized.startsWith('crates/') ||
    normalized.startsWith('src-tauri/src/') ||
    normalized.startsWith('src/') ||
    normalized.startsWith('xtask/src/') ||
    normalized.startsWith('scripts/') ||
    normalized.startsWith('handlers/')
  );
}

// 判断一行是否是注释（Rust 的 // 或 /* */，JS/TS 的 // 或 /* */）。
// 注释中提及品牌名属于知识记录，不是运行时耦合；但仍应清理，这里先标记为非违规以便聚焦真问题。
function isCommentLine(line) {
  const trimmed = line.trim();
  return trimmed.startsWith('//') || trimmed.startsWith('/*') || trimmed.startsWith('*');
}

// Rust 测试代码检测：`#[cfg(test)]` 修饰的 mod 块内的代码视为测试代码。
// 实现策略：维护 braceDepth（相对于 test mod 起点的深度），只有当 braceDepth
// 回到 0 以下时才认为退出了 test mod。避免函数体的 } 误减 testDepth。
function makeTestModTracker() {
  let testDepth = 0;
  let braceDepth = 0;
  let pendingCfgTest = false;
  return {
    update(line) {
      if (/^\s*#\[(cfg\(test\)|test|tokio::test)\]/.test(line)) {
        pendingCfgTest = true;
      }
      const modMatch = line.match(/^\s*(?:#\[[^\]]+\]\s*)*(?:pub\s+)?mod\s+(\w+)\s*\{/);
      if (modMatch) {
        if (pendingCfgTest || /\btests?\b/.test(modMatch[1])) {
          testDepth++;
          // mod tests { 的 { 是 test mod 的开括号，braceDepth 从 1 开始
          //（1 = test mod 自身的 {）。
          braceDepth = 1;
          // 同行可能还有其他 { }，需要计入
          const opens = (line.match(/\{/g) || []).length;
          const closes = (line.match(/\}/g) || []).length;
          // 已经计入了 1 个 {（mod 的），剩余的 opens-1 个 { 加到 braceDepth
          braceDepth += (opens - 1) - closes;
          if (braceDepth <= 0) {
            // mod tests {} 单行完成（罕见），退出 test mod
            testDepth = Math.max(0, testDepth - 1);
            braceDepth = 0;
          }
          pendingCfgTest = false;
          return;
        }
        pendingCfgTest = false;
      } else {
        pendingCfgTest = false;
      }
      if (testDepth > 0) {
        const opens = (line.match(/\{/g) || []).length;
        const closes = (line.match(/\}/g) || []).length;
        braceDepth += opens - closes;
        if (braceDepth <= 0) {
          // 退出了 test mod
          testDepth = Math.max(0, testDepth - 1);
          braceDepth = 0;
        }
      }
    },
    inTest() {
      return testDepth > 0;
    },
  };
}

// 文件后缀白名单：只扫描文本源码文件。
const textExtensions = new Set([
  '.rs', '.ts', '.tsx', '.js', '.mjs', '.cjs', '.json', '.toml', '.yaml', '.yml',
  '.md', '.rst', '.txt', '.html', '.css', '.wit', '.example', '.seed',
]);

function hasTextExt(rel) {
  for (const ext of textExtensions) {
    if (rel.endsWith(ext)) return true;
  }
  return false;
}

// 品牌关键词清单（来自第十一章与第三章）。
// 注意：用整词匹配，避免误命中通用前缀。
// - amaster / AMaster / AM INFINITY MOUSE
// - am35 / AM35
// - protocol-a / Protocol A / protocol-a-receiver
// - hidpp / HID++ / HIDPP
// - logitech / Logitech
// - razer / Razer
// - mira.amaster / mira.logitech-hidpp / mira.razer-viper / mira.razer-chroma
// - 品牌 VID/PID：0x3151（AMaster VID）、0x5007（AMaster PID）、0x402B/0x402E（AMaster PID）、0x0e8d（MediaTek）
const brandPatterns = [
  { pattern: /\bAM\s*INFINITY\s*MOUSE\b/i, label: 'AMaster 产品型号' },
  { pattern: /\bamaster\b/i, label: 'amaster 品牌名' },
  { pattern: /\bAMaster\b/, label: 'AMaster 品牌名' },
  { pattern: /\bam35\b/i, label: 'AM35 型号' },
  { pattern: /\bprotocol-a\b/i, label: 'Protocol A 协议族' },
  { pattern: /\bprotocol-a-receiver\b/i, label: 'Protocol A receiver' },
  { pattern: /\bhidpp\b/i, label: 'HID++ 协议' },
  { pattern: /\bHID\+\+/, label: 'HID++ 协议' },
  { pattern: /\bHIDPP\b/, label: 'HIDPP 协议' },
  { pattern: /\blogitech\b/i, label: 'Logitech 品牌' },
  { pattern: /\brazer\b/i, label: 'Razer 品牌' },
  { pattern: /\bmira\.amaster\b/i, label: 'pluginId mira.amaster' },
  { pattern: /\bmira\.logitech-hidpp\b/i, label: 'pluginId mira.logitech-hidpp' },
  { pattern: /\bmira\.razer-viper\b/i, label: 'pluginId mira.razer-viper' },
  { pattern: /\bmira\.razer-chroma\b/i, label: 'pluginId mira.razer-chroma' },
  { pattern: /0x3151/i, label: 'AMaster VID 0x3151' },
  { pattern: /0x5007/i, label: 'AMaster PID 0x5007' },
  { pattern: /0x402[BE]/i, label: 'AMaster PID 0x402B/0x402E' },
  { pattern: /0x0e8d/i, label: 'MediaTek VID 0x0e8d' },
  // 已知 HID++ 错误码与短/长报告转换（来自 3.3）
  { pattern: /hidpp_short_output_as_long_report/, label: 'HID++ 短报告转长报告函数' },
  { pattern: /0x8F\b.*HID\+\+|HID\+\+.*0x8F/, label: 'HID++ 1.0 错误 sub ID 0x8F' },
];

// UI 与插件协议耦合检查（保留原有规则）。
const protocolCouplingPatterns = [
  { pattern: /invoke\s*\(\s*['"]read_projection['"]/, label: 'UI 直接调用 read_projection' },
  { pattern: /invoke\s*\(\s*['"]read_device_with_package['"]/, label: 'UI 直接调用 runtime read' },
  { pattern: /protocol\/workflows\.json/, label: 'UI 读取插件协议文件' },
];
const protocolFilePattern = /^src\/(?!.*\.test\.tsx?$).*\.[tc]sx?$/;

// 已知品牌字段名（来自 3.1）：宿主不得用这些字段名推断语义。
const brandFieldPatterns = [
  { pattern: /\bmouseLightMode\b/, label: 'AMaster mouseLightMode 字段' },
  { pattern: /\bmouseEffect\b/, label: 'AMaster mouseEffect 字段' },
  { pattern: /\breceiverLight\b/, label: 'AMaster receiverLight 字段' },
  { pattern: /\bcolorLedInfo\b/, label: 'AMaster colorLedInfo 字段' },
  { pattern: /\brgbEffectsInfo\b/, label: 'AMaster rgbEffectsInfo 字段' },
  { pattern: /\bcolor1\b/, label: 'AMaster color1 字段（需人工确认上下文）' },
];

const violations = [];

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    if (ignoredDirs.has(entry.name)) continue;
    const path = join(dir, entry.name);
    const rel = relative(root, path);
    if (entry.isDirectory()) {
      await walk(path);
      continue;
    }
    if (!hasTextExt(rel)) continue;
    const normalized = rel.split(sep).join('/');
    // 白名单文件直接跳过（但仍执行 UI 协议耦合检查？不，白名单就是全部跳过）。
    const isAllowed = allowedFiles.has(normalized) || allowedFiles.has(rel);
    if (isAllowed) continue;

    const text = await readFile(path, 'utf8').catch(() => '');
    if (!text) continue;
    const lines = text.split(/\r?\n/);

    // 1) 品牌关键词检查（只对生产代码，跳过注释和 test mod）
    if (isProductionCode(normalized)) {
      const isRust = normalized.endsWith('.rs');
      const tracker = isRust ? makeTestModTracker() : null;
      // 后备策略：Rust 文件中第一个 #[cfg(test)] 之后的代码视为测试代码。
      // Rust 惯例是 #[cfg(test)] mod tests { ... } 放在文件末尾，#[cfg(test)] 后只有测试。
      // 与 tracker 结合：两者任一判定为测试则跳过。
      let firstCfgTestLine = -1;
      if (isRust) {
        for (let i = 0; i < lines.length; i++) {
          if (/^\s*#\[\s*cfg\s*\(\s*test\s*\)\s*\]/.test(lines[i])) {
            firstCfgTestLine = i;
            break;
          }
        }
      }
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (tracker) tracker.update(line);
        // 跳过注释行：注释中提及品牌名属于知识记录，不是运行时耦合
        if (isCommentLine(line)) continue;
        // 跳过 test mod 内的测试数据（tracker 判定）
        if (tracker && tracker.inTest()) continue;
        // 后备：第一个 #[cfg(test)] 之后的行视为测试代码
        if (firstCfgTestLine >= 0 && i > firstCfgTestLine) continue;
        for (const { pattern, label } of brandPatterns) {
          if (pattern.test(line)) {
            violations.push(`${normalized}:${i + 1}: brand-lint: ${label} | ${line.trim().slice(0, 160)}`);
          }
        }
        for (const { pattern, label } of brandFieldPatterns) {
          if (pattern.test(line)) {
            violations.push(`${normalized}:${i + 1}: brand-field-lint: ${label} | ${line.trim().slice(0, 160)}`);
          }
        }
      }
    }

    // 2) UI 协议耦合检查（保留原有规则，只对 src/ 下非测试 .ts/.tsx）
    if (protocolFilePattern.test(normalized)) {
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        for (const { pattern, label } of protocolCouplingPatterns) {
          if (pattern.test(line)) {
            violations.push(`${normalized}:${i + 1}: ui-protocol-coupling: ${label} | ${line.trim().slice(0, 160)}`);
          }
        }
      }
    }
  }
}

await walk(root);

if (violations.length) {
  console.error(`\n架构 lint 失败：发现 ${violations.length} 处违规\n`);
  console.error(violations.join('\n'));
  console.error('\n参考：Mira_All_Plugins_Development_Requirements_Complete_Decoupling_Trae_Prompt_v7 第 1.1、3.1-3.3、11、16 节');
  process.exit(1);
}
console.log('architecture lint: clean (brand boundaries ok, ui protocol coupling ok)');
