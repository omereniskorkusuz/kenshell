//! Linux helpers (future target).

pub fn port_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}
