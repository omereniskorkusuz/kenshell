//! Known USB-serial console chips: VID:PID → [`ChipInfo`] — P1-T09.
//!
//! Small static table; [`chip`] picks the driver page for the OS the app was built
//! for. Vendors whose VID is used almost exclusively for USB-serial bridges (FTDI,
//! Prolific) get a generic fallback entry for unknown PIDs; general-purpose MCU vendors
//! (Silicon Labs, WCH, Cisco) only match exact PIDs so mice, hubs or HID gadgets with
//! the same VID never show up as consoles or trigger a driver hint.

use crate::model::ChipInfo;

/// Which OS's driver page to link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetOs {
    Windows,
    MacOs,
    /// Linux and others: drivers ship with the kernel; the Windows page is used as the
    /// "official vendor page" link.
    Other,
}

impl TargetOs {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// Driver download page(s) of one driver package.
struct Driver {
    name: &'static str,
    windows: &'static str,
    macos: &'static str,
}

impl Driver {
    fn url(&self, os: TargetOs) -> &'static str {
        match os {
            TargetOs::MacOs => self.macos,
            TargetOs::Windows | TargetOs::Other => self.windows,
        }
    }
}

const FTDI_VCP: Driver = Driver {
    name: "FTDI VCP",
    windows: "https://ftdichip.com/drivers/vcp-drivers/",
    macos: "https://ftdichip.com/drivers/vcp-drivers/",
};

const PROLIFIC: Driver = Driver {
    name: "Prolific PL2303",
    windows: "https://www.prolific.com.tw/US/ShowProduct.aspx?p_id=225&pcid=41",
    macos: "https://www.prolific.com.tw/US/ShowProduct.aspx?p_id=225&pcid=41",
};

const SILABS_VCP: Driver = Driver {
    name: "Silicon Labs CP210x VCP",
    windows: "https://www.silabs.com/developers/usb-to-uart-bridge-vcp-drivers",
    macos: "https://www.silabs.com/developers/usb-to-uart-bridge-vcp-drivers",
};

/// CH340/CH341 family. On macOS one package (CH34XSER_MAC) covers the whole family.
const WCH_CH341SER: Driver = Driver {
    name: "WCH CH341SER",
    windows: "https://www.wch-ic.com/downloads/CH341SER_ZIP.html",
    macos: "https://www.wch-ic.com/downloads/CH34XSER_MAC_ZIP.html",
};

/// CH342/CH343/CH9102 use WCH's CDC-based CH343SER package on Windows.
const WCH_CH343SER: Driver = Driver {
    name: "WCH CH343SER",
    windows: "https://www.wch-ic.com/downloads/CH343SER_ZIP.html",
    macos: "https://www.wch-ic.com/downloads/CH34XSER_MAC_ZIP.html",
};

/// The Cisco USB console driver ships with IOS software downloads; this support page is
/// the closest stable entry point (to be verified with real hardware, see Known Issues).
const CISCO_CONSOLE: Driver = Driver {
    name: "Switch USB console driver",
    windows: "https://www.cisco.com/c/en/us/support/switches/index.html",
    macos: "https://www.cisco.com/c/en/us/support/switches/index.html",
};

struct Entry {
    vid: u16,
    /// `None` = fallback for any other PID of this vendor.
    pid: Option<u16>,
    vendor: &'static str,
    model: &'static str,
    driver: &'static Driver,
}

const fn exact(
    vid: u16,
    pid: u16,
    vendor: &'static str,
    model: &'static str,
    driver: &'static Driver,
) -> Entry {
    Entry {
        vid,
        pid: Some(pid),
        vendor,
        model,
        driver,
    }
}

const fn fallback(
    vid: u16,
    vendor: &'static str,
    model: &'static str,
    driver: &'static Driver,
) -> Entry {
    Entry {
        vid,
        pid: None,
        vendor,
        model,
        driver,
    }
}

pub const VID_FTDI: u16 = 0x0403;
pub const VID_PROLIFIC: u16 = 0x067B;
pub const VID_SILABS: u16 = 0x10C4;
pub const VID_WCH: u16 = 0x1A86;
pub const VID_CISCO: u16 = 0x05A6;
pub const VID_ATEN: u16 = 0x0557;

/// Exact entries first; fallbacks are only consulted when no exact entry matched.
static TABLE: &[Entry] = &[
    // FTDI
    exact(VID_FTDI, 0x6001, "FTDI", "FT232R", &FTDI_VCP),
    exact(VID_FTDI, 0x6010, "FTDI", "FT2232", &FTDI_VCP),
    exact(VID_FTDI, 0x6011, "FTDI", "FT4232", &FTDI_VCP),
    exact(VID_FTDI, 0x6014, "FTDI", "FT232H", &FTDI_VCP),
    exact(VID_FTDI, 0x6015, "FTDI", "FT230X/FT231X", &FTDI_VCP),
    fallback(VID_FTDI, "FTDI", "USB-Serial", &FTDI_VCP),
    // Prolific (PID map as in the Linux pl2303 driver)
    exact(VID_PROLIFIC, 0x2303, "Prolific", "PL2303", &PROLIFIC),
    exact(VID_PROLIFIC, 0x2304, "Prolific", "PL2303TB", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23A3, "Prolific", "PL2303GC", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23B3, "Prolific", "PL2303GB", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23C3, "Prolific", "PL2303GT", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23D3, "Prolific", "PL2303GL", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23E3, "Prolific", "PL2303GE", &PROLIFIC),
    exact(VID_PROLIFIC, 0x23F3, "Prolific", "PL2303GS", &PROLIFIC),
    fallback(VID_PROLIFIC, "Prolific", "USB-Serial", &PROLIFIC),
    // ATEN UC-232A: PL2303 inside, Prolific driver.
    exact(VID_ATEN, 0x2008, "ATEN", "UC-232A", &PROLIFIC),
    // Silicon Labs
    exact(VID_SILABS, 0xEA60, "Silicon Labs", "CP210x", &SILABS_VCP),
    exact(VID_SILABS, 0xEA70, "Silicon Labs", "CP2105", &SILABS_VCP),
    exact(VID_SILABS, 0xEA71, "Silicon Labs", "CP2108", &SILABS_VCP),
    // WCH
    exact(VID_WCH, 0x7523, "WCH", "CH340", &WCH_CH341SER),
    exact(VID_WCH, 0x7522, "WCH", "CH340K", &WCH_CH341SER),
    exact(VID_WCH, 0x5523, "WCH", "CH341", &WCH_CH341SER),
    exact(VID_WCH, 0x55D2, "WCH", "CH342", &WCH_CH343SER),
    exact(VID_WCH, 0x55D3, "WCH", "CH343", &WCH_CH343SER),
    exact(VID_WCH, 0x55D4, "WCH", "CH9102", &WCH_CH343SER),
    // Cisco
    exact(VID_CISCO, 0x0009, "Switch", "USB Console", &CISCO_CONSOLE),
];

/// Chip info for a USB VID:PID, if it is a known console chip.
pub fn chip(vid: u16, pid: u16) -> Option<ChipInfo> {
    chip_for_os(vid, pid, TargetOs::current())
}

pub(crate) fn chip_for_os(vid: u16, pid: u16, os: TargetOs) -> Option<ChipInfo> {
    let entry = TABLE
        .iter()
        .find(|e| e.vid == vid && e.pid == Some(pid))
        .or_else(|| TABLE.iter().find(|e| e.vid == vid && e.pid.is_none()))?;
    Some(ChipInfo {
        vendor: entry.vendor.to_owned(),
        model: entry.model.to_owned(),
        vid,
        pid,
        driver_name: Some(entry.driver.name.to_owned()),
        driver_url: Some(entry.driver.url(os).to_owned()),
    })
}

/// `FTDI FT232R` style label (identity bar).
pub fn label(chip: &ChipInfo) -> String {
    format!("{} {}", chip.vendor, chip.model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matches() {
        let cases = [
            (0x0403, 0x6001, "FTDI", "FT232R"),
            (0x0403, 0x6010, "FTDI", "FT2232"),
            (0x0403, 0x6011, "FTDI", "FT4232"),
            (0x0403, 0x6014, "FTDI", "FT232H"),
            (0x0403, 0x6015, "FTDI", "FT230X/FT231X"),
            (0x067B, 0x2303, "Prolific", "PL2303"),
            (0x067B, 0x23A3, "Prolific", "PL2303GC"),
            (0x067B, 0x23C3, "Prolific", "PL2303GT"),
            (0x067B, 0x23D3, "Prolific", "PL2303GL"),
            (0x067B, 0x23F3, "Prolific", "PL2303GS"),
            (0x10C4, 0xEA60, "Silicon Labs", "CP210x"),
            (0x10C4, 0xEA70, "Silicon Labs", "CP2105"),
            (0x10C4, 0xEA71, "Silicon Labs", "CP2108"),
            (0x1A86, 0x7523, "WCH", "CH340"),
            (0x1A86, 0x5523, "WCH", "CH341"),
            (0x1A86, 0x55D4, "WCH", "CH9102"),
            (0x1A86, 0x7522, "WCH", "CH340K"),
            (0x05A6, 0x0009, "Switch", "USB Console"),
        ];
        for (vid, pid, vendor, model) in cases {
            let c = chip(vid, pid).unwrap_or_else(|| panic!("{vid:04x}:{pid:04x}"));
            assert_eq!((c.vendor.as_str(), c.model.as_str()), (vendor, model));
            assert_eq!((c.vid, c.pid), (vid, pid));
            assert!(c.driver_name.is_some());
            assert!(c.driver_url.as_deref().unwrap().starts_with("https://"));
        }
    }

    #[test]
    fn vendor_fallbacks_keep_real_pid() {
        let c = chip(0x0403, 0x6042).unwrap();
        assert_eq!(label(&c), "FTDI USB-Serial");
        assert_eq!(c.pid, 0x6042);
        let c = chip(0x067B, 0xAAA0).unwrap();
        assert_eq!(label(&c), "Prolific USB-Serial");
    }

    #[test]
    fn general_purpose_vendors_need_exact_pid() {
        assert_eq!(chip(0x10C4, 0x8A2A), None); // Silicon Labs, not a UART bridge
        assert_eq!(chip(0x1A86, 0x8091), None); // WCH USB hub
        assert_eq!(chip(0x05A6, 0x0001), None);
        assert_eq!(chip(0x05AC, 0x8262), None); // Apple
        assert_eq!(chip(0x0000, 0x0000), None);
    }

    #[test]
    fn driver_url_per_os() {
        let win = chip_for_os(0x1A86, 0x7523, TargetOs::Windows).unwrap();
        let mac = chip_for_os(0x1A86, 0x7523, TargetOs::MacOs).unwrap();
        assert_eq!(
            win.driver_url.as_deref(),
            Some("https://www.wch-ic.com/downloads/CH341SER_ZIP.html")
        );
        assert_eq!(
            mac.driver_url.as_deref(),
            Some("https://www.wch-ic.com/downloads/CH34XSER_MAC_ZIP.html")
        );
        let ch9102 = chip_for_os(0x1A86, 0x55D4, TargetOs::Windows).unwrap();
        assert_eq!(
            ch9102.driver_url.as_deref(),
            Some("https://www.wch-ic.com/downloads/CH343SER_ZIP.html")
        );
        let ftdi = chip_for_os(0x0403, 0x6001, TargetOs::MacOs).unwrap();
        assert_eq!(
            ftdi.driver_url.as_deref(),
            Some("https://ftdichip.com/drivers/vcp-drivers/")
        );
    }

    #[test]
    fn table_has_no_duplicate_keys() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert!(
                    !(a.vid == b.vid && a.pid == b.pid),
                    "duplicate {:04x}:{:?}",
                    a.vid,
                    a.pid
                );
            }
        }
    }
}
