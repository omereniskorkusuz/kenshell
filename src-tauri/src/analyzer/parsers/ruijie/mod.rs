//! Ruijie RGOS parser set (P4-T03). Interface names are often spaced
//! (`GigabitEthernet 0/1`, `AggregatePort 1`); fixed-width tables are cut at the column
//! offsets of their `---- ----` line. Layouts follow RGOS 11.x (see Known Issues: to be
//! confirmed on more trains).

pub mod aggregate_port;
pub mod cpu;
pub mod interfaces_status;
pub mod lldp;
pub mod mac_address_table;
pub mod memory;
pub mod poe;
pub mod reuse;
pub mod version;
