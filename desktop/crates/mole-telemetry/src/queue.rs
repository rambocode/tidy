// 上报队列：内存缓冲 + 失败落盘。
//
// 为什么不直接发：清理/扫描过程中事件会成串出现，逐条发会开一堆连接，也会
// 在离线时全部丢掉。这里攒批，60 秒或退出时发一次；发失败就落盘，下次启动
// 接着发，所以断网一天的数据不会消失。
//
// 队列有硬上限（500 条）。上限是必须的：一个长期离线的用户不该在磁盘上攒出
// 一个无限增长的文件。超限时丢**最旧**的，因为新事件更能反映当前版本。

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// 队列最多保留的事件条数（内存与磁盘同一上限）。
pub const CAPACITY: usize = 500;

/// 磁盘溢出文件：`~/.config/mole/telemetry-queue.json`。
pub fn spill_file(home: &str) -> PathBuf {
    PathBuf::from(home).join(format!(
        ".config/{}/telemetry-queue.json",
        mole_core::brand::CONFIG_DIR
    ))
}

/// 删除溢出队列文件。
///
/// SAFE: 只删 mole-telemetry 自己写出来的那一个文件，路径完全由
/// `spill_file` 计算，永远不来自用户输入或扫描结果。走 mole_core::sink
/// 会给这个纯内部缓存加上废纸篓副本和一条删除审计记录，那是噪音。
#[allow(clippy::disallowed_methods)]
pub fn discard_spill(path: &Path) {
    let _ = fs::remove_file(path);
}

/// 读回上次未发送成功的事件。文件损坏时当作空队列并删除它——一个坏掉的
/// 队列文件不值得让应用每次启动都失败一次。
pub fn load_spill(path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<Value>>(&text) {
        Ok(mut events) => {
            trim(&mut events);
            events
        }
        Err(_) => {
            discard_spill(path);
            Vec::new()
        }
    }
}

/// 把未发送成功的事件写回磁盘；空队列则删除文件，不留空壳。
pub fn save_spill(path: &Path, events: &[Value]) {
    if events.is_empty() {
        discard_spill(path);
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(events) {
        let _ = fs::write(path, text);
    }
}

/// 丢弃超出上限的最旧事件。
pub fn trim(events: &mut Vec<Value>) {
    if events.len() > CAPACITY {
        events.drain(..events.len() - CAPACITY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trim_drops_the_oldest_events_first() {
        let mut events: Vec<Value> = (0..CAPACITY + 10).map(|i| json!({ "i": i })).collect();
        trim(&mut events);
        assert_eq!(events.len(), CAPACITY);
        assert_eq!(events[0]["i"], json!(10));
    }

    #[test]
    fn spill_roundtrips_and_empty_queue_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.json");
        save_spill(&path, &[json!({ "event": "app_launched" })]);
        assert_eq!(load_spill(&path).len(), 1);
        save_spill(&path, &[]);
        assert!(!path.exists());
        assert!(load_spill(&path).is_empty());
    }

    #[test]
    fn corrupt_spill_file_is_discarded_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.json");
        fs::write(&path, "{not json").unwrap();
        assert!(load_spill(&path).is_empty());
        assert!(!path.exists());
    }
}
