//! Topology-map sources for manual and end-to-end tests: an SNMPv2c agent (`--snmp`) that
//! presents a third device of the profiles' story (`dist-sw-01`), and a sender of sFlow /
//! NetFlow exports (`--sflow-to`, `--netflow-to`). The wire code is KenShell's own,
//! included by path, so both sides stay in step.

#[allow(dead_code)]
#[path = "../../../src-tauri/src/snmp/ber.rs"]
pub mod ber;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/snmp/agent.rs"]
pub mod agent;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/flow/testgen.rs"]
pub mod flowgen;

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use agent::{SampleNeighbor, sample_mib};

/// `dist-sw-01`: the distribution switch core-sw-01 reports on Te1/1/1.
fn dist_mib() -> agent::Mib {
    sample_mib(
        "dist-sw-01",
        "L3 Switch Software (C9500-24Y), Version 17.9.4a",
        "0011.2233.9500",
        &[
            (1, "Te1/0/1"),
            (2, "Te1/0/2"),
            (10, "Gi1/0/10"),
            (11, "Gi1/0/11"),
        ],
        &[
            SampleNeighbor {
                local_port: 1,
                chassis: "0011.2233.4400",
                port_id: "Te1/1/1",
                name: "core-sw-01",
                description: "L3 Switch Software (CAT9K), Version 17.9.4a",
                caps: 0x28,
                ip: [10, 0, 0, 1],
            },
            SampleNeighbor {
                local_port: 2,
                chassis: "5869.6c00.3333",
                port_id: "eth0",
                name: "ap-roof",
                description: "Outdoor access point",
                caps: 0x10,
                ip: [10, 20, 0, 40],
            },
        ],
        &[
            ("3c52.8200.0d05", 10),
            ("0015.9a00.0e06", 11),
            ("0011.2233.4400", 1),
        ],
        &[
            ([10, 0, 0, 2], "0011.2233.9500"),
            ([10, 10, 0, 77], "3c52.8200.0d05"),
            ([10, 10, 0, 78], "0015.9a00.0e06"),
        ],
    )
}

/// Serves the agent until the process is killed.
pub fn run_snmp(bind: &str, port: u16, community: &str) -> std::io::Result<()> {
    let socket = UdpSocket::bind((bind, port))?;
    eprintln!("device-simulator: SNMPv2c agent (dist-sw-01) on {bind}:{port}/udp");
    agent::serve(
        &socket,
        &dist_mib(),
        community.as_bytes(),
        &Arc::new(AtomicBool::new(false)),
    );
    Ok(())
}

/// Sends the same exports once a second, `rounds` times: access-sw-02 (10.0.0.12) sampling
/// two stations on ports 7 and 8 over sFlow, and a router exporting NetFlow v9.
pub fn send_flows(sflow: Option<&str>, netflow: Option<&str>, rounds: u32) -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let sflow_datagram = flowgen::sflow_v5(
        [10, 0, 0, 12],
        &[
            (
                7,
                flowgen::frame(
                    [0x3c, 0x52, 0x82, 0, 0x0f, 0x07],
                    [10, 20, 0, 107],
                    Some(20),
                ),
            ),
            (
                8,
                flowgen::frame(
                    [0x00, 0x15, 0x9a, 0, 0x0f, 0x08],
                    [10, 20, 0, 108],
                    Some(20),
                ),
            ),
        ],
        false,
    );
    let netflow_datagram = flowgen::template_flows(
        false,
        &[([10, 30, 0, 9], 3, [0x3c, 0x52, 0x82, 0, 0x10, 0x09])],
        true,
    );
    for _ in 0..rounds {
        if let Some(to) = sflow {
            socket.send_to(&sflow_datagram, to)?;
        }
        if let Some(to) = netflow {
            socket.send_to(&netflow_datagram, to)?;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}
