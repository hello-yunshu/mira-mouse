// SPDX-License-Identifier: AGPL-3.0-or-later
// 仅供 attention.test.tsx 读取 attention-beam.css 原始文本做断言用的最小类型。
//
// vitest 默认 stub CSS 导入（`?raw` 会返回空串），测试改用 node:fs 直接读源文件。
// 项目 tsconfig.app 的 types 只含 vite/client（不加载 @types/node），因此在此
// 声明 node:fs / node:path 的最小模块形状与 ImportMeta.dirname。
// 注意：不要在这里引入全局 timer/setTimeout 等 node 全局声明，否则会把
// window.setInterval 等解析成 node 的 Timeout 类型，破坏应用代码类型检查。

declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
}

declare module 'node:path' {
  export function join(...parts: string[]): string;
}

interface ImportMeta {
  /** Node ≥ 20.11 提供的当前模块目录（vitest 运行于 Node）。 */
  dirname: string;
}
