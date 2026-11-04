//! Windows helpers.

/// `COMx` names are not filesystem paths; ask the OS port list instead.
pub fn port_exists(path: &str) -> bool {
    let name = path.trim_start_matches(r"\\.\");
    serialport::available_ports()
        .map(|ports| ports.iter().any(|p| p.port_name.eq_ignore_ascii_case(name)))
        .unwrap_or(false)
}
