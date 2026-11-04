//! Missing-driver detection — P1-T11.
//!
//! A known console chip is on the USB bus but the OS created no serial port for it →
//! the vendor driver is (most likely) missing. [`hints`] is PURE; [`list_usb_devices`]
//! is a thin nusb wrapper.

use std::collections::BTreeSet;

use super::usb_ids;
use crate::model::DriverHint;

/// USB identity of a device or of the device behind a serial port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsbDevice {
    pub vid: u16,
    pub pid: u16,
    pub serial_number: Option<String>,
}

/// All USB devices, or `None` when the OS listing failed (callers keep their previous
/// hints instead of flickering).
pub(crate) fn list_usb_devices() -> Option<Vec<UsbDevice>> {
    use nusb::MaybeFuture;
    match nusb::list_devices().wait() {
        Ok(devices) => Some(
            devices
                .map(|d| UsbDevice {
                    vid: d.vendor_id(),
                    pid: d.product_id(),
                    serial_number: d
                        .serial_number()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                })
                .collect(),
        ),
        Err(e) => {
            tracing::warn!("usb device listing failed: {e}");
            None
        }
    }
}

/// Serial numbers are compatible when either is unknown, or one is a prefix of the
/// other (Windows FTDI ports append the channel letter: `A10K3XYZ` → `A10K3XYZA`).
fn serials_compatible(device: Option<&str>, port: Option<&str>) -> bool {
    match (device, port) {
        (Some(d), Some(p)) => {
            let (d, p) = (d.to_ascii_uppercase(), p.to_ascii_uppercase());
            d.starts_with(&p) || p.starts_with(&d)
        }
        _ => true,
    }
}

/// One hint per known console chip (VID:PID) that has no matching serial port.
/// Ambiguous cases (serial number unknown on one side) count as "has a port", so a
/// hint is never shown for a working cable.
pub fn hints(devices: &[UsbDevice], port_usb: &[UsbDevice]) -> Vec<DriverHint> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for d in devices {
        let Some(chip) = usb_ids::chip(d.vid, d.pid) else {
            continue;
        };
        let has_port = port_usb.iter().any(|p| {
            p.vid == d.vid
                && p.pid == d.pid
                && serials_compatible(d.serial_number.as_deref(), p.serial_number.as_deref())
        });
        if !has_port && seen.insert((d.vid, d.pid)) {
            out.push(DriverHint { chip });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(vid: u16, pid: u16, sn: Option<&str>) -> UsbDevice {
        UsbDevice {
            vid,
            pid,
            serial_number: sn.map(Into::into),
        }
    }

    fn models(h: &[DriverHint]) -> Vec<&str> {
        h.iter().map(|h| h.chip.model.as_str()).collect()
    }

    #[test]
    fn known_chip_without_port_gets_hint() {
        let devices = [dev(0x067B, 0x2303, None), dev(0x05AC, 0x8262, None)];
        let h = hints(&devices, &[]);
        assert_eq!(models(&h), ["PL2303"]);
        assert!(h[0].chip.driver_url.is_some());
    }

    #[test]
    fn chip_with_port_gets_no_hint() {
        let devices = [dev(0x0403, 0x6001, Some("A10K3XYZ"))];
        assert!(hints(&devices, &[dev(0x0403, 0x6001, Some("A10K3XYZ"))]).is_empty());
        // Windows FTDI channel suffix.
        assert!(hints(&devices, &[dev(0x0403, 0x6001, Some("a10k3xyzA"))]).is_empty());
        // Serial unknown on the port side → assume it is this one.
        assert!(hints(&devices, &[dev(0x0403, 0x6001, None)]).is_empty());
    }

    #[test]
    fn second_identical_cable_without_driver_is_detected_by_serial() {
        let devices = [
            dev(0x0403, 0x6001, Some("AAAA1111")),
            dev(0x0403, 0x6001, Some("BBBB2222")),
        ];
        let h = hints(&devices, &[dev(0x0403, 0x6001, Some("AAAA1111"))]);
        assert_eq!(models(&h), ["FT232R"]);
    }

    #[test]
    fn hints_are_deduplicated_per_chip() {
        let devices = [dev(0x1A86, 0x7523, None), dev(0x1A86, 0x7523, None)];
        assert_eq!(hints(&devices, &[]).len(), 1);
    }

    #[test]
    fn unknown_devices_and_empty_inputs() {
        assert!(hints(&[], &[]).is_empty());
        assert!(hints(&[dev(0x046D, 0xC52B, None)], &[]).is_empty()); // Logitech receiver
        assert!(hints(&[dev(0x1A86, 0x8091, None)], &[]).is_empty()); // WCH hub
    }

    #[test]
    fn port_of_other_chip_does_not_cover() {
        let h = hints(&[dev(0x10C4, 0xEA60, None)], &[dev(0x0403, 0x6001, None)]);
        assert_eq!(models(&h), ["CP210x"]);
    }
}
