/*
* Copyright 2019 Comcast Cable Communications Management, LLC
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
* http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
*
* SPDX-License-Identifier: Apache-2.0
*/

use packets::ip::{IpAddrMismatchError, ProtocolNumber};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::slice;

/// Generic Pseudo Header used for checksumming purposes.
pub enum PseudoHeader {
    V4 {
        src: Ipv4Addr,
        dst: Ipv4Addr,
        packet_len: u16,
        protocol: ProtocolNumber,
    },
    V6 {
        src: Ipv6Addr,
        dst: Ipv6Addr,
        packet_len: u16,
        protocol: ProtocolNumber,
    },
}

impl PseudoHeader {
    pub fn sum(&self) -> u16 {
        let mut sum = match *self {
            PseudoHeader::V4 {
                src,
                dst,
                packet_len,
                protocol,
            } => self.v4_sum(src, dst, packet_len, protocol),
            PseudoHeader::V6 {
                src,
                dst,
                packet_len,
                protocol,
            } => self.v6_sum(src, dst, packet_len, protocol),
        };

        while sum >> 16 != 0 {
            sum = (sum >> 16) + (sum & 0xFFFF);
        }

        sum as u16
    }

    /*   0      7 8     15 16    23 24    31
       +--------+--------+--------+--------+
       |          source address           |
       +--------+--------+--------+--------+
       |        destination address        |
       +--------+--------+--------+--------+
       |  zero  |protocol|  packet length  |
       +--------+--------+--------+--------+
    */

    fn v4_sum(
        &self,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        packet_len: u16,
        protocol: ProtocolNumber,
    ) -> u32 {
        let src: u32 = src.into();
        let dst: u32 = dst.into();

        (src >> 16)
            + (src & 0xFFFF)
            + (dst >> 16)
            + (dst & 0xFFFF)
            + protocol.0 as u32
            + packet_len as u32
    }

    /*  https://tools.ietf.org/html/rfc2460#section-8.1

       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       |                                                               |
       +                                                               +
       |                                                               |
       +                         Source Address                        +
       |                                                               |
       +                                                               +
       |                                                               |
       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       |                                                               |
       +                                                               +
       |                                                               |
       +                      Destination Address                      +
       |                                                               |
       +                                                               +
       |                                                               |
       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       |                   Upper-Layer Packet Length                   |
       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       |                      zero                     |  Next Header  |
       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    */

    fn v6_sum(
        &self,
        src: Ipv6Addr,
        dst: Ipv6Addr,
        packet_len: u16,
        protocol: ProtocolNumber,
    ) -> u32 {
        src.segments().iter().fold(0, |acc, &x| acc + x as u32)
            + dst.segments().iter().fold(0, |acc, &x| acc + x as u32)
            + packet_len as u32
            + protocol.0 as u32
    }
}

/// Computes the internet checksum
/// https://tools.ietf.org/html/rfc1071
///
/// (1) Adjacent octets to be checksummed are paired to form 16-bit
///     integers, and the 1's complement sum of these 16-bit integers is
///     formed.
///
/// (2) To generate a checksum, the checksum field itself is cleared,
///     the 16-bit 1's complement sum is computed over the octets
///     concerned, and the 1's complement of this sum is placed in the
///     checksum field.
///
/// (3) To check a checksum, the 1's complement sum is computed over the
///     same set of octets, including the checksum field.  If the result
///     is all 1 bits (-0 in 1's complement arithmetic), the check
///     succeeds.
pub fn compute(pseudo_header_sum: u16, payload: &[u8]) -> u16 {
    let len = payload.len();
    let mut data = payload;
    let mut checksum = pseudo_header_sum as u32;

    // odd # of bytes, we add the last byte with padding separately
    if len % 2 > 0 {
        checksum += (payload[len - 1] as u32) << 8;
        data = &payload[..(len - 1)];
    }

    // a bit of unsafe magic to cast [u8] to [u16], and fix endianness later
    let data = unsafe { slice::from_raw_parts(data.as_ptr() as *const u16, len / 2) };

    checksum = data
        .iter()
        .fold(checksum, |acc, &x| acc + u16::from_be(x) as u32);

    while checksum >> 16 != 0 {
        checksum = (checksum >> 16) + (checksum & 0xFFFF);
    }

    return !(checksum as u16);
}

/// Computes the internet checksum via incremental update
/// https://tools.ietf.org/html/rfc1624
///
/// Given the following notation:
/// - `HC`  - old checksum in header
/// - `HC'` - new checksum in header
/// - `m`   - old value of a 16-bit field
/// - `m'`  - new value of a 16-bit field
///
/// `HC' = ~(~HC + ~m + m')`
pub fn compute_inc(old_checksum: u16, old_value: &[u16], new_value: &[u16]) -> u16 {
    let mut checksum = old_value
        .iter()
        .zip(new_value.iter())
        .fold(!old_checksum as u32, |acc, (&old, &new)| {
            acc + !old as u32 + new as u32
        });

    while checksum >> 16 != 0 {
        checksum = (checksum >> 16) + (checksum & 0xFFFF);
    }

    !(checksum as u16)
}

/// Incrementally computes the new checksum for an IP address change
pub fn compute_with_ipaddr(
    old_checksum: u16,
    old_value: &IpAddr,
    new_value: &IpAddr,
) -> Result<u16> {
    match (old_value, new_value) {
        (IpAddr::V4(old), IpAddr::V4(new)) => {
            let old: u32 = (*old).into();
            let old = [(old >> 16) as u16, (old & 0xFFFF) as u16];
            let new: u32 = (*new).into();
            let new = [(new >> 16) as u16, (new & 0xFFFF) as u16];
            Ok(compute_inc(old_checksum, &old, &new))
        }
        (IpAddr::V6(old), IpAddr::V6(new)) => {
            Ok(compute_inc(old_checksum, &old.segments(), &new.segments()))
        }
        _ => Err(IpAddrMismatchError.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_checksum_incrementally() {
        assert_eq!(0x0000, compute_inc(0xdd2f, &[0x5555], &[0x3285]));
    }
}
