//! KenShell device simulator — a local fake Cisco/Ruijie CLI for development and tests.
//!
//! ```text
//! device-simulator [--profile cisco|ruijie|<file.toml>] [--port 2323] [--bind 127.0.0.1]
//!                  [--raw] [--stdio] [--list]
//! ```
//! * default: Telnet server (with IAC negotiation) on 127.0.0.1:2323
//! * `--raw`:  plain TCP, no Telnet negotiation (KenShell "Raw" protocol)
//! * `--stdio`: one session on stdin/stdout — pair with `socat` to emulate a serial console:
//!   `socat pty,raw,echo=0,link=/tmp/ks-console EXEC:'device-simulator --stdio --profile ruijie'`
//! * `--ssh` SSH server (default port 2222; `--host-key-seed N`, `--legacy-only`)
//! * `--pty` (unix): creates a pseudo-terminal pair and prints the device path to open in
//!   KenShell as a serial port (no socat needed). With `--baud N` the simulator emits
//!   garbage unless the port is set to N (exercises autobaud).

mod engine;
mod profile;
mod ssh;
mod telnet;
mod topo;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use engine::Session;
use profile::Profile;

struct Options {
    profile: String,
    bind: String,
    port: u16,
    raw: bool,
    stdio: bool,
    pty: bool,
    baud: Option<u32>,
    ssh: bool,
    host_key_seed: u64,
    legacy_only: bool,
    /// OpenSSH public key lines accepted for publickey auth (repeatable).
    authorized_keys: Vec<String>,
    /// Lines of `show`-like output sent on connect (fills scrollback for RAM benchmarks).
    flood_lines: u32,
    port_set: bool,
    list: bool,
    /// Serve the SNMPv2c agent on this UDP port instead of a CLI.
    snmp: Option<u16>,
    community: String,
    /// Send sFlow / NetFlow exports to these `host:port` addresses instead of a CLI.
    sflow_to: Option<String>,
    netflow_to: Option<String>,
    flow_rounds: u32,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        profile: "cisco-ios".into(),
        bind: "127.0.0.1".into(),
        port: 2323,
        raw: false,
        stdio: false,
        pty: false,
        baud: None,
        ssh: false,
        host_key_seed: 1,
        legacy_only: false,
        authorized_keys: Vec::new(),
        flood_lines: 0,
        port_set: false,
        list: false,
        snmp: None,
        community: "public".into(),
        sflow_to: None,
        netflow_to: None,
        flow_rounds: 30,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--profile" | "-p" => o.profile = val()?,
            "--bind" => o.bind = val()?,
            "--port" => {
                o.port = val()?.parse().map_err(|e| format!("--port: {e}"))?;
                o.port_set = true;
            }
            "--ssh" => o.ssh = true,
            "--legacy-only" => o.legacy_only = true,
            "--authorized-key" => o.authorized_keys.push(val()?),
            "--flood-lines" => {
                o.flood_lines = val()?.parse().map_err(|e| format!("--flood-lines: {e}"))?;
            }
            "--host-key-seed" => {
                o.host_key_seed = val()?
                    .parse()
                    .map_err(|e| format!("--host-key-seed: {e}"))?;
            }
            "--raw" => o.raw = true,
            "--stdio" => o.stdio = true,
            "--pty" => o.pty = true,
            "--baud" => o.baud = Some(val()?.parse().map_err(|e| format!("--baud: {e}"))?),
            "--list" => o.list = true,
            "--snmp" => o.snmp = Some(val()?.parse().map_err(|e| format!("--snmp: {e}"))?),
            "--community" => o.community = val()?,
            "--sflow-to" => o.sflow_to = Some(val()?),
            "--netflow-to" => o.netflow_to = Some(val()?),
            "--flow-rounds" => {
                o.flow_rounds = val()?.parse().map_err(|e| format!("--flow-rounds: {e}"))?;
            }
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(o)
}

fn load_profile(spec: &str) -> Result<Profile, String> {
    if spec.ends_with(".toml") {
        let src = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
        Profile::parse(&src)
    } else {
        Profile::builtin(spec)
    }
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!(
                "usage: device-simulator [--profile NAME|FILE.toml] [--port N] [--bind ADDR] [--raw] [--ssh [--host-key-seed N] [--legacy-only] [--authorized-key 'ssh-ed25519 AAAA…']] [--flood-lines N] [--stdio] [--pty [--baud N]] [--list] | --snmp PORT [--community NAME] | [--sflow-to HOST:PORT] [--netflow-to HOST:PORT] [--flow-rounds N]"
            );
            std::process::exit(2);
        }
    };
    if opts.list {
        for (name, _) in profile::BUILTIN {
            println!("{name}");
        }
        return;
    }
    if let Some(port) = opts.snmp {
        if let Err(e) = topo::run_snmp(&opts.bind, port, &opts.community) {
            eprintln!("error: snmp: {e}");
            std::process::exit(1);
        }
        return;
    }
    if opts.sflow_to.is_some() || opts.netflow_to.is_some() {
        if let Err(e) = topo::send_flows(
            opts.sflow_to.as_deref(),
            opts.netflow_to.as_deref(),
            opts.flow_rounds,
        ) {
            eprintln!("error: flows: {e}");
            std::process::exit(1);
        }
        return;
    }
    let profile = match load_profile(&opts.profile) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    if opts.ssh {
        // The SSH handler needs a 'static profile; this process serves it until exit.
        let profile: &'static Profile = Box::leak(Box::new((*profile).clone()));
        let result = ssh::run(
            profile,
            ssh::Options {
                bind: opts.bind.clone(),
                port: if opts.port_set { opts.port } else { 2222 },
                host_key_seed: opts.host_key_seed,
                legacy_only: opts.legacy_only,
                authorized_keys: opts.authorized_keys.clone(),
            },
        );
        if let Err(e) = result {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let result = if opts.stdio {
        run_stdio(&profile)
    } else if opts.pty {
        run_pty(&profile, opts.baud)
    } else {
        run_tcp(&opts, profile)
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_stdio(profile: &Profile) -> std::io::Result<()> {
    let mut session = Session::new(profile);
    let mut stdout = std::io::stdout().lock();
    let mut stdin = std::io::stdin().lock();
    stdout.write_all(&session.start())?;
    stdout.flush()?;
    let mut buf = [0u8; 1024];
    while !session.is_closed() {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        stdout.write_all(&session.feed(&buf[..n]))?;
        stdout.flush()?;
    }
    Ok(())
}

fn run_tcp(opts: &Options, profile: Arc<Profile>) -> std::io::Result<()> {
    let listener = TcpListener::bind((opts.bind.as_str(), opts.port))?;
    let kind = if opts.raw { "raw" } else { "telnet" };
    eprintln!(
        "device-simulator: {} ({}) listening on {} [{kind}]",
        profile.name,
        profile.vendor,
        listener.local_addr()?
    );
    for stream in listener.incoming() {
        let stream = stream?;
        let profile = Arc::clone(&profile);
        let raw = opts.raw;
        let flood = opts.flood_lines;
        std::thread::spawn(move || {
            let peer = stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_default();
            eprintln!("+ {peer}");
            if let Err(e) = serve(stream, &profile, raw, flood) {
                eprintln!("  {peer}: {e}");
            }
            eprintln!("- {peer}");
        });
    }
    Ok(())
}

/// `n` lines resembling `show interfaces status` (about 80 columns, some colour-worthy words).
fn flood_text(n: u32) -> Vec<u8> {
    let mut out = String::with_capacity(n as usize * 82);
    for i in 0..n {
        let state = if i % 7 == 0 {
            "notconnect"
        } else {
            "connected "
        };
        out.push_str(&format!(
            "Gi1/0/{:<4} access-{:05}   {state}  {:>4}  a-full a-1000 10/100/1000BaseTX  \r\n",
            i % 48 + 1,
            i,
            i % 4094 + 1,
        ));
    }
    out.into_bytes()
}

fn serve(mut stream: TcpStream, profile: &Profile, raw: bool, flood: u32) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut codec = telnet::Codec::new();
    let mut session = Session::new(profile);
    let encode = |b: Vec<u8>| if raw { b } else { telnet::Codec::encode(&b) };
    if !raw {
        stream.write_all(&telnet::Codec::initial_negotiation())?;
    }
    if flood > 0 {
        stream.write_all(&encode(flood_text(flood)))?;
    }
    stream.write_all(&encode(session.start()))?;
    let mut buf = [0u8; 4096];
    while !session.is_closed() {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if raw {
            stream.write_all(&session.feed(&buf[..n]))?;
            continue;
        }
        let decoded = codec.decode(&buf[..n]);
        if !decoded.reply.is_empty() {
            stream.write_all(&decoded.reply)?;
        }
        for ev in decoded.events {
            match ev {
                telnet::Event::Break => stream.write_all(&encode(session.on_break()))?,
                other => eprintln!("  telnet: {other:?}"),
            }
        }
        if !decoded.data.is_empty() {
            stream.write_all(&encode(session.feed(&decoded.data)))?;
        }
    }
    Ok(())
}

/// Speed the client set on the pty (termios is per device, so any fd shows it).
/// Linux: serialport sets it via termios2/BOTHER, which only `TTYPort::baud_rate`
/// (TCGETS2) reads back correctly. macOS: serialport caches the IOSSIOSPEED value,
/// so read the termios of a second fd instead.
#[cfg(target_os = "linux")]
fn line_speed(slave: &serialport::TTYPort, _tty: &std::fs::File) -> Option<u32> {
    use serialport::SerialPort;
    slave.baud_rate().ok()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn line_speed(_slave: &serialport::TTYPort, tty: &std::fs::File) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let t = nix::sys::termios::tcgetattr(tty.as_raw_fd()).ok()?;
    u32::try_from(nix::sys::termios::cfgetospeed(&t) as u64).ok()
}

#[cfg(unix)]
fn run_pty(profile: &Profile, baud: Option<u32>) -> std::io::Result<()> {
    use serialport::{SerialPort, TTYPort};
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Duration;

    let (mut master, slave) = TTYPort::pair().map_err(std::io::Error::other)?;
    let path = slave.name().unwrap_or_default();
    // Keep the slave end alive so the device node stays valid while KenShell (re)opens it.
    let slave = slave;
    // Termios is per tty device: a second fd on the slave reveals the speed KenShell set.
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::fcntl::OFlag::O_NOCTTY.bits())
        .open(&path)?;
    master
        .set_timeout(Duration::from_millis(200))
        .map_err(std::io::Error::other)?;
    eprintln!(
        "device-simulator: {} ({}) on pseudo-terminal {path}{}",
        profile.name,
        profile.vendor,
        baud.map(|b| format!(" (clean output only at {b} baud)"))
            .unwrap_or_default()
    );
    println!("{path}");

    let mut session = Session::new(profile);
    let mut started = false;
    let mut buf = [0u8; 1024];
    loop {
        let n = match master.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            // EIO while no client has the slave open: wait for the next one.
            Err(_) => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let mut out = Vec::new();
        if !started {
            started = true;
            out.extend(session.start());
        }
        out.extend(session.feed(&buf[..n]));
        if session.is_closed() {
            session = Session::new(profile);
            started = false;
        }
        let speed_ok = match baud {
            Some(want) => line_speed(&slave, &probe).is_none_or(|b| b == want),
            None => true,
        };
        if !speed_ok {
            out = garble(&out);
        }
        master.write_all(&out)?;
    }
}

#[cfg(not(unix))]
fn run_pty(_profile: &Profile, _baud: Option<u32>) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "--pty is unix-only; on Windows use a com0com pair with --stdio",
    ))
}

/// What a terminal sees when the line speed is wrong: framing noise, high bytes.
#[cfg(unix)]
fn garble(data: &[u8]) -> Vec<u8> {
    data.iter().map(|b| b.rotate_left(3) | 0x80).collect()
}
