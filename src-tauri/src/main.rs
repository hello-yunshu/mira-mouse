// SPDX-License-Identifier: AGPL-3.0-or-later
// 防止 Windows release 构建启动时出现命令行窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mira_app_lib::run()
}
