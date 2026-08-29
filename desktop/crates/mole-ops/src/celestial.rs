//! Persistent celestial catalog used by the optional clean-page explorer.
//!
//! The database always contains a small, offline-safe Solar System seed. When
//! the NASA Exoplanet Archive is reachable, its current `PSCompPars` rows are
//! refreshed atomically and become available on subsequent offline launches.

use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ARCHIVE_URL: &str = "https://exoplanetarchive.ipac.caltech.edu/TAP/sync";
const ARCHIVE_PAGE: &str = "https://exoplanetarchive.ipac.caltech.edu/";
const ARCHIVE_QUERY: &str =
    "select pl_name,hostname,pl_rade,st_rad,sy_dist,disc_year,discoverymethod,ra,dec from pscomppars";
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const EARTH_RADIUS_KM: f64 = 6_371.0;
const PARSEC_TO_LIGHT_YEAR: f64 = 3.261_56;

/// Complete payload returned to the WebView in one IPC call. A few thousand
/// compact rows are cheaper and smoother than issuing IPC during every wheel
/// event; viewport culling and level-of-detail selection stay in the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CelestialCatalog {
    pub objects: Vec<CelestialObject>,
    pub updated_at: Option<i64>,
    pub archive_complete: bool,
    pub source_name: &'static str,
    pub source_url: &'static str,
    pub warning: Option<String>,
}

/// One real object stored in SQLite. `map_x` wraps at -1/1 and `map_y` spans
/// -1/1; they are display coordinates derived from catalog RA/Dec, not a claim
/// that very different physical distances share one Euclidean plane.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CelestialObject {
    pub id: String,
    pub name: String,
    pub body_type: String,
    pub map_x: f64,
    pub map_y: f64,
    pub distance_ly: Option<f64>,
    pub radius_km: Option<f64>,
    pub discovery_year: Option<i32>,
    pub discovery_method: Option<String>,
    pub host_name: Option<String>,
    pub summary_zh: String,
    pub summary_en: String,
    pub source_url: String,
    pub prominence: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("cannot prepare celestial catalog directory: {0}")]
    Directory(#[source] std::io::Error),
    #[error("celestial SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Deserialize)]
struct ArchiveRow {
    pl_name: String,
    hostname: Option<String>,
    pl_rade: Option<f64>,
    st_rad: Option<f64>,
    sy_dist: Option<f64>,
    disc_year: Option<i32>,
    discoverymethod: Option<String>,
    ra: Option<f64>,
    dec: Option<f64>,
}

struct SolarSeed {
    id: &'static str,
    name: &'static str,
    body_type: &'static str,
    map_x: f64,
    map_y: f64,
    distance_ly: f64,
    radius_km: f64,
    summary_zh: &'static str,
    summary_en: &'static str,
    prominence: i32,
}

const SOLAR_SEEDS: &[SolarSeed] = &[
    SolarSeed { id: "sun", name: "Sun", body_type: "star", map_x: -0.11, map_y: -0.04, distance_ly: 0.000_015_81, radius_km: 700_000.0, summary_zh: "太阳是太阳系中心的恒星，提供驱动地球气候与生命所需的能量。", summary_en: "The Sun is the star at the center of the Solar System and supplies the energy that drives Earth's climate and life.", prominence: 100 },
    SolarSeed { id: "mercury", name: "Mercury", body_type: "planet", map_x: -0.08, map_y: -0.02, distance_ly: 0.000_006_4, radius_km: 2_440.0, summary_zh: "水星是最靠近太阳、也是太阳系中最小的行星。", summary_en: "Mercury is the closest planet to the Sun and the smallest planet in the Solar System.", prominence: 94 },
    SolarSeed { id: "venus", name: "Venus", body_type: "planet", map_x: -0.04, map_y: 0.02, distance_ly: 0.000_004_3, radius_km: 6_052.0, summary_zh: "金星大小接近地球，浓厚的二氧化碳大气造成了极端温室效应。", summary_en: "Venus is close to Earth in size, with a dense carbon-dioxide atmosphere that drives an extreme greenhouse effect.", prominence: 95 },
    SolarSeed { id: "earth", name: "Earth", body_type: "planet", map_x: 0.0, map_y: 0.0, distance_ly: 0.0, radius_km: 6_371.0, summary_zh: "地球是太阳系第三颗行星，也是目前唯一确认存在生命的天体。", summary_en: "Earth is the third planet from the Sun and the only world currently known to support life.", prominence: 100 },
    SolarSeed { id: "moon", name: "Moon", body_type: "moon", map_x: 0.018, map_y: -0.014, distance_ly: 0.000_000_040_6, radius_km: 1_737.4, summary_zh: "月球是地球唯一的天然卫星，其引力是地球潮汐的主要驱动力。", summary_en: "The Moon is Earth's only natural satellite and its gravity is the main driver of Earth's tides.", prominence: 96 },
    SolarSeed { id: "mars", name: "Mars", body_type: "planet", map_x: 0.05, map_y: -0.025, distance_ly: 0.000_007_6, radius_km: 3_390.0, summary_zh: "火星是一颗寒冷的岩质行星，地表保留着远古河流与湖泊的证据。", summary_en: "Mars is a cold rocky planet whose surface preserves evidence of ancient rivers and lakes.", prominence: 95 },
    SolarSeed { id: "jupiter", name: "Jupiter", body_type: "planet", map_x: 0.10, map_y: 0.025, distance_ly: 0.000_082, radius_km: 69_911.0, summary_zh: "木星是太阳系最大的行星，拥有强磁场和规模庞大的卫星系统。", summary_en: "Jupiter is the Solar System's largest planet, with a powerful magnetic field and a large system of moons.", prominence: 98 },
    SolarSeed { id: "saturn", name: "Saturn", body_type: "planet", map_x: 0.15, map_y: -0.035, distance_ly: 0.000_15, radius_km: 58_232.0, summary_zh: "土星是以明亮冰环著称的气态巨行星。", summary_en: "Saturn is a gas giant distinguished by its bright system of icy rings.", prominence: 98 },
    SolarSeed { id: "uranus", name: "Uranus", body_type: "planet", map_x: 0.20, map_y: 0.035, distance_ly: 0.000_30, radius_km: 25_362.0, summary_zh: "天王星是一颗冰巨星，自转轴几乎横躺在轨道平面上。", summary_en: "Uranus is an ice giant whose rotation axis lies almost sideways in its orbital plane.", prominence: 94 },
    SolarSeed { id: "neptune", name: "Neptune", body_type: "planet", map_x: 0.25, map_y: -0.02, distance_ly: 0.000_48, radius_km: 24_622.0, summary_zh: "海王星是最外侧的主要行星，拥有太阳系中观测到的最快行星风。", summary_en: "Neptune is the outermost major planet and has the fastest observed planetary winds in the Solar System.", prominence: 94 },
    SolarSeed { id: "ceres", name: "Ceres", body_type: "dwarf_planet", map_x: 0.075, map_y: 0.055, distance_ly: 0.000_044, radius_km: 469.7, summary_zh: "谷神星是火星与木星之间小行星带中最大的天体，也是矮行星。", summary_en: "Ceres is the largest object in the asteroid belt between Mars and Jupiter and is classified as a dwarf planet.", prominence: 88 },
    SolarSeed { id: "pluto", name: "Pluto", body_type: "dwarf_planet", map_x: 0.29, map_y: 0.04, distance_ly: 0.000_63, radius_km: 1_188.3, summary_zh: "冥王星是柯伊伯带中的矮行星，拥有包括冥卫一在内的五颗已知卫星。", summary_en: "Pluto is a dwarf planet in the Kuiper Belt with five known moons, including Charon.", prominence: 92 },
    SolarSeed { id: "eris", name: "Eris", body_type: "dwarf_planet", map_x: 0.34, map_y: -0.05, distance_ly: 0.001_0, radius_km: 1_163.0, summary_zh: "阋神星是位于海王星外侧离散盘中的矮行星。", summary_en: "Eris is a dwarf planet in the scattered disc beyond Neptune.", prominence: 84 },
];

/// Open (or create) the app-owned SQLite database, opportunistically refresh
/// the remote archive, and return every locally known object.
pub fn catalog(database_path: &Path) -> Result<CelestialCatalog, CatalogError> {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent).map_err(CatalogError::Directory)?;
    }
    let mut connection = Connection::open(database_path)?;
    migrate(&connection)?;
    seed_solar_system(&mut connection)?;

    let updated_at = meta_i64(&connection, "archive_updated_at")?;
    let cached_archive_count = archive_count(&connection)?;
    let stale = updated_at
        .and_then(|stamp| now_unix().checked_sub(stamp))
        .is_none_or(|age| age as u64 >= REFRESH_AFTER.as_secs());
    let warning = if stale {
        match fetch_archive().and_then(|rows| replace_archive(&mut connection, &rows)) {
            Ok(()) => None,
            Err(error) => Some(format!("NASA catalog refresh unavailable: {error}")),
        }
    } else {
        None
    };

    let final_count = archive_count(&connection)?;
    Ok(CelestialCatalog {
        objects: read_all(&connection)?,
        updated_at: meta_i64(&connection, "archive_updated_at")?,
        archive_complete: final_count > 0 || cached_archive_count > 0,
        source_name: "NASA Exoplanet Archive PSCompPars",
        source_url: ARCHIVE_PAGE,
        warning,
    })
}

fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS celestial_objects (
           id TEXT PRIMARY KEY,
           name TEXT NOT NULL,
           body_type TEXT NOT NULL,
           map_x REAL NOT NULL,
           map_y REAL NOT NULL,
           distance_ly REAL,
           radius_km REAL,
           discovery_year INTEGER,
           discovery_method TEXT,
           host_name TEXT,
           summary_zh TEXT NOT NULL,
           summary_en TEXT NOT NULL,
           source TEXT NOT NULL,
           source_url TEXT NOT NULL,
           prominence INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS celestial_objects_map
           ON celestial_objects(map_x, map_y, prominence);
         CREATE TABLE IF NOT EXISTS celestial_meta (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );",
    )
}

fn seed_solar_system(connection: &mut Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    for seed in SOLAR_SEEDS {
        transaction.execute(
            "INSERT INTO celestial_objects (
               id, name, body_type, map_x, map_y, distance_ly, radius_km,
               summary_zh, summary_en, source, source_url, prominence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'nasa-solar-system',
                       'https://science.nasa.gov/solar-system/', ?10)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, body_type=excluded.body_type,
               map_x=excluded.map_x, map_y=excluded.map_y,
               distance_ly=excluded.distance_ly, radius_km=excluded.radius_km,
               summary_zh=excluded.summary_zh, summary_en=excluded.summary_en,
               source=excluded.source, source_url=excluded.source_url,
               prominence=excluded.prominence",
            params![
                seed.id,
                seed.name,
                seed.body_type,
                seed.map_x,
                seed.map_y,
                seed.distance_ly,
                seed.radius_km,
                seed.summary_zh,
                seed.summary_en,
                seed.prominence,
            ],
        )?;
    }
    transaction.commit()
}

fn fetch_archive() -> Result<Vec<ArchiveRow>, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?
        .get(ARCHIVE_URL)
        .query(&[("query", ARCHIVE_QUERY), ("format", "json")])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json::<Vec<ArchiveRow>>()
        .map_err(|error| error.to_string())
        .and_then(|rows| {
            if rows.is_empty() {
                Err("archive returned no rows".to_string())
            } else {
                Ok(rows)
            }
        })
}

fn replace_archive(connection: &mut Connection, rows: &[ArchiveRow]) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM celestial_objects WHERE source = 'nasa-exoplanet-archive'",
            [],
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        insert_archive_row(&transaction, row).map_err(|error| error.to_string())?;
    }
    transaction
        .execute(
            "INSERT INTO celestial_meta(key, value) VALUES('archive_updated_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [now_unix().to_string()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn insert_archive_row(transaction: &Transaction<'_>, row: &ArchiveRow) -> rusqlite::Result<()> {
    let id = format!("exo:{}", row.pl_name);
    let (map_x, map_y) = catalog_position(row);
    let distance_ly = row.sy_dist.map(|parsecs| parsecs * PARSEC_TO_LIGHT_YEAR);
    let radius_km = row.pl_rade.map(|radius| radius * EARTH_RADIUS_KM);
    let method = row
        .discoverymethod
        .as_deref()
        .unwrap_or("an unspecified method");
    let host = row.hostname.as_deref().unwrap_or("its host star");
    let year_en = row
        .disc_year
        .map(|year| format!(" in {year}"))
        .unwrap_or_default();
    let year_zh = row
        .disc_year
        .map(|year| format!("于 {year} 年"))
        .unwrap_or_else(|| "已被".to_string());
    let summary_en = format!(
        "{} is a confirmed exoplanet orbiting {}. It was discovered{} using {}.",
        row.pl_name, host, year_en, method
    );
    let summary_zh = format!(
        "{} 是围绕 {} 运行的确认系外行星，{}通过 {} 方法发现。",
        row.pl_name, host, year_zh, method
    );
    let prominence = prominence(distance_ly, radius_km);
    transaction.execute(
        "INSERT INTO celestial_objects (
           id, name, body_type, map_x, map_y, distance_ly, radius_km,
           discovery_year, discovery_method, host_name, summary_zh, summary_en,
           source, source_url, prominence
         ) VALUES (?1, ?2, 'exoplanet', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                   'nasa-exoplanet-archive', ?12, ?13)",
        params![
            id,
            row.pl_name,
            map_x,
            map_y,
            distance_ly,
            radius_km,
            row.disc_year,
            row.discoverymethod,
            row.hostname,
            summary_zh,
            summary_en,
            ARCHIVE_PAGE,
            prominence,
        ],
    )?;
    insert_host_star(transaction, row, distance_ly)?;
    Ok(())
}

/// `PSCompPars` also carries the host star's catalog position and measured
/// radius. Store one deduplicated host row beside its planets so the explorer
/// represents both sides of every confirmed planetary system.
fn insert_host_star(
    transaction: &Transaction<'_>,
    row: &ArchiveRow,
    distance_ly: Option<f64>,
) -> rusqlite::Result<()> {
    let Some(host) = row.hostname.as_deref() else {
        return Ok(());
    };
    let map_x = row.ra.map(|ra| ra / 180.0 - 1.0).unwrap_or(0.0);
    let map_y = row.dec.map(|dec| -dec / 90.0).unwrap_or(0.0);
    let radius_km = row.st_rad.map(|solar_radii| solar_radii * 695_700.0);
    let summary_en =
        format!("{host} is a cataloged star known to host at least one confirmed exoplanet.");
    let summary_zh = format!("{host} 是一颗已确认拥有至少一颗系外行星的恒星。");
    transaction.execute(
        "INSERT OR IGNORE INTO celestial_objects (
           id, name, body_type, map_x, map_y, distance_ly, radius_km,
           summary_zh, summary_en, source, source_url, prominence
         ) VALUES (?1, ?2, 'star', ?3, ?4, ?5, ?6, ?7, ?8,
                   'nasa-exoplanet-archive', ?9, ?10)",
        params![
            format!("star:{host}"),
            host,
            map_x,
            map_y,
            distance_ly,
            radius_km,
            summary_zh,
            summary_en,
            ARCHIVE_PAGE,
            (prominence(distance_ly, radius_km) + 5).clamp(8, 80),
        ],
    )?;
    Ok(())
}

fn catalog_position(row: &ArchiveRow) -> (f64, f64) {
    let ra = row
        .ra
        .unwrap_or_else(|| (stable_hash(&row.pl_name) % 360_000) as f64 / 1_000.0);
    let dec = row.dec.unwrap_or(0.0);
    // Planets in one host system share sky coordinates. A minute deterministic
    // offset fans them apart only after deep zoom, while preserving the host's
    // actual catalog position at overview scale.
    let hash = stable_hash(&row.pl_name);
    let angle = (hash % 6_283) as f64 / 1_000.0;
    let jitter = 0.000_35 + ((hash >> 16) % 100) as f64 / 200_000.0;
    let x = (ra / 180.0 - 1.0 + angle.cos() * jitter).clamp(-1.0, 1.0);
    let y = (-dec / 90.0 + angle.sin() * jitter).clamp(-1.0, 1.0);
    (x, y)
}

fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(14_695_981_039_346_656_037, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
    })
}

fn prominence(distance_ly: Option<f64>, radius_km: Option<f64>) -> i32 {
    let nearby = distance_ly
        .filter(|distance| *distance > 0.0)
        .map(|distance| (58.0 - distance.log10() * 12.0).round() as i32)
        .unwrap_or(18);
    let measured = if radius_km.is_some() { 8 } else { 0 };
    (nearby + measured).clamp(8, 76)
}

fn read_all(connection: &Connection) -> rusqlite::Result<Vec<CelestialObject>> {
    let mut statement = connection.prepare(
        "SELECT id, name, body_type, map_x, map_y, distance_ly, radius_km,
                discovery_year, discovery_method, host_name, summary_zh,
                summary_en, source_url, prominence
         FROM celestial_objects
         ORDER BY prominence DESC, distance_ly ASC, name COLLATE NOCASE ASC",
    )?;
    let objects = statement
        .query_map([], |row| {
            Ok(CelestialObject {
                id: row.get(0)?,
                name: row.get(1)?,
                body_type: row.get(2)?,
                map_x: row.get(3)?,
                map_y: row.get(4)?,
                distance_ly: row.get(5)?,
                radius_km: row.get(6)?,
                discovery_year: row.get(7)?,
                discovery_method: row.get(8)?,
                host_name: row.get(9)?,
                summary_zh: row.get(10)?,
                summary_en: row.get(11)?,
                source_url: row.get(12)?,
                prominence: row.get(13)?,
            })
        })?
        .collect();
    objects
}

fn archive_count(connection: &Connection) -> rusqlite::Result<usize> {
    connection.query_row(
        "SELECT COUNT(*) FROM celestial_objects WHERE source = 'nasa-exoplanet-archive'",
        [],
        |row| row.get(0),
    )
}

fn meta_i64(connection: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    connection
        .query_row(
            "SELECT value FROM celestial_meta WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|value| value.and_then(|text| text.parse().ok()))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar_seed_is_idempotent_and_keeps_physical_radii() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        seed_solar_system(&mut connection).unwrap();
        seed_solar_system(&mut connection).unwrap();
        let objects = read_all(&connection).unwrap();
        assert_eq!(objects.len(), SOLAR_SEEDS.len());
        assert_eq!(
            objects
                .iter()
                .find(|item| item.id == "earth")
                .unwrap()
                .radius_km,
            Some(6_371.0)
        );
    }

    #[test]
    fn archive_replace_is_atomic_and_deduplicated_by_name() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        let row = ArchiveRow {
            pl_name: "Test b".to_string(),
            hostname: Some("Test".to_string()),
            pl_rade: Some(2.0),
            st_rad: Some(1.0),
            sy_dist: Some(10.0),
            disc_year: Some(2026),
            discoverymethod: Some("Transit".to_string()),
            ra: Some(180.0),
            dec: Some(0.0),
        };
        replace_archive(&mut connection, &[row]).unwrap();
        assert_eq!(archive_count(&connection).unwrap(), 2);
        let object = read_all(&connection)
            .unwrap()
            .into_iter()
            .find(|object| object.id == "exo:Test b")
            .unwrap();
        assert_eq!(object.radius_km, Some(EARTH_RADIUS_KM * 2.0));
        assert!((object.distance_ly.unwrap() - 32.6156).abs() < 0.0001);
    }
}
