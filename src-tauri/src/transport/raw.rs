//! Raw TCP (P2-T07): bytes pass through unchanged (terminal servers, reverse telnet
//! ports configured as raw).

use super::net::Codec;

#[derive(Debug, Default)]
pub struct RawCodec;

impl Codec for RawCodec {
    fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, _reply: &mut Vec<u8>) {
        data.extend_from_slice(input);
    }

    fn encode(&mut self, input: &[u8]) -> Vec<u8> {
        input.to_vec()
    }
}
