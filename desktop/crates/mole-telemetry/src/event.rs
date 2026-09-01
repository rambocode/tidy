// 事件定义：一个编译期封闭集合。
//
// 关键约束：除版本号外，所有属性值都是 `&'static str` 或 u64。这不是风格
// 偏好，而是**结构性**保证——调用方在语法上就无法把运行期字符串（路径、
// 文件名、应用名）塞进遥测负载。隐私页承诺的"不采集路径/文件名/体积"由
// 这个类型系统兜底，而不是靠 review 时盯着看。
//
// 唯一的 String 是版本号，且发送前会过 `sanitize_version` 白名单过滤。

use serde_json::{json, Map, Value};

/// 一条待上报的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 应用启动。`first_run` 区分首次安装。
    AppLaunched { first_run: bool },
    /// 打开某个界面。
    ViewOpened { view: &'static str },
    /// 一次扫描完成。只带类型和耗时——耗时是性能指标，不描述用户的文件。
    ScanCompleted {
        kind: &'static str,
        duration_ms: u64,
    },
    /// 执行了一次清理。刻意不带数量和体积（见隐私页）。
    CleanExecuted {
        mode: &'static str,
        result: &'static str,
    },
    /// 卸载了应用。刻意不带数量，也不带被卸载应用的名字。
    AppUninstalled,
    /// 执行了一个优化任务。`action` 取自 mole-ops 静态任务目录。
    OptimizeRun { action: &'static str },
    /// 执行了一次「帮别的应用升级」。
    UpdatesRun { source: &'static str },
    /// Tidy 自身更新。
    SelfUpdate {
        from: String,
        to: String,
        result: &'static str,
    },
    /// 出错。只带预定义错误码，绝不带错误消息原文（原文里常有绝对路径）。
    ErrorOccurred {
        code: &'static str,
        view: &'static str,
    },
}

impl Event {
    /// PostHog 事件名。
    pub fn name(&self) -> &'static str {
        match self {
            Event::AppLaunched { .. } => "app_launched",
            Event::ViewOpened { .. } => "view_opened",
            Event::ScanCompleted { .. } => "scan_completed",
            Event::CleanExecuted { .. } => "clean_executed",
            Event::AppUninstalled => "app_uninstalled",
            Event::OptimizeRun { .. } => "optimize_run",
            Event::UpdatesRun { .. } => "updates_run",
            Event::SelfUpdate { .. } => "self_update",
            Event::ErrorOccurred { .. } => "error_occurred",
        }
    }

    /// 事件自身的属性（不含 install id / 版本等公共属性）。
    pub fn properties(&self) -> Map<String, Value> {
        let mut props = Map::new();
        let mut put = |key: &str, value: Value| {
            props.insert(key.to_string(), value);
        };
        match self {
            Event::AppLaunched { first_run } => put("first_run", json!(first_run)),
            Event::ViewOpened { view } => put("view", json!(view)),
            Event::ScanCompleted { kind, duration_ms } => {
                put("kind", json!(kind));
                put("duration_ms", json!(duration_ms));
            }
            Event::CleanExecuted { mode, result } => {
                put("mode", json!(mode));
                put("result", json!(result));
            }
            Event::AppUninstalled => {}
            Event::OptimizeRun { action } => put("action", json!(action)),
            Event::UpdatesRun { source } => put("source", json!(source)),
            Event::SelfUpdate { from, to, result } => {
                put("from_version", json!(sanitize_version(from)));
                put("to_version", json!(sanitize_version(to)));
                put("result", json!(result));
            }
            Event::ErrorOccurred { code, view } => {
                put("code", json!(code));
                put("view", json!(view));
            }
        }
        props
    }
}

/// 版本号是唯一允许在事件里出现的运行期字符串，所以按白名单洗一遍：只留
/// 数字、字母、`.`、`-`、`+`，并截断到 32 字符。哪怕上游 feed 被投毒，也
/// 不可能把一段路径或一行日志夹带进来。
pub fn sanitize_version(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_sanitizer_strips_anything_path_shaped() {
        assert_eq!(sanitize_version("1.2.3"), "1.2.3");
        assert_eq!(sanitize_version("1.0.0-beta.1+build"), "1.0.0-beta.1+build");
        assert_eq!(sanitize_version("/Users/mike/secret"), "Usersmikesecret");
        assert_eq!(sanitize_version(&"9".repeat(64)).len(), 32);
    }

    #[test]
    fn clean_event_carries_no_counts_or_sizes() {
        let props = Event::CleanExecuted {
            mode: "trash",
            result: "ok",
        }
        .properties();
        assert_eq!(props.len(), 2);
        assert!(props.contains_key("mode") && props.contains_key("result"));
    }

    #[test]
    fn uninstall_event_carries_nothing_at_all() {
        assert!(Event::AppUninstalled.properties().is_empty());
    }
}
