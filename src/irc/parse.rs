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

#[derive(PartialEq)]
enum ParseState {
    Prefix,
    Command,
    Params,
}

/// A non-general purpose IRCv2 parsed message.
/// This struct does not support tags as I do not use them or need them.
/// It also assumes the content is free of line delimiters.
/// This type was constructed to zero-copy view into a raw read buffer returned in parts
/// from crate::irc::iter::BufIterator.
pub struct Message<'a> {
    pub nick: Option<&'a [u8]>,
    pub user: Option<&'a [u8]>,
    pub host: Option<&'a [u8]>,
    pub command: Option<&'a [u8]>,
    params: Option<&'a [u8]>,
}

pub struct MessageParamIter<'a> {
    pos: usize,
    params: Option<&'a [u8]>,
}

impl<'a> Iterator for MessageParamIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(params) = self.params {
            if self.pos >= params.len() {
                return None;
            }

            let mut i = self.pos;
            // find the start of the next argument
            let start = loop {
                if i < params.len() {
                    if params[i] != b' ' {
                        break i;
                    }
                    i += 1;
                } else {
                    break params.len();
                }
            };

            // if the next argument is a trailing
            // collect the rest of the string and return
            if i < params.len() && params[i] == b':' {
                self.pos = params.len();
                return Some(&params[i + 1..]);
            }

            // else try and find the next separator or end of string
            let end = loop {
                if i < params.len() {
                    if params[i] == b' ' {
                        break i;
                    }
                    i += 1;
                } else {
                    break params.len();
                }
            };

            if start < end {
                self.pos = end + 1;
                Some(&params[start..end])
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl<'a> Default for Message<'a> {
    fn default() -> Self {
        Message {
            nick: None,
            user: None,
            host: None,
            command: None,
            params: None,
        }
    }
}

fn parse_prefix(b: &[u8]) -> (Option<&[u8]>, Option<&[u8]>, Option<&[u8]>) {
    let user_start = b.iter().position(|&chr| chr == b'!');
    let host_start = b.iter().position(|&chr| chr == b'@');
    match (user_start, host_start) {
        (None, None) => (Some(b), None, None),
        (None, Some(host)) => (Some(&b[0..host]), None, Some(&b[host + 1..])),
        (Some(user), None) => (Some(&b[0..user]), Some(&b[user + 1..]), None),
        // the expected path
        (Some(user), Some(host)) if user < host => (
            Some(&b[0..user]),
            Some(&b[user + 1..host]),
            Some(&b[host + 1..]),
        ),
        // this shouldn't happen, but it's not exactly hard to support it.
        // basically instead of x!y@z we got x@z!y
        (Some(user), Some(host)) => (
            Some(&b[0..host]),
            Some(&b[user + 1..]),
            Some(&b[host + 1..user]),
        ),
    }
}

impl<'a> Message<'a> {
    pub fn is_empty(&self) -> bool {
        self.nick == None
            && self.user == None
            && self.host == None
            && self.command == None
            && self.params == None
    }

    pub fn parameters(&self) -> MessageParamIter {
        MessageParamIter {
            pos: 0,
            params: self.params,
        }
    }

    pub fn new(raw: &'a [u8]) -> Self {
        let mut ret = Message::default();
        let mut arg_state = ParseState::Prefix;

        // this is the byte position the iterator has not consumed.
        let mut pos = 0usize;
        for part in raw.split(|&chr| chr == b' ') {
            pos += part.len() + 1;
            if part.is_empty() {
                continue;
            }

            arg_state = match arg_state {
                ParseState::Prefix => {
                    let has_prefix = if let Some(chr) = part.get(0) {
                        *chr == b':'
                    } else {
                        false
                    };
                    if has_prefix {
                        let (nick, user, host) = parse_prefix(&part[1..]);
                        ret.nick = nick;
                        ret.user = user;
                        ret.host = host;
                        ParseState::Command
                    } else {
                        ret.command = Some(part);
                        ParseState::Params
                    }
                }
                ParseState::Command => {
                    ret.command = Some(part);
                    ParseState::Params
                }
                ParseState::Params => {
                    ret.params = Some(&raw[pos - part.len() - 1..]);
                    break;
                }
            }
        }
        ret
    }
}

#[cfg(test)]
mod test {
    use super::Message;
    #[test]
    fn test_irc_message_parse_full() {
        let t1 = Message::new(b":happy!test@case command 1 2 3 :trailing param.");
        assert_eq!(t1.nick.unwrap_or(b""), b"happy");
        assert_eq!(t1.user.unwrap_or(b""), b"test");
        assert_eq!(t1.host.unwrap_or(b""), b"case");
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"1");
        assert_eq!(params[1], b"2");
        assert_eq!(params[2], b"3");
        assert_eq!(params[3], b"trailing param.");
    }

    #[test]
    fn test_irc_message_parse_no_prefix() {
        let t1 = Message::new(b"command 1 2 3 :trailing param.");
        assert_eq!(t1.nick, None);
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        // we test the remianing fields parse correctly
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"1");
        assert_eq!(params[1], b"2");
        assert_eq!(params[2], b"3");
        assert_eq!(params[3], b"trailing param.");
    }

    #[test]
    fn test_irc_message_parse_prefix_server() {
        let t1 = Message::new(b":some.irc.server command 1 2 3 :trailing param.");
        // we just put the server name into the nick field.
        assert_eq!(t1.nick.unwrap_or(b""), b"some.irc.server");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"1");
        assert_eq!(params[1], b"2");
        assert_eq!(params[2], b"3");
        assert_eq!(params[3], b"trailing param.");
    }

    #[test]
    fn test_irc_message_parse_prefix_user_host_swap() {
        let t1 = Message::new(b":happy@case!test command 1 2 3 :trailing param.");
        assert_eq!(t1.nick.unwrap_or(b""), b"happy");
        assert_eq!(t1.user.unwrap_or(b""), b"test");
        assert_eq!(t1.host.unwrap_or(b""), b"case");
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"1");
        assert_eq!(params[1], b"2");
        assert_eq!(params[2], b"3");
        assert_eq!(params[3], b"trailing param.");
    }

    #[test]
    fn test_irc_message_parse_prefix_blank() {
        let t1 = Message::new(b": com arg1 arg2");
        // an empty prefix means nick will point to an empty slice
        assert_eq!(t1.nick.unwrap_or(b"fafdsaf"), b"");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"com");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"arg1");
        assert_eq!(params[1], b"arg2");
    }

    #[test]
    fn test_irc_message_parse_prefix_no_user() {
        let t1 = Message::new(b":x@y com arg1 arg2");
        // an empty prefix means nick will point to an empty slice
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host.unwrap_or(b""), b"y");
        assert_eq!(t1.command.unwrap_or(b""), b"com");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"arg1");
        assert_eq!(params[1], b"arg2");
    }

    #[test]
    fn test_irc_message_parse_prefix_no_host() {
        let t1 = Message::new(b":x!y com arg1 arg2");
        // an empty prefix means nick will point to an empty slice
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user.unwrap_or(b""), b"y");
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"com");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"arg1");
        assert_eq!(params[1], b"arg2");
    }

    #[test]
    fn test_irc_message_parse_prefix_only() {
        let t1 = Message::new(b":x!y@z");
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user.unwrap_or(b""), b"y");
        assert_eq!(t1.host.unwrap_or(b""), b"z");
        assert_eq!(t1.command, None);
        assert!(t1.parameters().next().is_none());
    }

    #[test]
    fn test_irc_message_parse_command_only() {
        let t1 = Message::new(b"PING");
        assert_eq!(t1.nick, None);
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"PING");
        assert!(t1.parameters().next().is_none());
    }

    #[test]
    fn test_irc_message_parse_command_trailing_only() {
        let t1 = Message::new(b"PING : PONG");
        assert_eq!(t1.nick, None);
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"PING");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b" PONG");
    }

    #[test]
    fn test_irc_message_parse_command_trailing_blank() {
        let t1 = Message::new(b"PING :");
        assert_eq!(t1.nick, None);
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"PING");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"");
    }

    #[test]
    fn test_irc_message_parse_weird_spacing() {
        let t1 = Message::new(b":x     command    arg1  arg2        :     afdasfda  fdas   a .");
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"arg1");
        assert_eq!(params[1], b"arg2");
        assert_eq!(params[2], b"     afdasfda  fdas   a .");
    }

    #[test]
    fn test_irc_message_parse_weird_spacing_no_trailer() {
        let t1 = Message::new(b":x     command    arg1  arg2             afdasfda  fdas   a .  ");
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        let params = t1.parameters().collect::<Vec<&[u8]>>();
        assert_eq!(params[0], b"arg1");
        assert_eq!(params[1], b"arg2");
        assert_eq!(params[2], b"afdasfda");
        assert_eq!(params[3], b"fdas");
        assert_eq!(params[4], b"a");
        assert_eq!(params[5], b".");
    }

    #[test]
    fn test_irc_message_parse_weird_spacing_no_param() {
        let t1 = Message::new(b":x     command                 ");
        assert_eq!(t1.nick.unwrap_or(b""), b"x");
        assert_eq!(t1.user, None);
        assert_eq!(t1.host, None);
        assert_eq!(t1.command.unwrap_or(b""), b"command");
        assert!(t1.parameters().next().is_none());
    }

    #[test]
    fn test_irc_message_is_empty() {
        let t1 = Message::new(b"");
        assert!(t1.is_empty());
    }
}
