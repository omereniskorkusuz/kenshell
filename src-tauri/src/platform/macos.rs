//! macOS helpers.

pub fn port_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}
