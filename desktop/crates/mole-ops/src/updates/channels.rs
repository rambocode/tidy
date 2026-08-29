//! Native app update-channel discovery: Mac App Store lookup, Sparkle appcast,
//! and Electron updater metadata. Every network probe is bounded; failure is
//! reported as Unknown rather than "up to date".

use super::command;
use super::{app_info_plist, update_id, version_is_newer, AppUpdate, UpToDateApp};
use crate::uninstall::AppInfo;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct NativeScan {
    pub updates: Vec<AppUpdate>,
    pub up_to_date: Vec<UpToDateApp>,
    pub warnings: Vec<String>,
}

enum ProbeResult {
    Update(Box<AppUpdate>),
    Current(UpToDateApp),
    Warning(String),
    NotSupported,
}

pub(crate) fn scan(apps: &[AppInfo], cancelled: &AtomicBool) -> NativeScan {
    let candidates: Vec<AppInfo> = apps
        .iter()
        .filter(|app| local_channel(app).is_some())
        .cloned()
        .collect();
    if candidates.is_empty() {
        return NativeScan {
            updates: Vec::new(),
            up_to_date: Vec::new(),
            warnings: Vec::new(),
        };
    }
    let queue = Mutex::new(candidates);
    let results = Mutex::new(Vec::new());
    let workers = apps.len().clamp(1, 4);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let app = queue.lock().unwrap().pop();
                let Some(app) = app else { break };
                results.lock().unwrap().push(probe_app(&app, cancelled));
            });
        }
    });

    let mut scan = NativeScan {
        updates: Vec::new(),
        up_to_date: Vec::new(),
        warnings: Vec::new(),
    };
    for result in results.into_inner().unwrap() {
        match result {
            ProbeResult::Update(update) => scan.updates.push(*update),
            ProbeResult::Current(app) => scan.up_to_date.push(app),
            ProbeResult::Warning(warning) => scan.warnings.push(warning),
            ProbeResult::NotSupported => {}
        }
    }
    scan
}

#[derive(Clone)]
enum LocalChannel {
    AppStore,
    Sparkle(String),
    Electron(String),
}

fn local_channel(app: &AppInfo) -> Option<LocalChannel> {
    let path = std::path::Path::new(&app.path);
    if path.join("Contents/_MASReceipt/receipt").is_file() {
        return Some(LocalChannel::AppStore);
    }
    let info = app_info_plist(path)?;
    if let Some(feed) = info
        .get("SUFeedURL")
        .and_then(plist::Value::as_string)
        .filter(|url| safe_https_url(url))
    {
        return Some(LocalChannel::Sparkle(feed.to_string()));
    }
    let electron = path.join("Contents/Resources/app-update.yml");
    electron
        .is_file()
        .then(|| LocalChannel::Electron(electron.to_string_lossy().into_owned()))
}

fn probe_app(app: &AppInfo, cancelled: &AtomicBool) -> ProbeResult {
    match local_channel(app) {
        Some(LocalChannel::AppStore) => probe_app_store(app, cancelled),
        Some(LocalChannel::Sparkle(feed)) => probe_sparkle(app, &feed, cancelled),
        Some(LocalChannel::Electron(config)) => probe_electron(app, &config, cancelled),
        None => ProbeResult::NotSupported,
    }
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    #[serde(default)]
    results: Vec<LookupProduct>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookupProduct {
    track_id: u64,
    version: String,
    #[serde(default)]
    release_notes: Option<String>,
}

fn probe_app_store(app: &AppInfo, cancelled: &AtomicBool) -> ProbeResult {
    if !valid_bundle_id(&app.bundle_id) {
        return ProbeResult::Warning(format!("app_store:{}:invalid_bundle_id", app.name));
    }
    let url = format!(
        "https://itunes.apple.com/lookup?bundleId={}&entity=macSoftware&country={}",
        app.bundle_id,
        locale_country()
    );
    let response = match fetch(&url, cancelled, None) {
        Ok(response) => response,
        Err(cause) => return ProbeResult::Warning(format!("app_store:{}:{cause}", app.name)),
    };
    let lookup: LookupResponse = match serde_json::from_str(&response) {
        Ok(lookup) => lookup,
        Err(error) => {
            return ProbeResult::Warning(format!("app_store:{}:invalid_json:{error}", app.name))
        }
    };
    let Some(product) = lookup.results.into_iter().next() else {
        return ProbeResult::Warning(format!("app_store:{}:not_found", app.name));
    };
    if !version_is_newer(&product.version, &app.version) {
        return ProbeResult::Current(UpToDateApp {
            name: app.name.clone(),
            version: app.version.clone(),
            source: "app_store".into(),
        });
    }
    ProbeResult::Update(Box::new(AppUpdate {
        id: update_id(&format!("app_store:{}", app.bundle_id)),
        kind: "app".into(),
        name: app.name.clone(),
        bundle_id: Some(app.bundle_id.clone()),
        app_path: Some(app.path.clone()),
        installed: app.version.clone(),
        latest: product.version,
        source: "app_store".into(),
        action: "open_app_store".into(),
        release_notes: product.release_notes.map(|notes| truncate(&notes, 1200)),
        command_hint: None,
        ignored: false,
        external_url: Some(format!(
            "macappstore://itunes.apple.com/app/id{}",
            product.track_id
        )),
    }))
}

fn probe_sparkle(app: &AppInfo, feed: &str, cancelled: &AtomicBool) -> ProbeResult {
    let xml = match fetch(feed, cancelled, Some("application/xml, text/xml")) {
        Ok(xml) => xml,
        Err(cause) => return ProbeResult::Warning(format!("sparkle:{}:{cause}", app.name)),
    };
    let item = xml
        .find("<item")
        .map(|start| &xml[start..])
        .unwrap_or(xml.as_str());
    let version = extract_attribute(item, "sparkle:shortVersionString")
        .or_else(|| extract_tag(item, "sparkle:shortVersionString"))
        .or_else(|| extract_attribute(item, "sparkle:version"));
    let Some(version) = version.filter(|version| sane_version(version)) else {
        return ProbeResult::Warning(format!("sparkle:{}:missing_version", app.name));
    };
    if !version_is_newer(&version, &app.version) {
        return ProbeResult::Current(UpToDateApp {
            name: app.name.clone(),
            version: app.version.clone(),
            source: "sparkle".into(),
        });
    }
    let notes = extract_tag(item, "description").map(|value| truncate(&strip_markup(&value), 1200));
    ProbeResult::Update(Box::new(AppUpdate {
        id: update_id(&format!("sparkle:{}", app.bundle_id)),
        kind: "app".into(),
        name: app.name.clone(),
        bundle_id: nonempty(&app.bundle_id),
        app_path: Some(app.path.clone()),
        installed: app.version.clone(),
        latest: version,
        source: "sparkle".into(),
        action: "open_app".into(),
        release_notes: notes,
        command_hint: None,
        ignored: false,
        external_url: Some(feed.to_string()),
    }))
}

fn probe_electron(app: &AppInfo, config_path: &str, cancelled: &AtomicBool) -> ProbeResult {
    let config = match std::fs::read_to_string(config_path) {
        Ok(config) => parse_simple_yaml(&config),
        Err(error) => return ProbeResult::Warning(format!("electron:{}:config:{error}", app.name)),
    };
    let provider = config
        .get("provider")
        .map(String::as_str)
        .unwrap_or("github");
    let remote = if provider == "github" {
        let Some(owner) = config.get("owner").filter(|value| valid_github_part(value)) else {
            return ProbeResult::Warning(format!("electron:{}:missing_owner", app.name));
        };
        let Some(repo) = config.get("repo").filter(|value| valid_github_part(value)) else {
            return ProbeResult::Warning(format!("electron:{}:missing_repo", app.name));
        };
        github_release(owner, repo, cancelled)
    } else if provider == "generic" {
        let Some(base) = config.get("url").filter(|url| safe_https_url(url)) else {
            return ProbeResult::Warning(format!("electron:{}:invalid_url", app.name));
        };
        generic_electron_release(base, config.get("channel"), cancelled)
    } else if provider == "s3" {
        s3_electron_release(&config, cancelled)
    } else {
        return ProbeResult::Warning(format!(
            "electron:{}:unsupported_provider:{provider}",
            app.name
        ));
    };
    let remote = match remote {
        Ok(remote) => remote,
        Err(cause) => return ProbeResult::Warning(format!("electron:{}:{cause}", app.name)),
    };
    if !version_is_newer(&remote.version, &app.version) {
        return ProbeResult::Current(UpToDateApp {
            name: app.name.clone(),
            version: app.version.clone(),
            source: "electron".into(),
        });
    }
    ProbeResult::Update(Box::new(AppUpdate {
        id: update_id(&format!("electron:{}", app.bundle_id)),
        kind: "app".into(),
        name: app.name.clone(),
        bundle_id: nonempty(&app.bundle_id),
        app_path: Some(app.path.clone()),
        installed: app.version.clone(),
        latest: remote.version,
        source: "electron".into(),
        action: "open_app".into(),
        release_notes: remote.notes.map(|notes| truncate(&notes, 1200)),
        command_hint: None,
        ignored: false,
        external_url: Some(remote.url),
    }))
}

fn s3_electron_release(
    config: &std::collections::HashMap<String, String>,
    cancelled: &AtomicBool,
) -> Result<RemoteRelease, String> {
    let bucket = config
        .get("bucket")
        .filter(|value| valid_s3_part(value))
        .ok_or_else(|| "invalid_s3_bucket".to_string())?;
    let mut base = if let Some(endpoint) = config.get("endpoint").filter(|url| safe_https_url(url))
    {
        format!("{}/{}", endpoint.trim_end_matches('/'), bucket)
    } else {
        let region = config
            .get("region")
            .filter(|value| value.as_str() != "auto" && valid_s3_part(value));
        match region {
            Some(region) => format!("https://{bucket}.s3.{region}.amazonaws.com"),
            None => format!("https://{bucket}.s3.amazonaws.com"),
        }
    };
    if let Some(path) = config.get("path").filter(|value| valid_s3_path(value)) {
        base = format!("{}/{}", base.trim_end_matches('/'), path.trim_matches('/'));
    }
    generic_electron_release(&base, config.get("channel"), cancelled)
}

struct RemoteRelease {
    version: String,
    url: String,
    notes: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

fn github_release(
    owner: &str,
    repo: &str,
    cancelled: &AtomicBool,
) -> Result<RemoteRelease, String> {
    let api = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let response = fetch(&api, cancelled, Some("application/vnd.github+json"))?;
    let release: GithubRelease =
        serde_json::from_str(&response).map_err(|error| format!("invalid_json:{error}"))?;
    if !safe_https_url(&release.html_url) || !sane_version(&release.tag_name) {
        return Err("invalid_release".into());
    }
    Ok(RemoteRelease {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: release.html_url,
        notes: release.body,
    })
}

fn generic_electron_release(
    base: &str,
    channel: Option<&String>,
    cancelled: &AtomicBool,
) -> Result<RemoteRelease, String> {
    let channel = channel
        .filter(|channel| valid_channel(channel))
        .map(String::as_str)
        .unwrap_or("latest");
    let url = if base.ends_with(".yml") || base.ends_with(".yaml") {
        base.to_string()
    } else {
        format!("{}/{}-mac.yml", base.trim_end_matches('/'), channel)
    };
    let yaml = fetch(&url, cancelled, Some("text/yaml, text/plain"))?;
    let values = parse_simple_yaml(&yaml);
    let version = values
        .get("version")
        .filter(|value| sane_version(value))
        .cloned()
        .ok_or_else(|| "missing_version".to_string())?;
    Ok(RemoteRelease {
        version,
        url: base.to_string(),
        notes: None,
    })
}

fn fetch(url: &str, cancelled: &AtomicBool, accept: Option<&str>) -> Result<String, String> {
    if !safe_https_url(url) {
        return Err("unsafe_url".into());
    }
    let mut command = command::strings(&[
        "/usr/bin/curl",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--connect-timeout",
        "5",
        "--max-time",
        "12",
        "--user-agent",
        "Mole-Desktop-Update-Scanner/1",
    ]);
    if let Some(accept) = accept {
        command.extend(["--header".into(), format!("Accept: {accept}")]);
    }
    command.push(url.to_string());
    let output = command::run(&command, FETCH_TIMEOUT, cancelled);
    if output.success() {
        Ok(output.stdout)
    } else {
        Err(format!("fetch:{:?}:{}", output.status, output.text))
    }
}

fn extract_attribute(xml: &str, attribute: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let marker = format!("{attribute}={quote}");
        if let Some(position) = xml.find(&marker) {
            let start = position + marker.len();
            if let Some(end) = xml[start..].find(quote) {
                return Some(xml[start..start + end].trim().to_string());
            }
        }
    }
    None
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn strip_markup(value: &str) -> String {
    let mut text = String::new();
    let mut inside = false;
    for character in value.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => text.push(character),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_simple_yaml(value: &str) -> std::collections::HashMap<String, String> {
    value
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let value = value.trim().trim_matches(['"', '\'']);
            (!key.trim().is_empty() && !value.is_empty())
                .then(|| (key.trim().to_string(), value.to_string()))
        })
        .collect()
}

fn safe_https_url(value: &str) -> bool {
    value.starts_with("https://")
        && value.len() <= 4096
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
}

fn valid_bundle_id(value: &str) -> bool {
    value.contains('.')
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_github_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !value.starts_with('.')
}

fn valid_channel(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_s3_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_s3_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.split('/').any(|part| part == "..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'@')
        })
}

fn sane_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.bytes().any(|byte| byte.is_ascii_digit())
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn locale_country() -> String {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    locale
        .split(['_', '-'])
        .nth(1)
        .and_then(|part| part.split('.').next())
        .filter(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .unwrap_or("US")
        .to_ascii_lowercase()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appcast_version_and_notes_are_parsed_without_executing_markup() {
        let xml = r#"<item><description><![CDATA[<p>Fixes bugs</p>]]></description><enclosure sparkle:shortVersionString="2.4.1" url="https://example.test/app.zip" /></item>"#;
        assert_eq!(
            extract_attribute(xml, "sparkle:shortVersionString").as_deref(),
            Some("2.4.1")
        );
        assert_eq!(strip_markup("<p>Fixes <b>bugs</b></p>"), "Fixes bugs");
    }

    #[test]
    fn electron_yaml_accepts_only_scalar_top_level_values() {
        let yaml = "provider: github\nowner: demo\nrepo: app\n# ignored\n";
        let values = parse_simple_yaml(yaml);
        assert_eq!(values.get("owner").map(String::as_str), Some("demo"));
        assert_eq!(values.get("repo").map(String::as_str), Some("app"));
    }

    #[test]
    fn unsafe_update_urls_are_rejected() {
        assert!(safe_https_url("https://example.test/appcast.xml"));
        assert!(!safe_https_url("http://example.test/appcast.xml"));
        assert!(!safe_https_url("https://example.test/a b"));
        assert!(!safe_https_url("file:///tmp/appcast.xml"));
    }

    #[test]
    fn s3_metadata_parts_reject_traversal_and_url_syntax() {
        assert!(valid_s3_part("term.us-bucket"));
        assert!(!valid_s3_part("bucket/other"));
        assert!(valid_s3_path("mac-arm64/releases"));
        assert!(!valid_s3_path("../private"));
    }

    #[test]
    fn lookup_response_uses_camel_case_contract() {
        let response: LookupResponse = serde_json::from_str(
            r#"{"results":[{"trackId":42,"version":"3.0","releaseNotes":"New"}]}"#,
        )
        .unwrap();
        assert_eq!(response.results[0].track_id, 42);
    }
}
