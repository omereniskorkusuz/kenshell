//! Linux helpers (P5-T12 preparation: AppImage / .deb in CI; serial group check in
//! `serial_access`).

pub fn port_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}
