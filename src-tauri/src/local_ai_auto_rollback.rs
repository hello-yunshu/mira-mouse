// SPDX-License-Identifier: AGPL-3.0-or-later
//! 本地 AI bundle 自动回滚 supervisor。
//!
//! 监听 controller 投递的 [`CrashEvent`],在 10 分钟滑动窗口内累积 ≥3 次失败时
//! 自动回滚到上一个 bundle 版本并重启 controller;无 previous 可回滚时关闭
//! `local_ai_analysis_enabled` 开关并发送系统通知。supervisor 在独立线程运行,
//! 阻塞 I/O(rollback、save_settings、通知)不影响 predict 路径。
//!
//! supervisor 不持久化任何状态:Mira 重启后从干净态开始,避免一次回滚后永久
//! 不再保护。`rolled_back` 标志挡住本次会话内的重复触发,避免反复回滚循环。

use std::{
    collections::VecDeque,
    sync::{mpsc::Receiver, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager};

use crate::{
    cached_settings, local_ai_controller::CrashEvent, local_ai_update, save_settings,
    update_cached_settings, SessionState,
};

/// 10 分钟滑动窗口:窗口外的失败时间戳被清出。
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// 窗口内失败次数阈值,达到即触发自动回滚。
const FAILURE_THRESHOLD: usize = 3;

/// supervisor 写入的自动状态,供 `local_ai_status` 命令读取,让前端区分
/// 自动回滚/自动关开关/手动操作。
#[derive(Debug, Clone, Default)]
pub struct AutoState(pub Arc<Mutex<Option<String>>>);

impl AutoState {
    /// 读取当前自动状态标识(`"autoRolledBack"` / `"autoDisabled"` / `None`)。
    pub fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|g| g.clone())
    }

    fn set(&self, value: impl Into<String>) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(value.into());
        }
    }
}

pub struct AutoRollbackSupervisor {
    app: AppHandle,
    crash_rx: Receiver<CrashEvent>,
    auto_state: AutoState,
}

impl AutoRollbackSupervisor {
    /// 启动 supervisor 线程。
    ///
    /// `auto_state` 由调用方(SessionState)持有,supervisor 写入自动回滚/关开关
    /// 标识,`local_ai_status` 命令读取以让前端区分自动与手动操作。
    ///
    /// 线程生命周期由 channel 控制:`crash_tx` 全部 drop 时 `recv` 返回 `Err`,
    /// supervisor 退出。`crash_tx` 由 `LocalAiController` 持有,跟随 `SessionState`
    /// 一起 drop,因此 App 关闭时 supervisor 自动退出,不会泄漏。
    pub fn start(app: AppHandle, crash_rx: Receiver<CrashEvent>, auto_state: AutoState) {
        let supervisor = Self {
            app,
            crash_rx,
            auto_state,
        };
        thread::spawn(move || supervisor.run());
    }

    fn run(self) {
        let mut failure_window: VecDeque<Instant> = VecDeque::new();
        let mut rolled_back = false;
        while let Ok(event) = self.crash_rx.recv() {
            let (new_window, trigger) =
                process_failure_event(std::mem::take(&mut failure_window), &event, rolled_back);
            failure_window = new_window;
            if trigger {
                rolled_back = true;
                self.execute_rollback();
            }
        }
    }

    /// 两段式回滚:先试本地 rollback,有 previous 则重启;无 previous 则关开关 + 通知。
    fn execute_rollback(&self) {
        match local_ai_update::rollback(&self.app, "bundle") {
            Ok(_) => {
                let controller = self.app.state::<SessionState>();
                let _ = controller.local_ai_controller.restart(&self.app);
                self.auto_state.set("autoRolledBack");
            }
            Err(_) => {
                self.disable_local_ai_and_notify();
            }
        }
    }

    fn disable_local_ai_and_notify(&self) {
        let mut settings = cached_settings(&self.app);
        settings.local_ai_analysis_enabled = false;
        let _ = save_settings(&self.app, &settings);
        update_cached_settings(&self.app, &settings);
        let controller = self.app.state::<SessionState>();
        controller.local_ai_controller.stop();
        self.auto_state.set("autoDisabled");
        show_local_ai_notification(
            &self.app,
            "本地 AI 已停用",
            "当前版本连续故障且无可用历史版本,已自动关闭本地 AI 分析。",
        );
    }
}

/// 处理单个事件,返回新窗口和是否应触发回滚。
///
/// 抽成纯函数便于单元测试:窗口清理、阈值判断、rolled_back 短路逻辑都不
/// 依赖 AppHandle 或线程,可以独立验证。
fn process_failure_event(
    mut window: VecDeque<Instant>,
    event: &CrashEvent,
    rolled_back: bool,
) -> (VecDeque<Instant>, bool) {
    match event {
        CrashEvent::Failed { at, .. } => {
            if rolled_back {
                return (window, false);
            }
            window.push_back(*at);
            // 清理窗口外(> 10 分钟)的旧失败记录。
            while let Some(front) = window.front() {
                if front.elapsed() > FAILURE_WINDOW {
                    window.pop_front();
                } else {
                    break;
                }
            }
            let trigger = window.len() >= FAILURE_THRESHOLD;
            (window, trigger)
        }
        CrashEvent::Success => {
            if !rolled_back {
                window.clear();
            }
            (window, false)
        }
    }
}

/// 跨平台系统通知,无点击跳转动作,仅告知用户。
///
/// macOS:`tauri-plugin-notification`(项目规则要求系统级通知而非界面内消息)。
/// Windows/Linux:`notify-rust`。
fn show_local_ai_notification(app: &AppHandle, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = app.notification().builder().title(title).body(body).show();
    }
    #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
    {
        let identifier = app.config().identifier.clone();
        let title = title.to_string();
        let body = body.to_string();
        thread::spawn(move || {
            let mut notification = notify_rust::Notification::new();
            notification
                .summary(&title)
                .body(&body)
                .timeout(notify_rust::Timeout::Never);
            #[cfg(target_os = "windows")]
            notification.app_id(&identifier);
            #[cfg(all(unix, not(target_os = "macos")))]
            notification.appname(&identifier);
            let _ = notification.show();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_window_evicts_old_entries() {
        let now = Instant::now();
        let old = now - Duration::from_secs(11 * 60);
        let mut window = VecDeque::new();
        window.push_back(old);
        window.push_back(old);
        // 推入一个新失败,旧的两个应被清出,窗口只剩 1 个。
        let (new_window, trigger) = process_failure_event(
            window,
            &CrashEvent::Failed {
                at: now,
                reason: "x".into(),
            },
            false,
        );
        assert_eq!(new_window.len(), 1);
        assert!(!trigger);
    }

    #[test]
    fn three_failures_in_window_triggers_rollback() {
        let now = Instant::now();
        let mut window = VecDeque::new();
        let mut trigger = false;
        for i in 0..3 {
            let at = now - Duration::from_secs(i * 60);
            let (w, t) = process_failure_event(
                window,
                &CrashEvent::Failed {
                    at,
                    reason: format!("fail-{i}"),
                },
                false,
            );
            window = w;
            trigger = t;
        }
        assert!(trigger);
        assert_eq!(window.len(), 3);
    }

    #[test]
    fn success_resets_window() {
        let now = Instant::now();
        let mut window = VecDeque::new();
        window.push_back(now);
        window.push_back(now);
        let (new_window, trigger) = process_failure_event(window, &CrashEvent::Success, false);
        assert!(new_window.is_empty());
        assert!(!trigger);
    }

    #[test]
    fn rolled_back_flag_blocks_repeat_rollback() {
        let now = Instant::now();
        let window = VecDeque::new();
        // 已经回滚过,后续失败应被忽略,不再触发。
        let (new_window, trigger) = process_failure_event(
            window,
            &CrashEvent::Failed {
                at: now,
                reason: "post-rollback".into(),
            },
            true,
        );
        assert!(new_window.is_empty());
        assert!(!trigger);
    }

    #[test]
    fn success_after_rolled_back_does_not_clear() {
        // 回滚后收到 Success 不应清窗口(窗口已无效,但语义上不应影响 rolled_back 状态)。
        let now = Instant::now();
        let mut window = VecDeque::new();
        window.push_back(now);
        let (new_window, _) = process_failure_event(window, &CrashEvent::Success, true);
        // rolled_back 时 Success 不清窗口(避免误重置已回滚状态)。
        assert_eq!(new_window.len(), 1);
    }

    #[test]
    fn two_failures_do_not_trigger() {
        let now = Instant::now();
        let mut window = VecDeque::new();
        let mut trigger = false;
        for _ in 0..2 {
            let (w, t) = process_failure_event(
                window,
                &CrashEvent::Failed {
                    at: now,
                    reason: "fail".into(),
                },
                false,
            );
            window = w;
            trigger = t;
        }
        assert!(!trigger);
        assert_eq!(window.len(), 2);
    }
}
