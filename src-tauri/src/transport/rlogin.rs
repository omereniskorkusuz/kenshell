//! Rlogin (RFC 1282, P2-T07). Handshake `\0local\0remote\0term/speed\0`; the server
//! acknowledges with one NUL byte which is dropped. Window-size updates need the
//! server's out-of-band request (TCP urgent data), which is not available through
//! portable socket APIs, so resizes are not forwarded (documented limitation).

use super::net::Codec;

pub const TERMINAL: &str = "xterm/38400";

#[derive(Debug)]
pub struct RloginCodec {
    local_user: String,
    remote_user: String,
    acked: bool,
}

impl RloginCodec {
    pub fn new(local_user: String, remote_user: String) -> Self {
        Self {
            local_user,
            remote_user,
            acked: false,
        }
    }
}

/// Current OS user name (rlogin "local user"); falls back to the remote name.
pub fn local_user_name(fallback: &str) -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| fallback.to_owned())
}

impl Codec for RloginCodec {
    fn greeting(&mut self) -> Vec<u8> {
        let mut g = vec![0u8];
        for part in [&self.local_user, &self.remote_user] {
            g.extend(part.bytes().filter(|b| *b != 0));
            g.push(0);
        }
        g.extend(TERMINAL.as_bytes());
        g.push(0);
        g
    }

    fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, _reply: &mut Vec<u8>) {
        let mut rest = input;
        if !self.acked && !rest.is_empty() {
            self.acked = true;
            if rest[0] == 0 {
                rest = &rest[1..];
            }
        }
        data.extend_from_slice(rest);
    }

    fn encode(&mut self, input: &[u8]) -> Vec<u8> {
        input.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_ack() {
        let mut c = RloginCodec::new("kens".into(), "admin".into());
        assert_eq!(c.greeting(), b"\0kens\0admin\0xterm/38400\0".to_vec());
        let (mut d, mut r) = (Vec::new(), Vec::new());
        c.decode(b"\0Password:", &mut d, &mut r);
        assert_eq!(d, b"Password:");
        d.clear();
        c.decode(b"\0x", &mut d, &mut r);
        assert_eq!(d, b"\0x", "only the first byte is the ack");
    }
}
