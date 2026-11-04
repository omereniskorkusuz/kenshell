//! Telnet client (RFC 854/855, P2-T03): full IAC handling with option negotiation for
//! ECHO, SGA, BINARY (both directions), NAWS (window size, re-sent on resize) and
//! TTYPE; BREAK = `IAC BRK`. A small per-option state machine (RFC 1143 "Q method",
//! simplified) prevents negotiation loops: we only answer requests that change state.

use super::net::Codec;

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const BRK: u8 = 243;
pub const SE: u8 = 240;

pub const OPT_BINARY: u8 = 0;
pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
/// Max subnegotiation payload we buffer (protects against a hostile peer).
const SB_LIMIT: usize = 1024;

pub const TERMINAL_TYPE: &str = "XTERM-256COLOR";

/// Options the server may enable on its side (`WILL x` → we say `DO x`).
fn remote_ok(opt: u8) -> bool {
    matches!(opt, OPT_ECHO | OPT_SGA | OPT_BINARY)
}

/// Options we are willing to enable on our side (`DO x` → we say `WILL x`).
fn local_ok(opt: u8) -> bool {
    matches!(opt, OPT_NAWS | OPT_TTYPE | OPT_BINARY | OPT_SGA)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    /// Previous byte was CR (server side NVT: `CR NUL` = CR).
    Cr,
    Iac,
    Opt(u8),
    Sb,
    SbIac,
}

#[derive(Debug, Clone, Copy, Default)]
struct OptState {
    /// Enabled on our side (we WILL).
    us: bool,
    /// Enabled on the server side (server WILL).
    him: bool,
    /// We sent WILL and await DO/DONT.
    us_pending: bool,
    /// We sent DO and await WILL/WONT.
    him_pending: bool,
}

#[derive(Debug)]
pub struct TelnetCodec {
    state: State,
    opts: [OptState; 256],
    sb: Vec<u8>,
    size: (u16, u16),
    /// Last byte written by the user was CR (so a following LF is not doubled).
    last_input_cr: bool,
}

impl Default for TelnetCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl TelnetCodec {
    pub fn new() -> Self {
        Self {
            state: State::Data,
            opts: [OptState::default(); 256],
            sb: Vec::new(),
            size: (80, 24),
            last_input_cr: false,
        }
    }

    fn opt(&mut self, o: u8) -> &mut OptState {
        &mut self.opts[o as usize]
    }

    /// Server echoes typed characters (we must not echo locally).
    #[cfg(test)]
    pub fn remote_echo(&self) -> bool {
        self.opts[OPT_ECHO as usize].him
    }

    fn naws_bytes(&self) -> Vec<u8> {
        let (c, r) = self.size;
        let mut v = vec![IAC, SB, OPT_NAWS];
        for b in c.to_be_bytes().into_iter().chain(r.to_be_bytes()) {
            v.push(b);
            if b == IAC {
                v.push(IAC); // IAC inside SB must be doubled
            }
        }
        v.extend([IAC, SE]);
        v
    }

    fn negotiate(&mut self, cmd: u8, o: u8, reply: &mut Vec<u8>) {
        match cmd {
            WILL => {
                let st = *self.opt(o);
                if st.him {
                    return;
                }
                if remote_ok(o) {
                    self.opt(o).him = true;
                    if !st.him_pending {
                        reply.extend([IAC, DO, o]);
                    }
                } else {
                    reply.extend([IAC, DONT, o]);
                }
                self.opt(o).him_pending = false;
            }
            WONT => {
                let st = *self.opt(o);
                if st.him {
                    self.opt(o).him = false;
                    reply.extend([IAC, DONT, o]);
                }
                self.opt(o).him_pending = false;
            }
            DO => {
                let st = *self.opt(o);
                if !st.us {
                    if local_ok(o) {
                        self.opt(o).us = true;
                        if !st.us_pending {
                            reply.extend([IAC, WILL, o]);
                        }
                        if o == OPT_NAWS {
                            reply.extend(self.naws_bytes());
                        }
                    } else {
                        reply.extend([IAC, WONT, o]);
                    }
                }
                self.opt(o).us_pending = false;
            }
            DONT => {
                let st = *self.opt(o);
                if st.us {
                    self.opt(o).us = false;
                    reply.extend([IAC, WONT, o]);
                }
                self.opt(o).us_pending = false;
            }
            _ => {}
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if let [OPT_TTYPE, TTYPE_SEND, ..] = self.sb.as_slice()
            && self.opts[OPT_TTYPE as usize].us
        {
            reply.extend([IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend(TERMINAL_TYPE.as_bytes());
            reply.extend([IAC, SE]);
        }
        self.sb.clear();
    }
}

impl Codec for TelnetCodec {
    /// Active negotiation: offer window size + terminal type, ask the server to
    /// echo and suppress go-ahead (character mode, what network devices expect).
    fn greeting(&mut self) -> Vec<u8> {
        for (o, us) in [
            (OPT_NAWS, true),
            (OPT_TTYPE, true),
            (OPT_SGA, false),
            (OPT_ECHO, false),
        ] {
            if us {
                self.opt(o).us_pending = true;
            } else {
                self.opt(o).him_pending = true;
            }
        }
        vec![
            IAC, WILL, OPT_NAWS, IAC, WILL, OPT_TTYPE, IAC, DO, OPT_SGA, IAC, DO, OPT_ECHO,
        ]
    }

    fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &b in input {
            self.state = match (self.state, b) {
                (State::Data | State::Cr, IAC) => State::Iac,
                // NVT: CR NUL means a bare CR — drop the NUL.
                (State::Cr, 0) => State::Data,
                (State::Data | State::Cr, b'\r') => {
                    data.push(b);
                    State::Cr
                }
                (State::Data | State::Cr, _) => {
                    data.push(b);
                    State::Data
                }
                (State::Iac, IAC) => {
                    data.push(IAC);
                    State::Data
                }
                (State::Iac, DO | DONT | WILL | WONT) => State::Opt(b),
                (State::Iac, SB) => {
                    self.sb.clear();
                    State::Sb
                }
                (State::Iac, _) => State::Data, // NOP, GA, DM, AYT …
                (State::Opt(cmd), o) => {
                    self.negotiate(cmd, o, reply);
                    State::Data
                }
                (State::Sb, IAC) => State::SbIac,
                (State::Sb, _) => {
                    if self.sb.len() < SB_LIMIT {
                        self.sb.push(b);
                    }
                    State::Sb
                }
                (State::SbIac, IAC) => {
                    if self.sb.len() < SB_LIMIT {
                        self.sb.push(IAC);
                    }
                    State::Sb
                }
                (State::SbIac, SE) => {
                    self.subnegotiation(reply);
                    State::Data
                }
                (State::SbIac, _) => {
                    self.sb.clear();
                    State::Data
                }
            };
        }
    }

    /// Escapes IAC; outside BINARY mode a lone CR (Enter) becomes CR LF (RFC 854 NVT,
    /// what network devices accept). A CR already followed by LF is left alone.
    fn encode(&mut self, input: &[u8]) -> Vec<u8> {
        let binary = self.opts[OPT_BINARY as usize].us;
        let mut out = Vec::with_capacity(input.len() + 2);
        for &b in input {
            if !binary && self.last_input_cr && b != b'\n' {
                out.push(b'\n');
            }
            self.last_input_cr = false;
            match b {
                IAC => out.extend([IAC, IAC]),
                b'\r' if !binary => {
                    out.push(b'\r');
                    self.last_input_cr = true;
                }
                _ => out.push(b),
            }
        }
        // Enter is usually the last byte of a chunk: finish the pair now.
        if self.last_input_cr {
            out.push(b'\n');
            self.last_input_cr = false;
        }
        out
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.size = (cols, rows);
        self.opts[OPT_NAWS as usize].us.then(|| self.naws_bytes())
    }

    fn brk(&self) -> Option<Vec<u8>> {
        Some(vec![IAC, BRK])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(c: &mut TelnetCodec, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (mut d, mut r) = (Vec::new(), Vec::new());
        c.decode(input, &mut d, &mut r);
        (d, r)
    }

    #[test]
    fn data_passes_and_iac_is_unescaped() {
        let mut c = TelnetCodec::new();
        let (d, r) = decode(
            &mut c,
            &[b'a', IAC, IAC, b'b', b'\r', 0, b'c', b'\r', b'\n'],
        );
        assert_eq!(d, vec![b'a', IAC, b'b', b'\r', b'c', b'\r', b'\n']);
        assert!(r.is_empty());
    }

    #[test]
    fn answers_server_offers_without_loops() {
        let mut c = TelnetCodec::new();
        let _ = c.greeting(); // we already asked DO ECHO / DO SGA
        let (_, r) = decode(&mut c, &[IAC, WILL, OPT_ECHO, IAC, WILL, OPT_SGA]);
        assert!(
            r.is_empty(),
            "acks of our own requests need no reply: {r:?}"
        );
        assert!(c.remote_echo());
        // Repeating WILL ECHO must not produce another DO (no loop).
        let (_, r) = decode(&mut c, &[IAC, WILL, OPT_ECHO]);
        assert!(r.is_empty());
        // Unknown option offered → refuse once.
        let (_, r) = decode(&mut c, &[IAC, WILL, 39]);
        assert_eq!(r, vec![IAC, DONT, 39]);
        let (_, r) = decode(&mut c, &[IAC, DO, 36]);
        assert_eq!(r, vec![IAC, WONT, 36]);
        // Server turns echo off → we confirm.
        let (_, r) = decode(&mut c, &[IAC, WONT, OPT_ECHO]);
        assert_eq!(r, vec![IAC, DONT, OPT_ECHO]);
        assert!(!c.remote_echo());
    }

    #[test]
    fn naws_on_do_and_on_resize_with_iac_doubling() {
        let mut c = TelnetCodec::new();
        assert_eq!(c.resize(100, 30), None, "NAWS not negotiated yet");
        let (_, r) = decode(&mut c, &[IAC, DO, OPT_NAWS]);
        assert_eq!(
            r,
            vec![
                IAC, WILL, OPT_NAWS, IAC, SB, OPT_NAWS, 0, 100, 0, 30, IAC, SE
            ]
        );
        let b = c.resize(255, 40).unwrap();
        assert_eq!(b, vec![IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 40, IAC, SE]);
    }

    #[test]
    fn ttype_send_is_answered_split_across_chunks() {
        let mut c = TelnetCodec::new();
        let (_, r) = decode(&mut c, &[IAC, DO, OPT_TTYPE, IAC, SB, OPT_TTYPE]);
        assert_eq!(r, vec![IAC, WILL, OPT_TTYPE]);
        let (_, r) = decode(&mut c, &[TTYPE_SEND, IAC, SE]);
        let mut want = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        want.extend(TERMINAL_TYPE.as_bytes());
        want.extend([IAC, SE]);
        assert_eq!(r, want);
    }

    #[test]
    fn oversized_subnegotiation_is_bounded() {
        let mut c = TelnetCodec::new();
        let mut evil = vec![IAC, SB, 99];
        evil.extend(std::iter::repeat_n(b'x', 100_000));
        let (d, _) = decode(&mut c, &evil);
        assert!(d.is_empty());
        assert!(c.sb.len() <= SB_LIMIT);
        let (d, _) = decode(&mut c, &[IAC, SE, b'k']);
        assert_eq!(d, b"k");
    }

    #[test]
    fn enter_becomes_crlf_and_iac_is_escaped() {
        let mut c = TelnetCodec::new();
        assert_eq!(c.encode(b"show ver\r"), b"show ver\r\n");
        assert_eq!(c.encode(b"a\r\nb"), b"a\r\nb");
        assert_eq!(c.encode(&[1, IAC, 2]), vec![1, IAC, IAC, 2]);
        // Binary mode: CR passes untouched.
        let _ = decode(&mut c, &[IAC, DO, OPT_BINARY]);
        assert_eq!(c.encode(b"x\r"), b"x\r");
    }

    #[test]
    fn break_is_iac_brk() {
        assert_eq!(TelnetCodec::new().brk(), Some(vec![IAC, BRK]));
    }
}
