// Copyright (C) 2021  Anthony DeDominic <adedomin@gmail.com>

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use crate::irc::{client::CaseMapping, parse::Message};

macro_rules! hashmap {
    // map-like
    ($($k:expr => $v:expr),* $(,)?) => {
        std::iter::Iterator::collect(std::array::IntoIter::new([$(($k, $v),)*]))
    };
}

fn join_part_channels(command: &[u8], channels: &Vec<String>) -> Vec<u8> {
    let mut ret = vec![];
    let mut lsize = ret.len();
    let mut first = true;

    for channel in channels {
        if channel.len() + lsize >= 510 {
            lsize = 0usize;
            first = true;
            ret.extend(b"\r\n");
        }

        if !first {
            ret.push(b',');
        } else {
            ret.extend(command);
            ret.push(b' ');
            lsize = command.len();
            first = false;
        }
        ret.extend(channel.as_bytes());
        lsize += channel.len() + 1;
    }
    ret.extend(b"\r\n");

    ret
}

pub fn join_channels(channels: &Vec<String>) -> Vec<u8> {
    join_part_channels(b"JOIN", channels)
}

pub fn part_channels(channels: &Vec<String>) -> Vec<u8> {
    join_part_channels(b"PART", channels)
}

/// Uppercases a slice and returns a copy.
/// Note that this function currently only supports CASEMAPPING=ascii or CASEMAPPING=rfc1459
pub fn irc_uppercase(casemap: &CaseMapping, the_str: &[u8]) -> Vec<u8> {
    the_str
        .iter()
        .map(|&chr| match chr {
            b'a'..=b'z' => chr - 32u8,
            b'{'..=b'}' if *casemap == CaseMapping::Rfc1459 => chr - 32u8,
            b'^' if *casemap == CaseMapping::Rfc1459 => chr + 32,
            _ => chr,
        })
        .collect::<Vec<u8>>()
}

pub fn case_cmp(casemap: &CaseMapping, lhs: &[u8], rhs: &[u8]) -> bool {
    irc_uppercase(casemap, lhs) == irc_uppercase(casemap, rhs)
}

/// Parse the CAP command from the server
/// Messages usually look like -> :server CAP YOUR_NICK ACK :cap1 [cap2...]
/// We currently only handle ACK for multi-prefix with a future use of
/// sasl to come.
pub fn parse_cap(m: &Message) -> bool {
    let mut piter = m.parameters();

    // We throw away the nickmake parameter
    if piter.next().is_none() {
        return false; // we have an error.
    }
    if let Some(ack) = piter.next() {
        if ack != b"ACK" {
            return false;
        }
    } else {
        // not enough params
        return false;
    }

    if let Some(caplist) = piter.next() {
        caplist
            .split(|&chr| chr == b' ')
            .any(|cap| cap == b"multi-prefix")
    } else {
        false
    }
}

#[cfg(test)]
mod test {
    use rand::{prelude::SmallRng, Rng, SeedableRng};

    use crate::irc::{
        client::{helpers::case_cmp, CaseMapping},
        iter::{BufIterator, TruncStatus},
        parse::Message,
    };

    use super::join_channels;

    #[test]
    fn uppercase() {
        assert!(case_cmp(&CaseMapping::Rfc1459, b"^{|}", b"~[\\]"));
        assert!(case_cmp(&CaseMapping::Rfc1459, b"^{|}abc", b"~[\\]ABC"));
        assert!(!case_cmp(&CaseMapping::Ascii, b"^{|}abc", b"~[\\]ABC"));
    }

    #[test]
    fn mass_channel_join() {
        let mut prng = SmallRng::seed_from_u64(123456789);
        let mut channels = Vec::new();
        while channels.len() < 256 {
            let mut channel = "#".to_owned();
            for _ in 0..prng.gen_range(5..30) {
                channel.push(prng.gen_range('a'..'z'));
            }
            channels.push(channel);
        }

        let mut channels2: Vec<String> = Vec::new();
        let res = join_channels(&channels);
        for line in BufIterator::new(&res) {
            match line {
                TruncStatus::Full(msg) => {
                    assert!(msg.len() <= 512);
                    let m = Message::new(msg);
                    let list = m.parameters().next().unwrap();
                    for chan in list.split(|&chr| chr == b',') {
                        channels2.push(String::from_utf8_lossy(chan).to_string());
                    }
                }
                TruncStatus::Part(_) => panic!("shouldn't happen."),
            }
        }

        assert_eq!(channels.len(), channels2.len());
        for (lhs, rhs) in channels.iter().zip(channels2.iter()) {
            assert_eq!(lhs, rhs);
        }
    }
}
