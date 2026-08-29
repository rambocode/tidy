// Docker: images and build cache reported by `docker system df -v`. Nothing
// here touches the filesystem; removal goes through the docker CLI itself so
// the daemon keeps its own consistency guarantees (an image that is still in
// use is refused by docker, not force-removed by us).

use crate::optimize::{run_bounded, CommandStatus};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

/// One image row from the daemon.
#[derive(Debug, Clone, Serialize)]
pub struct DockerImage {
    /// Full sha256 id (the removal key).
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size_kb: u64,
    /// Containers (any state) that use this image; 0 = unused.
    pub containers: u64,
    /// Days since the image was created (docker has no last-used stamp for
    /// images; "unused for N months" = no container AND created ≥ N months).
    pub age_days: Option<u64>,
    /// `<none>:<none>` leftovers from rebuilds.
    pub dangling: bool,
}

/// Everything the preview needs.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DockerCatalog {
    pub images: Vec<DockerImage>,
    /// Build-cache entries not in use by a running build, summed.
    pub build_cache_unused_kb: u64,
}

/// Locate the docker CLI: the app's PATH is minimal, so check PATH first and
/// then the usual install locations.
pub fn find_docker(home: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = PathBuf::from(dir).join("docker");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    [
        "/usr/local/bin/docker".to_string(),
        "/opt/homebrew/bin/docker".to_string(),
        format!("{home}/.docker/bin/docker"),
        "/Applications/Docker.app/Contents/Resources/bin/docker".to_string(),
        "/Applications/OrbStack.app/Contents/MacOS/xbin/docker".to_string(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// Parse docker's human sizes ("280MB", "1.2GB", "383kB", "-1.025e+08B").
fn parse_size_kb(text: &str) -> u64 {
    let text = text.trim();
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (num, unit) = text.split_at(split);
    let value: f64 = num.trim().parse().unwrap_or(0.0);
    if value <= 0.0 {
        return 0;
    }
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" | "kib" => 1e3,
        "mb" | "mib" => 1e6,
        "gb" | "gib" => 1e9,
        "tb" | "tib" => 1e12,
        _ => 1.0,
    };
    (value * mult / 1024.0) as u64
}

/// Days from a docker timestamp ("2026-08-29 19:44:49 +0800 +08") to now.
fn age_days(created_at: &str) -> Option<u64> {
    // Keep "YYYY-MM-DD HH:MM:SS +ZZZZ", drop the trailing zone name.
    let parts: Vec<&str> = created_at.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let stamp = format!("{} {} {}", parts[0], parts[1].split('.').next()?, parts[2]);
    let t = chrono::DateTime::parse_from_str(&stamp, "%Y-%m-%d %H:%M:%S %z").ok()?;
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds();
    Some(secs.max(0) as u64 / 86_400)
}

/// Turn `docker system df -v` JSON into a catalog.
fn parse_df(json: &str) -> Option<DockerCatalog> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let str_of = |o: &serde_json::Value, k: &str| o.get(k)?.as_str().map(str::to_string);
    let mut catalog = DockerCatalog::default();
    for img in v.get("Images")?.as_array()? {
        let Some(id) = str_of(img, "ID") else {
            continue;
        };
        let repository = str_of(img, "Repository").unwrap_or_default();
        let tag = str_of(img, "Tag").unwrap_or_default();
        catalog.images.push(DockerImage {
            dangling: repository == "<none>" || tag == "<none>",
            size_kb: parse_size_kb(&str_of(img, "Size").unwrap_or_default()),
            containers: str_of(img, "Containers")
                .and_then(|c| c.parse().ok())
                .unwrap_or(0),
            age_days: str_of(img, "CreatedAt").and_then(|c| age_days(&c)),
            id,
            repository,
            tag,
        });
    }
    if let Some(cache) = v.get("BuildCache").and_then(|c| c.as_array()) {
        for entry in cache {
            if str_of(entry, "InUse").as_deref() == Some("true") {
                continue;
            }
            catalog.build_cache_unused_kb +=
                parse_size_kb(&str_of(entry, "Size").unwrap_or_default());
        }
    }
    Some(catalog)
}

/// Query the daemon. None when docker is absent, not running, or the reply
/// is not understood — the preview simply has no Docker section then.
pub fn scan(home: &str) -> Option<DockerCatalog> {
    let docker = find_docker(home)?;
    let argv = [
        docker.to_string_lossy().into_owned(),
        "system".into(),
        "df".into(),
        "-v".into(),
        "--format".into(),
        "{{json .}}".into(),
    ];
    let result = run_bounded(&argv, Duration::from_secs(20));
    if result.status != CommandStatus::Success {
        return None;
    }
    parse_df(&result.output)
}

/// Remove one image by id (no --force: in-use images are refused by docker).
pub fn remove_image(docker: &std::path::Path, id: &str) -> Result<(), String> {
    let argv = [
        docker.to_string_lossy().into_owned(),
        "image".into(),
        "rm".into(),
        id.into(),
    ];
    let result = run_bounded(&argv, Duration::from_secs(60));
    if result.success() {
        Ok(())
    } else {
        Err(result.output)
    }
}

/// Drop every build-cache entry not in use.
pub fn prune_build_cache(docker: &std::path::Path) -> Result<(), String> {
    let argv = [
        docker.to_string_lossy().into_owned(),
        "builder".into(),
        "prune".into(),
        "--force".into(),
        "--all".into(),
    ];
    let result = run_bounded(&argv, Duration::from_secs(300));
    if result.success() {
        Ok(())
    } else {
        Err(result.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_like_docker_prints_them() {
        assert_eq!(parse_size_kb("280MB"), 273_437);
        assert_eq!(parse_size_kb("1.5GB"), 1_464_843);
        assert_eq!(parse_size_kb("383kB"), 374);
        assert_eq!(parse_size_kb("-1.025e+08B"), 0);
        assert_eq!(parse_size_kb("0B"), 0);
    }

    #[test]
    fn df_json_becomes_catalog() {
        let json = r#"{"Images":[
          {"ID":"sha256:aa","Repository":"app","Tag":"dev","Size":"536MB","Containers":"1","CreatedAt":"2020-01-01 00:00:00 +0000 UTC"},
          {"ID":"sha256:bb","Repository":"<none>","Tag":"<none>","Size":"10MB","Containers":"0","CreatedAt":"bogus"}],
          "BuildCache":[{"InUse":"false","Size":"136MB"},{"InUse":"true","Size":"1GB"}]}"#;
        let c = parse_df(json).unwrap();
        assert_eq!(c.images.len(), 2);
        assert_eq!(c.images[0].containers, 1);
        assert!(c.images[0].age_days.unwrap() > 365 * 5);
        assert!(c.images[1].dangling);
        assert_eq!(c.images[1].age_days, None);
        assert_eq!(c.build_cache_unused_kb, parse_size_kb("136MB"));
    }
}

#[cfg(test)]
mod real_machine {
    /// Manual: cargo test -p mole-ops real_docker_scan -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_docker_scan() {
        let home = std::env::var("HOME").unwrap();
        let Some(c) = super::scan(&home) else {
            eprintln!("docker unavailable");
            return;
        };
        let unused = c.images.iter().filter(|i| i.containers == 0).count();
        eprintln!(
            "{} images ({unused} unused), build cache unused {} KB",
            c.images.len(),
            c.build_cache_unused_kb
        );
        for i in c.images.iter().take(3) {
            eprintln!("{:?}", i);
        }
    }
}
