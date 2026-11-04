//! Minimal server-side Telnet codec (RFC 854/855): strips IAC sequences from client input,
//! answers option negotiation and escapes 0xFF in output. Enough to exercise KenShell's
//! Telnet client (ECHO, SGA, NAWS, TTYPE).

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;
pub const BRK: u8 = 243;

pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    Opt(u8),
    Sb,
    SbIac,
}

/// Events decoded from the client that the session may care about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    WindowSize { cols: u16, rows: u16 },
    TerminalType(String),
    Break,
}

#[derive(Debug)]
pub struct Codec {
    state: State,
    sb: Vec<u8>,
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of decoding a chunk of client bytes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// Application data (IAC removed).
    pub data: Vec<u8>,
    /// Raw bytes to send back to the client (negotiation replies).
    pub reply: Vec<u8>,
    pub events: Vec<Event>,
}

impl Codec {
    pub fn new() -> Self {
        Self {
            state: State::Data,
            sb: Vec::new(),
        }
    }

    /// What the server announces right after accept.
    pub fn initial_negotiation() -> Vec<u8> {
        vec![
            IAC, WILL, OPT_ECHO, IAC, WILL, OPT_SGA, IAC, DO, OPT_NAWS, IAC, DO, OPT_TTYPE,
        ]
    }

    pub fn decode(&mut self, input: &[u8]) -> Decoded {
        let mut out = Decoded::default();
        for &b in input {
            self.state = match (self.state, b) {
                (State::Data, IAC) => State::Iac,
                (State::Data, _) => {
                    out.data.push(b);
                    State::Data
                }
                (State::Iac, IAC) => {
                    out.data.push(IAC);
                    State::Data
                }
                (State::Iac, DO | DONT | WILL | WONT) => State::Opt(b),
                (State::Iac, SB) => {
                    self.sb.clear();
                    State::Sb
                }
                (State::Iac, BRK) => {
                    out.events.push(Event::Break);
                    State::Data
                }
                (State::Iac, _) => State::Data, // NOP, GA, AYT … ignored
                (State::Opt(cmd), opt) => {
                    Self::negotiate(cmd, opt, &mut out.reply);
                    State::Data
                }
                (State::Sb, IAC) => State::SbIac,
                (State::Sb, _) => {
                    self.sb.push(b);
                    State::Sb
                }
                (State::SbIac, IAC) => {
                    self.sb.push(IAC);
                    State::Sb
                }
                (State::SbIac, SE) => {
                    if let Some(ev) = Self::subnegotiation(&self.sb) {
                        out.events.push(ev);
                    }
                    State::Data
                }
                (State::SbIac, _) => State::Data,
            };
        }
        out
    }

    fn negotiate(cmd: u8, opt: u8, reply: &mut Vec<u8>) {
        match (cmd, opt) {
            // Acks of what we offered/requested: nothing to answer.
            (DO, OPT_ECHO | OPT_SGA) | (WILL, OPT_NAWS) => {}
            (WILL, OPT_TTYPE) => reply.extend([IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]),
            (DONT | WONT, _) => {}
            (DO, o) => reply.extend([IAC, WONT, o]),
            (WILL, o) => reply.extend([IAC, DONT, o]),
            _ => {}
        }
    }

    fn subnegotiation(sb: &[u8]) -> Option<Event> {
        match sb {
            [OPT_NAWS, c1, c0, r1, r0] => Some(Event::WindowSize {
                cols: u16::from_be_bytes([*c1, *c0]),
                rows: u16::from_be_bytes([*r1, *r0]),
            }),
            [OPT_TTYPE, TTYPE_IS, name @ ..] => Some(Event::TerminalType(
                String::from_utf8_lossy(name).into_owned(),
            )),
            _ => None,
        }
    }

    /// Escapes 0xFF in application data.
    pub fn encode(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for &b in data {
            out.push(b);
            if b == IAC {
                out.push(IAC);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_data_passes_through() {
        let d = Codec::new().decode(b"show ver\r\n");
        assert_eq!(d.data, b"show ver\r\n");
        assert!(d.reply.is_empty());
    }

    #[test]
    fn strips_negotiation_and_answers_unknown_options() {
        let mut c = Codec::new();
        let d = c.decode(&[IAC, DO, OPT_ECHO, b'a', IAC, DO, 39, IAC, WILL, 33, b'b']);
        assert_eq!(d.data, b"ab");
        assert_eq!(d.reply, vec![IAC, WONT, 39, IAC, DONT, 33]);
    }

    #[test]
    fn escaped_iac_in_data() {
        let d = Codec::new().decode(&[b'x', IAC, IAC, b'y']);
        assert_eq!(d.data, vec![b'x', IAC, b'y']);
        assert_eq!(Codec::encode(&[1, IAC, 2]), vec![1, IAC, IAC, 2]);
    }

    #[test]
    fn naws_and_ttype_subnegotiation_split_across_chunks() {
        let mut c = Codec::new();
        let mut d = c.decode(&[IAC, SB, OPT_NAWS, 0, 120]);
        assert!(d.events.is_empty());
        d = c.decode(&[0, 40, IAC, SE]);
        assert_eq!(
            d.events,
            vec![Event::WindowSize {
                cols: 120,
                rows: 40
            }]
        );

        let d = c.decode(&[IAC, WILL, OPT_TTYPE]);
        assert_eq!(d.reply, vec![IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        let mut sb = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        sb.extend(b"XTERM-256COLOR");
        sb.extend([IAC, SE]);
        assert_eq!(
            c.decode(&sb).events,
            vec![Event::TerminalType("XTERM-256COLOR".into())]
        );
    }

    #[test]
    fn break_is_reported() {
        let d = Codec::new().decode(&[IAC, BRK]);
        assert_eq!(d.events, vec![Event::Break]);
    }
}
