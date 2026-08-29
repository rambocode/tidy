// Read-only SMC (System Management Controller) client for fan telemetry.
// Reading SMC keys via the AppleSMC IOKit user client needs no privileges;
// only *writing* (fan override) does, so this module exposes reads only and
// fan control stays out until the signed privileged helper exists. Every
// failure path returns None/empty — the dashboard shows "—" rather than a
// guessed number.

use serde::Serialize;
use std::ffi::{c_char, c_void};
use std::sync::{Mutex, OnceLock};

/// One fan's telemetry, all in RPM. `actual_rpm` is always present; the
/// bounds/target keys are optional because some models omit them.
#[derive(Debug, Clone, Serialize)]
pub struct FanStatus {
    pub actual_rpm: f32,
    pub min_rpm: Option<f32>,
    pub max_rpm: Option<f32>,
    pub target_rpm: Option<f32>,
}

// AppleSMC user-client ABI (stable since Intel; unchanged on Apple Silicon).
const KERNEL_INDEX_SMC: u32 = 2;
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SmcVers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu_plimit: u32,
    gpu_plimit: u32,
    mem_plimit: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

/// The 80-byte in/out struct AppleSMC's kSMCHandleYPCEvent selector expects.
/// Field order and C padding must match the kernel's layout exactly.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SmcKeyData {
    key: u32,
    vers: SmcVers,
    p_limit: SmcPLimit,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IOServiceOpen(service: u32, owning_task: u32, conn_type: u32, connect: *mut u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOConnectCallStructMethod(
        connection: u32,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
}

extern "C" {
    fn mach_task_self() -> u32;
}

/// Process-lifetime AppleSMC connection, opened once and mutex-guarded
/// (the user client is not documented as concurrency-safe). None means the
/// service is absent (VM/CI) or open failed — callers then report no fans.
fn connection() -> Option<&'static Mutex<u32>> {
    static CONN: OnceLock<Option<Mutex<u32>>> = OnceLock::new();
    CONN.get_or_init(|| {
        // Matching dictionary is consumed by IOServiceGetMatchingService.
        let service =
            unsafe { IOServiceGetMatchingService(0, IOServiceMatching(c"AppleSMC".as_ptr())) };
        if service == 0 {
            return None;
        }
        let mut conn: u32 = 0;
        let rc = unsafe { IOServiceOpen(service, mach_task_self(), 0, &mut conn) };
        unsafe { IOObjectRelease(service) };
        (rc == 0 && conn != 0).then(|| Mutex::new(conn))
    })
    .as_ref()
}

/// One round-trip through the SMC selector; None on any kernel error.
fn call(conn: u32, input: &SmcKeyData) -> Option<SmcKeyData> {
    let mut out = SmcKeyData::default();
    let mut out_size = std::mem::size_of::<SmcKeyData>();
    let rc = unsafe {
        IOConnectCallStructMethod(
            conn,
            KERNEL_INDEX_SMC,
            (input as *const SmcKeyData).cast(),
            std::mem::size_of::<SmcKeyData>(),
            (&mut out as *mut SmcKeyData).cast(),
            &mut out_size,
        )
    };
    (rc == 0).then_some(out)
}

/// Four-character SMC key as the big-endian u32 the ABI uses.
fn fourcc(key: &str) -> Option<u32> {
    let bytes: [u8; 4] = key.as_bytes().try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// Read one key: (type fourcc, raw bytes). Two round-trips — key info for
/// size/type, then the payload — because size is not knowable up front.
fn read_key(conn: u32, key: &str) -> Option<(u32, Vec<u8>)> {
    let key = fourcc(key)?;
    let mut info_req = SmcKeyData {
        key,
        data8: SMC_CMD_READ_KEYINFO,
        ..Default::default()
    };
    let info = call(conn, &info_req)?;
    let size = info.key_info.data_size;
    if info.result != 0 || size == 0 || size > 32 {
        return None;
    }
    info_req.data8 = SMC_CMD_READ_BYTES;
    info_req.key_info.data_size = size;
    let data = call(conn, &info_req)?;
    if data.result != 0 {
        return None;
    }
    Some((
        info.key_info.data_type,
        data.bytes[..size as usize].to_vec(),
    ))
}

/// Decode an unsigned SMC integer (ui8/ui16/ui32, big-endian).
fn decode_uint(data_type: u32, bytes: &[u8]) -> Option<u64> {
    match (&data_type.to_be_bytes(), bytes.len()) {
        (b"ui8 ", 1) => Some(u64::from(bytes[0])),
        (b"ui16", 2) => Some(u64::from(u16::from_be_bytes([bytes[0], bytes[1]]))),
        (b"ui32", 4) => Some(u64::from(u32::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        _ => None,
    }
}

/// Decode an SMC number as f32. Apple Silicon reports fans as IEEE `flt `
/// (little-endian); older Intel firmware uses `fpe2` fixed-point (big-endian,
/// two fraction bits). Integer types pass through for robustness.
fn decode_float(data_type: u32, bytes: &[u8]) -> Option<f32> {
    match (&data_type.to_be_bytes(), bytes.len()) {
        (b"flt ", 4) => Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        (b"fpe2", 2) => Some(f32::from(u16::from_be_bytes([bytes[0], bytes[1]])) / 4.0),
        _ => decode_uint(data_type, bytes).map(|v| v as f32),
    }
}

/// Read one fan-scoped float key like "F0Ac", rejecting NaN/negatives so a
/// misdecoded value can never reach the UI.
fn read_fan_float(conn: u32, index: u64, suffix: &str) -> Option<f32> {
    let (ty, bytes) = read_key(conn, &format!("F{index}{suffix}"))?;
    decode_float(ty, &bytes).filter(|v| v.is_finite() && *v >= 0.0)
}

/// All fans' telemetry. Empty on fanless models (MacBook Air), inside VMs,
/// or on any SMC error — the caller renders that as "no fans", not zero RPM.
pub fn fans() -> Vec<FanStatus> {
    let Some(lock) = connection() else {
        return Vec::new();
    };
    let Ok(conn) = lock.lock() else {
        return Vec::new();
    };
    let conn = *conn;
    let count = read_key(conn, "FNum")
        .and_then(|(ty, bytes)| decode_uint(ty, &bytes))
        .unwrap_or(0)
        // Real Macs top out at 2 fans; a larger count means garbage data.
        .min(8);
    (0..count)
        .filter_map(|i| {
            Some(FanStatus {
                actual_rpm: read_fan_float(conn, i, "Ac")?,
                min_rpm: read_fan_float(conn, i, "Mn"),
                max_rpm: read_fan_float(conn, i, "Mx"),
                target_rpm: read_fan_float(conn, i, "Tg"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel ABI expects exactly 80 bytes; a padding drift here corrupts
    /// every field after it.
    #[test]
    fn smc_key_data_layout_is_80_bytes() {
        assert_eq!(std::mem::size_of::<SmcKeyData>(), 80);
    }

    #[test]
    fn decodes_flt_little_endian() {
        let ty = u32::from_be_bytes(*b"flt ");
        assert_eq!(decode_float(ty, &1234.5f32.to_le_bytes()), Some(1234.5));
    }

    #[test]
    fn decodes_fpe2_fixed_point() {
        let ty = u32::from_be_bytes(*b"fpe2");
        // 0x1388 = 5000 raw → 1250 RPM after the 2-bit shift.
        assert_eq!(decode_float(ty, &[0x13, 0x88]), Some(1250.0));
    }

    #[test]
    fn decodes_uints_and_rejects_mismatched_sizes() {
        assert_eq!(decode_uint(u32::from_be_bytes(*b"ui8 "), &[2]), Some(2));
        assert_eq!(
            decode_uint(u32::from_be_bytes(*b"ui16"), &[0x01, 0x00]),
            Some(256)
        );
        assert_eq!(decode_uint(u32::from_be_bytes(*b"ui8 "), &[1, 2]), None);
        assert_eq!(decode_float(u32::from_be_bytes(*b"ch8*"), &[1, 2]), None);
    }

    /// Smoke: must never panic, and every reported value must be sane —
    /// covers real hardware, fanless models, and SMC-less CI VMs alike.
    #[test]
    fn fans_fail_closed() {
        for fan in fans() {
            assert!(fan.actual_rpm.is_finite() && fan.actual_rpm >= 0.0);
        }
    }
}
