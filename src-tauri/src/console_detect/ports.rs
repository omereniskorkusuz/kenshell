//! Serial port enumeration and normalization — P1-T09.
//!
//! [`normalize`] is PURE (unit-tested with synthetic `SerialPortInfo` lists):
//! * macOS `/dev/tty.X` + `/dev/cu.X` become ONE entry (`path` = cu, `alt_path` = tty)
//! * built-in non-console ports (Bluetooth, `debug-console`, `wlan-debug`) are skipped
//! * stable key: `usb:<vid>:<pid>:<serial>` when the USB serial number is known,
//!   else `path:<path>`
//! * deterministic order: USB ports first, then by display name (natural: COM2 < COM10)

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

use super::driver_hint::UsbDevice;
use super::usb_ids;
use crate::model::{ConsolePort, PortMemory};

const MAC_CU: &str = "/dev/cu.";
const MAC_TTY: &str = "/dev/tty.";
const WIN_DEVICE_PREFIX: &str = r"\\.\";

/// A normalized port plus the raw USB identity (also for chips not in the table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedPort {
    pub port: ConsolePort,
    pub usb: Option<UsbDevice>,
}

/// OS port list. Errors are returned (not swallowed) so a transient enumeration
/// failure never looks like "every cable was unplugged".
pub(crate) fn enumerate() -> Result<Vec<SerialPortInfo>, String> {
    serialport::available_ports().map_err(|e| e.description)
}

/// Built-in ports that are never network consoles.
fn is_skipped(info: &SerialPortInfo) -> bool {
    if matches!(info.port_type, SerialPortType::BluetoothPort) {
        return true;
    }
    let lower = info.port_name.to_ascii_lowercase();
    ["bluetooth", "debug-console", "wlan-debug"]
        .iter()
        .any(|s| lower.contains(s))
}

/// `(is_cu, stem)` for macOS `/dev/cu.X` / `/dev/tty.X`.
fn mac_twin(path: &str) -> Option<(bool, &str)> {
    if let Some(stem) = path.strip_prefix(MAC_CU) {
        (!stem.is_empty()).then_some((true, stem))
    } else if let Some(stem) = path.strip_prefix(MAC_TTY) {
        (!stem.is_empty()).then_some((false, stem))
    } else {
        None
    }
}

/// `COM4` for `\\.\COM4` / `com4`; other paths unchanged.
fn normalize_com(path: &str) -> Option<String> {
    let name = path.strip_prefix(WIN_DEVICE_PREFIX).unwrap_or(path);
    let is_com = name.len() > 3
        && name[..3].eq_ignore_ascii_case("COM")
        && name[3..].bytes().all(|b| b.is_ascii_digit());
    is_com.then(|| name.to_ascii_uppercase())
}

/// Short human label: `COM4`, `usbserial-A10K3XYZ`, else the file name.
pub fn display_name_for(path: &str) -> String {
    if let Some(com) = normalize_com(path) {
        return com;
    }
    if let Some((_, stem)) = mac_twin(path) {
        return stem.to_owned();
    }
    let name = path
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path);
    if name.is_empty() {
        path.to_owned()
    } else {
        name.to_owned()
    }
}

/// Same port? Windows `COMx` names compare case-insensitively (with or without `\\.\`).
pub fn paths_equal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (normalize_com(a), normalize_com(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn usb_info(info: &SerialPortInfo) -> Option<&UsbPortInfo> {
    match &info.port_type {
        SerialPortType::UsbPort(u) => Some(u),
        _ => None,
    }
}

#[derive(Default)]
struct Group<'a> {
    cu: Option<&'a SerialPortInfo>,
    tty: Option<&'a SerialPortInfo>,
    other: Option<&'a SerialPortInfo>,
}

/// Merges, filters, keys and sorts the OS port list. `memory(key)` supplies the
/// remembered baud / last device (lookup errors are the caller's to swallow).
pub(crate) fn normalize(
    infos: &[SerialPortInfo],
    memory: &dyn Fn(&str) -> Option<PortMemory>,
) -> Vec<ScannedPort> {
    let mut groups: BTreeMap<String, Group<'_>> = BTreeMap::new();
    for info in infos.iter().filter(|i| !is_skipped(i)) {
        let name = info.port_name.as_str();
        if name.trim().is_empty() {
            continue;
        }
        match mac_twin(name) {
            Some((is_cu, stem)) => {
                let g = groups.entry(format!("mac:{stem}")).or_default();
                let slot = if is_cu { &mut g.cu } else { &mut g.tty };
                slot.get_or_insert(info);
            }
            None => {
                let id = normalize_com(name).unwrap_or_else(|| name.to_owned());
                groups
                    .entry(format!("path:{id}"))
                    .or_default()
                    .other
                    .get_or_insert(info);
            }
        }
    }

    let mut out: Vec<ScannedPort> = groups
        .into_values()
        .filter_map(|g| {
            let primary = g.cu.or(g.other).or(g.tty)?;
            let alt_path = g.cu.and(g.tty).map(|t| t.port_name.clone());
            // USB details may only be attached to one of the twins.
            let usb = usb_info(primary)
                .or_else(|| g.tty.and_then(usb_info))
                .or_else(|| g.cu.and_then(usb_info));
            Some(build(primary, alt_path, usb))
        })
        .collect();

    disambiguate_keys(&mut out);
    for sp in &mut out {
        if let Some(mem) = memory(&sp.port.key) {
            sp.port.remembered_baud = (mem.baud > 0).then_some(mem.baud);
            sp.port.last_device = mem.last_device;
        }
    }
    out.sort_by(|a, b| {
        a.usb
            .is_none()
            .cmp(&b.usb.is_none())
            .then_with(|| natural_cmp(&a.port.display_name, &b.port.display_name))
            .then_with(|| a.port.path.cmp(&b.port.path))
    });
    out
}

fn build(
    info: &SerialPortInfo,
    alt_path: Option<String>,
    usb: Option<&UsbPortInfo>,
) -> ScannedPort {
    let path = info.port_name.clone();
    let display_name = display_name_for(&path);
    let serial_number = usb.and_then(|u| clean(&u.serial_number));
    let key = match (usb, &serial_number) {
        (Some(u), Some(sn)) => format!("usb:{:04x}:{:04x}:{sn}", u.vid, u.pid),
        _ => format!("path:{path}"),
    };
    let port = ConsolePort {
        key,
        path,
        alt_path,
        display_name,
        chip: usb.and_then(|u| usb_ids::chip(u.vid, u.pid)),
        manufacturer: usb.and_then(|u| clean(&u.manufacturer)),
        product: usb.and_then(|u| clean(&u.product)),
        serial_number: serial_number.clone(),
        remembered_baud: None,
        last_device: None,
    };
    ScannedPort {
        port,
        usb: usb.map(|u| UsbDevice {
            vid: u.vid,
            pid: u.pid,
            serial_number,
        }),
    }
}

/// Multi-channel chips (FT2232/FT4232, CP2105/CP2108) report one serial number for
/// several ports: make those keys unique (and stable) with the port's display name.
fn disambiguate_keys(ports: &mut [ScannedPort]) {
    let mut count: HashMap<String, usize> = HashMap::new();
    for p in ports.iter() {
        *count.entry(p.port.key.clone()).or_default() += 1;
    }
    for p in ports.iter_mut() {
        if count.get(&p.port.key).copied().unwrap_or(0) > 1 {
            p.port.key = format!("{}:{}", p.port.key, p.port.display_name);
        }
    }
}

/// Natural ordering: digit runs compare numerically (`COM2` < `COM10`), text
/// case-insensitively.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(cx), Some(cy)) if cx.is_ascii_digit() && cy.is_ascii_digit() => {
                let nx = x.iter().take_while(|c| c.is_ascii_digit()).count();
                let ny = y.iter().take_while(|c| c.is_ascii_digit()).count();
                let (dx, dy) = (trim_zeros(&x[..nx]), trim_zeros(&y[..ny]));
                let ord = dx.len().cmp(&dy.len()).then_with(|| dx.cmp(dy));
                if ord != Ordering::Equal {
                    return ord;
                }
                x = &x[nx..];
                y = &y[ny..];
            }
            (Some(cx), Some(cy)) => {
                let ord = cx.to_ascii_lowercase().cmp(&cy.to_ascii_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x = &x[1..];
                y = &y[1..];
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let n = d.iter().take_while(|&&c| c == b'0').count();
    &d[n..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(name: &str, vid: u16, pid: u16, sn: Option<&str>) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.into(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid,
                serial_number: sn.map(Into::into),
                manufacturer: Some("FTDI".into()),
                product: Some(" FT232R USB UART ".into()),
            }),
        }
    }

    fn plain(name: &str, kind: SerialPortType) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.into(),
            port_type: kind,
        }
    }

    fn no_mem(_: &str) -> Option<PortMemory> {
        None
    }

    fn ports(infos: &[SerialPortInfo]) -> Vec<ConsolePort> {
        normalize(infos, &no_mem)
            .into_iter()
            .map(|s| s.port)
            .collect()
    }

    #[test]
    fn macos_twins_merge_into_cu_entry() {
        let list = [
            usb(
                "/dev/tty.usbserial-A10K3XYZ",
                0x0403,
                0x6001,
                Some("A10K3XYZ"),
            ),
            plain("/dev/tty.Bluetooth-Incoming-Port", SerialPortType::Unknown),
            usb(
                "/dev/cu.usbserial-A10K3XYZ",
                0x0403,
                0x6001,
                Some("A10K3XYZ"),
            ),
            plain("/dev/cu.Bluetooth-Incoming-Port", SerialPortType::Unknown),
            plain("/dev/cu.debug-console", SerialPortType::Unknown),
            plain("/dev/tty.debug-console", SerialPortType::Unknown),
            plain("/dev/cu.wlan-debug", SerialPortType::Unknown),
            plain("/dev/cu.Kens-Headphones", SerialPortType::BluetoothPort),
        ];
        let p = ports(&list);
        assert_eq!(p.len(), 1, "{p:#?}");
        let p = &p[0];
        assert_eq!(p.path, "/dev/cu.usbserial-A10K3XYZ");
        assert_eq!(p.alt_path.as_deref(), Some("/dev/tty.usbserial-A10K3XYZ"));
        assert_eq!(p.display_name, "usbserial-A10K3XYZ");
        assert_eq!(p.key, "usb:0403:6001:A10K3XYZ");
        assert_eq!(p.chip.as_ref().unwrap().model, "FT232R");
        assert_eq!(p.manufacturer.as_deref(), Some("FTDI"));
        assert_eq!(p.product.as_deref(), Some("FT232R USB UART"));
        assert_eq!(p.serial_number.as_deref(), Some("A10K3XYZ"));
    }

    #[test]
    fn lone_tty_is_kept_and_usb_info_taken_from_either_twin() {
        let p = ports(&[plain("/dev/tty.usbmodem14201", SerialPortType::Unknown)]);
        assert_eq!(p[0].path, "/dev/tty.usbmodem14201");
        assert_eq!(p[0].alt_path, None);

        let p = ports(&[
            plain("/dev/cu.usbserial-1420", SerialPortType::Unknown),
            usb("/dev/tty.usbserial-1420", 0x1A86, 0x7523, None),
        ]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].path, "/dev/cu.usbserial-1420");
        assert_eq!(p[0].chip.as_ref().unwrap().model, "CH340");
        // No USB serial number → path key.
        assert_eq!(p[0].key, "path:/dev/cu.usbserial-1420");
    }

    #[test]
    fn windows_names_and_natural_order() {
        let list = [
            plain("COM1", SerialPortType::PciPort),
            usb("COM10", 0x067B, 0x2303, None),
            usb("COM4", 0x0403, 0x6001, Some("A10K3XYZ")),
            usb("COM2", 0x10C4, 0xEA60, Some("0001")),
        ];
        let p = ports(&list);
        let names: Vec<_> = p.iter().map(|p| p.display_name.as_str()).collect();
        assert_eq!(names, ["COM2", "COM4", "COM10", "COM1"]);
        assert_eq!(p[1].key, "usb:0403:6001:A10K3XYZ");
        assert_eq!(p[2].key, "path:COM10");
        assert_eq!(p[3].key, "path:COM1");
        assert_eq!(p[3].chip, None);
    }

    #[test]
    fn duplicate_windows_entries_collapse() {
        let p = ports(&[
            usb("COM4", 0x0403, 0x6001, Some("X")),
            usb(r"\\.\com4", 0x0403, 0x6001, Some("X")),
        ]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].path, "COM4");
    }

    #[test]
    fn multi_channel_serials_get_unique_keys() {
        let p = ports(&[
            usb("/dev/cu.usbserial-FT1234A", 0x0403, 0x6010, Some("FT1234")),
            usb("/dev/cu.usbserial-FT1234B", 0x0403, 0x6010, Some("FT1234")),
        ]);
        assert_eq!(p[0].key, "usb:0403:6010:FT1234:usbserial-FT1234A");
        assert_eq!(p[1].key, "usb:0403:6010:FT1234:usbserial-FT1234B");
    }

    #[test]
    fn blank_serial_number_is_treated_as_missing() {
        let p = ports(&[usb("COM5", 0x1A86, 0x7523, Some("  "))]);
        assert_eq!(p[0].key, "path:COM5");
        assert_eq!(p[0].serial_number, None);
    }

    #[test]
    fn memory_fills_remembered_baud() {
        let mem = |key: &str| {
            (key == "usb:0403:6001:A10K3XYZ").then(|| PortMemory {
                key: key.into(),
                path: "COM4".into(),
                vid: Some(0x0403),
                pid: Some(0x6001),
                serial_number: Some("A10K3XYZ".into()),
                baud: 115200,
                last_device: Some("core-sw-01".into()),
                updated_at: 1,
            })
        };
        let out = normalize(
            &[
                usb("COM4", 0x0403, 0x6001, Some("A10K3XYZ")),
                usb("COM7", 0x1A86, 0x7523, None),
            ],
            &mem,
        );
        assert_eq!(out[0].port.remembered_baud, Some(115200));
        assert_eq!(out[0].port.last_device.as_deref(), Some("core-sw-01"));
        assert_eq!(out[1].port.remembered_baud, None);
        assert_eq!(
            out[0].usb,
            Some(UsbDevice {
                vid: 0x0403,
                pid: 0x6001,
                serial_number: Some("A10K3XYZ".into())
            })
        );
    }

    #[test]
    fn empty_and_all_skipped() {
        assert!(ports(&[]).is_empty());
        assert!(
            ports(&[plain(
                "/dev/cu.Bluetooth-Incoming-Port",
                SerialPortType::Unknown
            )])
            .is_empty()
        );
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name_for("COM4"), "COM4");
        assert_eq!(display_name_for(r"\\.\COM12"), "COM12");
        assert_eq!(display_name_for("/dev/cu.usbserial-A1"), "usbserial-A1");
        assert_eq!(display_name_for("/dev/tty.usbserial-A1"), "usbserial-A1");
        assert_eq!(display_name_for("/dev/ttyUSB0"), "ttyUSB0");
        assert_eq!(display_name_for("ttyS1"), "ttyS1");
        assert_eq!(display_name_for(""), "");
    }

    #[test]
    fn path_equality() {
        assert!(paths_equal("COM4", r"\\.\com4"));
        assert!(!paths_equal("COM4", "COM40"));
        assert!(paths_equal("/dev/cu.x", "/dev/cu.x"));
        assert!(!paths_equal("/dev/cu.X", "/dev/cu.x"));
    }

    #[test]
    fn natural_ordering() {
        assert_eq!(natural_cmp("COM2", "COM10"), Ordering::Less);
        assert_eq!(natural_cmp("COM10", "COM9"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("x01", "x1"), Ordering::Less); // tie broken bytewise
        assert_eq!(natural_cmp("usbserial-1", "usbserial-1"), Ordering::Equal);
    }
}
