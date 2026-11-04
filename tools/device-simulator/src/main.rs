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

mod engine;
mod profile;
mod telnet;

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
    list: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        profile: "cisco-ios".into(),
        bind: "127.0.0.1".into(),
        port: 2323,
        raw: false,
        stdio: false,
        list: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--profile" | "-p" => o.profile = val()?,
            "--bind" => o.bind = val()?,
            "--port" => o.port = val()?.parse().map_err(|e| format!("--port: {e}"))?,
            "--raw" => o.raw = true,
            "--stdio" => o.stdio = true,
            "--list" => o.list = true,
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
                "usage: device-simulator [--profile NAME|FILE.toml] [--port N] [--bind ADDR] [--raw] [--stdio] [--list]"
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
    let profile = match load_profile(&opts.profile) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let result = if opts.stdio {
        run_stdio(&profile)
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
        std::thread::spawn(move || {
            let peer = stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_default();
            eprintln!("+ {peer}");
            if let Err(e) = serve(stream, &profile, raw) {
                eprintln!("  {peer}: {e}");
            }
            eprintln!("- {peer}");
        });
    }
    Ok(())
}

fn serve(mut stream: TcpStream, profile: &Profile, raw: bool) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut codec = telnet::Codec::new();
    let mut session = Session::new(profile);
    let encode = |b: Vec<u8>| if raw { b } else { telnet::Codec::encode(&b) };
    if !raw {
        stream.write_all(&telnet::Codec::initial_negotiation())?;
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
