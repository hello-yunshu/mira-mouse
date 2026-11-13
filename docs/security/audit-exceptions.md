# Cargo Audit 例外清单

本文件是 CI `cargo audit` 中每一条 `--ignore` 的唯一依据。禁止通配忽略；新增例外必须先在本文件登记，并在到期前复查。

复查日期：**2027-02-03**（由 2026-11-03 起三个月）。

## 当前例外

| Advisory | Crate / 版本 | 依赖链 | 平台范围 | 不修复原因 | 复查日期 |
|---|---|---|---|---|---|
| RUSTSEC-2024-0370 | proc-macro-error 1.0.4 | glib-macros 0.18.5 / gtk3-macros 0.18.2 → proc-macro-error | 仅 Linux GTK 构建期 | 上游只有 1.0.4，无已修复版本；仅作为 GTK 派生宏的构建期依赖，不进入运行时 | 2027-02-03 |
| RUSTSEC-2024-0429 | glib 0.18.5 | tauri 2.12.5 → gtk 0.18 → atk/gdk/pango/webkit2gtk → glib 0.18.5 | 仅 Linux GTK | 修复版本 glib >= 0.20 与 gtk-rs 0.18 栈语义不兼容；需等待 Tauri 上游整体升级 gtk-rs | 2027-02-03 |
| RUSTSEC-2026-0370 | zvariant 5.13.0 | zbus 5.17.0 / zbus_macros 5.17.0 → zvariant | 仅 Linux 系统集成 | 该 crate 被标记 unmaintained，5.15.0 仍无修复公告；zbus 5.x 尚未迁移到替代实现 | 2027-02-03 |
| RUSTSEC-2026-0371 | zbus_names 4.3.3 | zbus 5.17.0 → zbus_names | 仅 Linux 系统集成 | 该 crate 被标记 unmaintained，4.3.4 仍无修复公告 | 2027-02-03 |
| RUSTSEC-2026-0372 | zvariant_derive 5.13.0 | zvariant 5.13.0 → zvariant_derive | 仅 Linux 系统集成 | 该 crate 被标记 unmaintained，随 zvariant 一起等待上游迁移 | 2027-02-03 |

## 已移除的旧例外

- `RUSTSEC-2026-0194`、`RUSTSEC-2026-0195`（quick-xml）：`Cargo.lock` 现已解析到 `quick-xml 0.41.0`，达到修复版本，CI 不再忽略。
- `RUSTSEC-2024-0429` 仍保留，但原因已按当前 `tauri 2.12.5` 依赖链更新。

## 维护规则

1. 只允许逐条 `--ignore`，禁止通配或全局关闭 audit。
2. 每条忽略必须能在此表中找到同名条目，并填写复查日期。
3. 复查时先升级依赖；若上游仍无修复版本，更新原因与下一次复查日期。
4. `cargo audit` 的漏洞（vulnerabilities）与 yanked 检查不允许忽略。

## npm 依赖 override（非 cargo audit 例外）

- `minimatch` 通过 `package.json` 的 `overrides` 固定为 `^11.0.1`（实际解析 `11.0.8`）。
- 原因：`eslint 10.14.0`、`@eslint/config-array 0.23.5`、`@typescript-eslint/typescript-estree 8.72.1` 仍声明 `minimatch ^10.2.x`，而该漏洞的修复版本是 `>=11.0.1`；上游父包尚未支持 11。
- 验证：`npm ci`、`npm run lint`、`npm run test`、`npm run build`、`cargo build --workspace --locked` 均通过；`npm audit` 为 0。
- 复查：当上述父包声明 `minimatch ^11` 时移除 override；复查日期同 2027-02-03。

