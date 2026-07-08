// SPDX-License-Identifier: AGPL-3.0-or-later
//! 托盘 / 菜单栏状态显示模块。
//!
//! 四层架构：
//! - 状态层 `state`：从 `DeviceSnapshot` + `AppSettings` 提取托盘状态
//! - 视觉规则层 `style`：根据主题和设置决定颜色
//! - 平台渲染层 `renderer` + `dynamic_icon`：把状态和视觉规则画出来
//! - 平台接入层：由 `lib.rs` 中的 `update_tray` 调用控制器
//!
//! 静态 PNG fallback 见 `static_icon`，共享绘图原语见 `image`。

pub mod dynamic_icon;
pub mod image;
pub mod renderer;
pub mod state;
pub mod static_icon;
pub mod style;

// macOS 原生 NSStatusItem 渲染器（step 5）
#[cfg(target_os = "macos")]
pub mod macos;
