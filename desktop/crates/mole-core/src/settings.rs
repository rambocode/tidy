// 跨进程共享的应用设置：`~/.config/mole/settings`，一行一个 `key=value`。
//
// 存在的理由：前端的 localStorage 只有 WebView 能读，而遥测开关必须在 Rust
// 侧生效（决定要不要建立网络连接），自动更新开关也需要在窗口起来之前就能读到。
// 因此这两类"后端也要看"的偏好统一落到这个文件，与 whitelist / purge_paths
// 同目录，用户删掉整个目录即回到出厂状态。

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

/// 设置文件路径。
pub fn settings_file(home: &str) -> PathBuf {
    PathBuf::from(home).join(format!(".config/{}/settings", crate::brand::CONFIG_DIR))
}

/// 读取全部键值对。文件缺失或损坏都返回空表，绝不 panic —— 设置读失败必须
/// 退化成"用默认值"，而不是让应用起不来。
pub fn load(home: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(content) = fs::read_to_string(settings_file(home)) else {
        return map;
    };
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                map.insert(key.to_string(), value.trim().to_string());
            }
        }
    }
    map
}

/// 读一个键。
pub fn get(home: &str, key: &str) -> Option<String> {
    load(home).remove(key)
}

/// 读一个布尔键，缺失或无法解析时返回默认值。只认 `1`/`true`/`on` 为真。
pub fn bool_or(home: &str, key: &str, default: bool) -> bool {
    match get(home, key) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on"),
        None => default,
    }
}

/// 写一个键。
///
/// 先写同目录临时文件再 rename，避免进程在写一半时被杀导致设置文件半截损坏
/// （损坏会让遥测开关静默回到默认的"开"，那是用户不能接受的翻转方向）。
pub fn set(home: &str, key: &str, value: &str) -> io::Result<()> {
    if key.is_empty() || key.contains('=') || key.contains('\n') || value.contains('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings key/value must not contain '=' or newlines",
        ));
    }
    let mut map = load(home);
    map.insert(key.to_string(), value.to_string());

    let path = settings_file(home);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body: String = map
        .iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect::<Vec<_>>()
        .join("");

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独占一个临时 home，避免并行测试互相污染（不用环境变量，
    /// 因为 std::env::set_var 是进程级的，测试线程之间会打架）。
    fn with_temp<T>(f: impl FnOnce(&str) -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        f(dir.path().to_str().unwrap())
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        with_temp(|home| {
            assert_eq!(get(home, "telemetry.enabled"), None);
            assert!(bool_or(home, "telemetry.enabled", true));
            assert!(!bool_or(home, "telemetry.enabled", false));
        });
    }

    #[test]
    fn set_then_get_roundtrips_and_preserves_other_keys() {
        with_temp(|home| {
            set(home, "telemetry.enabled", "0").unwrap();
            set(home, "update.autocheck", "1").unwrap();
            assert_eq!(get(home, "telemetry.enabled").as_deref(), Some("0"));
            assert!(!bool_or(home, "telemetry.enabled", true));
            assert!(bool_or(home, "update.autocheck", false));
        });
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        with_temp(|home| {
            fs::create_dir_all(settings_file(home).parent().unwrap()).unwrap();
            fs::write(settings_file(home), "# note\n\n  a = 1 \nbroken\n").unwrap();
            assert_eq!(get(home, "a").as_deref(), Some("1"));
            assert_eq!(get(home, "broken"), None);
        });
    }

    #[test]
    fn rejects_keys_and_values_that_would_corrupt_the_file() {
        with_temp(|home| {
            assert!(set(home, "a=b", "1").is_err());
            assert!(set(home, "a", "1\nb=2").is_err());
        });
    }
}
