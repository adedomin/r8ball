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

mod helpers;

use std::{
    cmp,
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use rand::{prelude::SmallRng, Rng, SeedableRng};

use crate::{
    config::config_file::Config,
    irc::{
        client::helpers::{case_cmp, join_channels, parse_cap},
        iter::TruncStatus,
        parse::Message,
    },
};

use super::{
    iter::BufIterator,
    plugin::{Plugin, PluginReadStat},
};

const BUF_SIZ: usize = 1024 * 16;

pub struct Client {
    pub state: State,
    // If we overrun this massive buffer, we have issues.
    read_buffer: [u8; BUF_SIZ],
    read_head: usize,
    write_buffer: VecDeque<u8>,
    rng: SmallRng,
}

#[derive(PartialEq)]
enum IrcState {
    Unknown,
    PreAuth,
    Authenticated,
    Ready(bool),
}

#[derive(PartialEq)]
pub enum CaseMapping {
    Ascii,
    Rfc1459,
    Unicode, // ???
}

pub struct State {
    pub nick: String,
    pub channels: Vec<String>,
    // Modes are detected at runtime since each server has different ones
    pub umode: HashSet<u8>,
    // This only tracks the modes related to administrative privileges
    // For instance, this tracks if a user is +v (voiced) or is +o (op).
    // Much like umodes, these vary from server to server and are detected
    // at runtime.
    // Some servers only support (vo)+@ or some support (vhoaq)+%@&~
    pub channel_modes: HashMap<String, u64>,
    // the state of the client
    // determins if we are ready to join channels
    // of if we have functioning mode tracking
    ready_state: IrcState,
    // the old name we expected to have
    original_nick: Option<String>,

    // This is state related to 005 command
    casemapping: CaseMapping,
    // list of channel prefixes that are valid. e.g. #&!
    chantypes: Vec<u8>,
    // e.g. +v maps to +, o maps to @, etc.
    mode_prefix: Vec<(u8, u8)>,
}

#[derive(Debug, PartialEq)]
pub enum IrcProto {
    Okay,
    Data,
    Error(String),
}

#[derive(Debug, PartialEq)]
pub enum ClientReadStat {
    Error(String),
    ReadBufferFull,
    HasWritableData,
    Blocked,
    Okay,
    Eof,
}

#[derive(Debug, PartialEq)]
pub enum ClientWriteStat {
    Blocked,
    Okay,
    Eof,
}

fn login_command(nick: &str, user: &str) -> String {
    format!(
        "CAP REQ :multi-prefix\r
NICK {0}\r
USER {1} +i * :{0}\r
",
        nick, user
    )
}

enum ModeType {
    Type1, // has a parameter
    Type2, // has a parameter
    Type3, // has a parameter if positive signed + (not -)
           // Type4, // This mode isn't relevant for our uses, effectively no parameter.
}

impl Client {
    pub fn new(config: &Config) -> Self {
        let state = State {
            nick: config.general.nick.clone(),
            channels: config.general.channels.clone(),
            umode: HashSet::new(),
            channel_modes: HashMap::new(),
            ready_state: IrcState::Unknown,
            original_nick: None,
            casemapping: CaseMapping::Rfc1459,
            chantypes: vec![b'#', b'&'],
            mode_prefix: vec![],
        };
        let rng_v = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut ret = Client {
            state,
            read_buffer: [0u8; BUF_SIZ],
            read_head: 0,
            write_buffer: VecDeque::with_capacity(BUF_SIZ),
            rng: SmallRng::seed_from_u64(rng_v),
        };
        // setup login write.
        ret.write_buffer
            .extend(login_command(&ret.state.nick, &ret.state.nick).as_bytes());
        ret
    }

    fn is_me(&self, msg: &Message) -> bool {
        if let Some(my_nick) = msg.nick {
            // Looks like the server changed my name.
            case_cmp(&self.state.casemapping, my_nick, self.state.nick.as_bytes())
        } else {
            false
        }
    }

    // or in modern words "direct message"
    fn is_private_message(&self, target: &[u8]) -> bool {
        case_cmp(&self.state.casemapping, target, self.state.nick.as_bytes())
    }

    fn handle_data(&mut self, len: usize) -> IrcProto {
        let mut ret = IrcProto::Okay;
        let mut partial_idx = 0usize;
        let mut partial_end = 0usize;

        let buf = &self.read_buffer[..len];
        let iter = BufIterator::new(buf);
        for line in iter {
            let msg = match line {
                TruncStatus::Full(data) => Message::new(data),
                TruncStatus::Part(data) => {
                    partial_idx = data.as_ptr() as usize - buf.as_ptr() as usize;
                    partial_end = data.len() + partial_idx;
                    break;
                }
            };
            if msg.is_empty() {
                continue;
            }

            if msg.nick.is_none() {
                match msg.command {
                    Some(cmd) if cmd == b"PING" => {
                        self.write_buffer.extend(b"PONG ");
                        if let Some(params) = msg.params {
                            self.write_buffer.extend(params)
                        }
                        self.write_buffer.extend(b"\r\n");
                        ret = IrcProto::Data;
                    }
                    Some(cmd) if cmd == b"ERROR" => {
                        if let Some(params) = msg.params {
                            let str_v = String::from_utf8_lossy(params);
                            return IrcProto::Error(str_v.to_string());
                        }
                        // quit the stream
                        self.write_buffer.extend(b"QUIT :bye\r\n");
                        ret = IrcProto::Data;
                    }
                    Some(cmd) => {
                        let str_v = String::from_utf8_lossy(cmd);
                        println!("WARN: Recv unknown command: {:?}", str_v);
                    }
                    // !is_empty implies this HAS to be Some()
                    None => unreachable!(),
                }

                continue;
            }

            match msg.command {
                Some(nick) if nick == b"NICK" => {
                    if let Some(my_nick) = msg.nick {
                        // Looks like the server changed my name.
                        if case_cmp(&self.state.casemapping, my_nick, self.state.nick.as_bytes()) {
                            let str_v = String::from_utf8_lossy(my_nick);
                            self.state.nick = str_v.to_string();
                            println!(
                                "INFO: The server changed our nick to: {:?}",
                                self.state.nick
                            );
                        }
                    }
                }
                Some(privmsg) if privmsg == b"PRIVMSG" => {
                    let mut params = msg.parameters();
                    match (msg.nick, params.next(), params.next()) {
                        (Some(nick), Some(target), Some(message)) => {
                            if self.is_private_message(&target) && message == b"\x01VERSION\x01" {
                                self.write_buffer.extend(b"NOTICE ");
                                self.write_buffer.extend(nick);
                                self.write_buffer.extend(b" :\x01r8ball: v0.0.0\x01\r\n");
                                ret = IrcProto::Data;
                            }
                        }
                        _ => (),
                    };
                }
                // :me JOIN #chan
                Some(join) if join == b"JOIN" => {
                    if self.is_me(&msg) {
                        if let Some(chan) = msg.parameters().next() {
                            let ch = String::from_utf8_lossy(chan).to_string();
                            self.state.channels.push(ch);
                        }
                    }
                }
                // :me PART #chan
                Some(part) if part == b"PART" => {
                    if self.is_me(&msg) {
                        if let Some(chan) = msg.parameters().next() {
                            self.state.channels.retain(|x| x.as_bytes() != chan);
                        }
                    }
                }
                // :the_kicker KICK #chan the_victim :reason
                Some(kick) if kick == b"KICK" => {
                    let mut params = msg.parameters();
                    match (params.next(), params.next()) {
                        (Some(channel), Some(victim)) => {
                            if case_cmp(&self.state.casemapping, victim, self.state.nick.as_bytes())
                            {
                                self.state.channels.retain(|x| x.as_bytes() != channel);
                                if let Some(reason) = params.next() {
                                    let channel = String::from_utf8_lossy(channel);
                                    let reason_given = String::from_utf8_lossy(reason);
                                    println!("Kicked from {}. reason: {}", channel, reason_given);
                                }
                            }
                        }
                        _ => (),
                    }
                }
                Some(invite) if invite == b"INVITE" => {}
                Some(identified) if identified == b"004" => {
                    self.state.ready_state = IrcState::Authenticated;
                    self.write_buffer
                        .extend(join_channels(&self.state.channels));
                    self.state.channels.clear(); // remove all channels, we re-add them when we get a JOIN
                }
                Some(isupport) if isupport == b"005" => {
                    self.state.ready_state = IrcState::Ready(true);
                    // todo!(); // parse ISUPPORT
                }
                // reply to NAMES(X) Command or message sent on joining a channel
                Some(names_repl) if names_repl == b"353" => {
                    //if self.state.ready_state == IrcState::Ready(true) {
                    //    todo!()
                    //}
                }
                // nickname collision
                Some(nick_col) if nick_col == b"433" || nick_col == b"436" => {
                    if self.state.original_nick.is_none() {
                        self.state.original_nick = Some(self.state.nick.clone());
                    }

                    self.state.nick.push('_');
                    for _ in 0..4 {
                        // generate a number that is in [0, 9)
                        let a: char = self.rng.gen_range('0'..':');
                        self.state.nick.push(a);
                    }

                    self.write_buffer
                        .extend(format!("NICK {}\r\n", self.state.nick).as_bytes());
                    println!("WARN: NICK COLLIDE; Trying new nick: {:?}", self.state.nick);
                    ret = IrcProto::Data;
                }
                Some(bad_pass) if bad_pass == b"464" => {
                    return IrcProto::Error("Invalid password given in PASS command.".to_owned());
                }
                Some(banned) if banned == b"465" => {
                    return IrcProto::Error("We are banned.".to_owned());
                }
                Some(cap) if cap == b"CAP" => {
                    if !parse_cap(&msg) {
                        return IrcProto::Error(
                            "We did not receive and ACK for multi-prefix".to_owned(),
                        );
                    } else {
                        self.write_buffer.extend(b"CAP END\r\n");
                        ret = IrcProto::Data;
                    }
                }
                Some(cap) if cap == b"903" => {
                    todo!() // implement sasl challenge & response
                }
                Some(cap)
                    if cap == b"902"
                        || cap == b"903"
                        || cap == b"904"
                        || cap == b"905"
                        || cap == b"906" =>
                {
                    return IrcProto::Error("We had an SASL problem.".to_owned());
                }
                Some(pong) if pong == b"PONG" => {
                    println!("DEBUG: PONG recv. TODO");
                }
                Some(any) => {
                    let str_n = if let Some(nick) = msg.nick {
                        String::from_utf8_lossy(nick).to_string()
                    } else {
                        "<NO NICK>".to_owned()
                    };
                    let str_c = String::from_utf8_lossy(any);
                    let str_p = if let Some(params) = msg.params {
                        String::from_utf8_lossy(params).to_string()
                    } else {
                        "".to_owned()
                    };
                    println!("Unknown command: {} {} {}", str_n, str_c, str_p);
                }
                None => unreachable!(),
            }
        }

        // move partial read to front of buffer, set read head up
        if partial_idx != partial_end {
            let edit = &mut self.read_buffer[..len];
            edit.copy_within(partial_idx..partial_end, 0);
            self.read_head = partial_end - partial_idx;
        } else {
            self.read_head = 0;
        }

        ret
    }

    pub fn receive_data<T: Read>(&mut self, readable: &mut T) -> Result<ClientReadStat, io::Error> {
        if self.read_head == self.read_buffer.len() {
            return Ok(ClientReadStat::ReadBufferFull);
        }

        let buf = &mut self.read_buffer[self.read_head..];
        let size = match readable.read(buf) {
            Ok(size) if size == 0 => return Ok(ClientReadStat::Eof),
            Ok(size) => size + self.read_head,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(ClientReadStat::Blocked),
            Err(e) => return Err(e),
        };

        match self.handle_data(size) {
            IrcProto::Okay => Ok(ClientReadStat::Okay),
            IrcProto::Data => Ok(ClientReadStat::HasWritableData),
            IrcProto::Error(e) => Ok(ClientReadStat::Error(e)),
        }
    }

    fn process_plugbuff(&mut self, plug: &mut Plugin) -> bool {
        let mut has_data = false;
        let mut has_trunc = false;
        let mut slice_at = 0usize;
        for line in plug.iter() {
            match line {
                // todo, implement command lang?
                TruncStatus::Full(data) => {
                    has_data = true;
                    self.write_buffer.extend(data);
                    self.write_buffer.extend(b"\r\n");
                }
                TruncStatus::Part(partial) => {
                    has_trunc = true;
                    slice_at = plug.get_slice_pos(partial);
                }
            }
        }

        if !has_trunc {
            plug.reset_buf();
            plug.split_at(slice_at);
        }

        has_data
    }

    pub fn process_plugin(&mut self, plug: &mut Plugin) -> io::Result<bool> {
        let mut has_data = false;
        loop {
            match plug.receive()? {
                PluginReadStat::Okay => (),
                PluginReadStat::Eof => break,
                PluginReadStat::Blocked => break,
                // buffer needs to processed to make progress
                PluginReadStat::ReadBufferFull => {
                    // If true, we have writable data
                    if self.process_plugbuff(plug) {
                        has_data = true;
                    }
                }
            }
        }
        if self.process_plugbuff(plug) {
            has_data = true;
        }
        Ok(has_data)
    }

    pub fn write_data<T: Write>(&mut self, writable: &mut T) -> Result<ClientWriteStat, io::Error> {
        if self.is_empty() {
            return Ok(ClientWriteStat::Eof);
        }

        let wlen = cmp::min(BUF_SIZ, self.write_buffer.len());
        let mut wbuf = self.write_buffer.drain(..wlen).collect::<Vec<u8>>();

        match writable.write(&wbuf) {
            Ok(size) if size != wlen => {
                let (_, unwritten) = wbuf.split_at(size);
                for &byte in unwritten.iter().rev() {
                    self.write_buffer.push_front(byte);
                }
                return Ok(ClientWriteStat::Okay);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // no extend_front
                wbuf.reverse();
                for byte in wbuf {
                    self.write_buffer.push_front(byte);
                }
                return Ok(ClientWriteStat::Blocked);
            }
            Err(e) => {
                return Err(e);
            }
            _ => (),
        };

        Ok(ClientWriteStat::Okay)
    }

    pub fn is_empty(&self) -> bool {
        self.write_buffer.is_empty()
    }
}

#[cfg(test)]
mod test {
    use std::io::{Cursor, Write};

    use crate::{config::config_file::Config, irc::parse::Message};

    use super::{Client, ClientReadStat, ClientWriteStat};

    const DEFAULT_CONF: &str = r##"
[general]
nick = "bot"
server = "localhost"
port = 9643
tls = false

[commands]
test = "./test"
"##;
    const DEFAULT_GREETER: &str = "CAP REQ :multi-prefix\r
NICK bot\r
USER bot +i * :bot\r
";

    #[test]
    fn irc_client_greeter() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        c.write_data(&mut fake_io).unwrap();
        assert_eq!(fake_io.get_ref(), DEFAULT_GREETER.as_bytes());
    }

    fn replace_with(cur: &mut Cursor<Vec<u8>>, data: Option<&[u8]>) {
        cur.get_mut().clear();
        cur.set_position(0);
        if let Some(data) = data {
            cur.write_all(data).unwrap();
        }
        cur.set_position(0);
    }

    fn read_expect(c: &mut Client, cur: &mut Cursor<Vec<u8>>, exp_res: ClientReadStat) {
        let status = c.receive_data(cur).unwrap();
        assert_eq!(status, exp_res);
        replace_with(cur, None);
    }

    fn write_expect(
        c: &mut Client,
        cur: &mut Cursor<Vec<u8>>,
        exp_res: ClientWriteStat,
        exp_data: &[u8],
    ) {
        let status = c.write_data(cur).unwrap();
        assert_eq!(status, exp_res);
        assert_eq!(cur.get_ref(), exp_data);
        replace_with(cur, None);
    }

    #[test]
    fn irc_client_ping_pong() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        c.write_data(&mut fake_io).unwrap();

        // test truncated while I'm at it. (the dangling P)
        replace_with(&mut fake_io, Some(b"PING :xyz\r\nPIN"));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);
        write_expect(
            &mut c,
            &mut &mut fake_io,
            ClientWriteStat::Okay,
            b"PONG :xyz\r\n",
        );
    }

    #[test]
    fn irc_client_truncations() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        c.write_data(&mut fake_io).unwrap();

        // test truncated while I'm at it. (the dangling P)
        replace_with(&mut fake_io, Some(b"PING :xyz\r\nPIN"));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);
        write_expect(
            &mut c,
            &mut &mut fake_io,
            ClientWriteStat::Okay,
            b"PONG :xyz\r\n",
        );

        // test truncation handling by writing out the rest
        replace_with(&mut fake_io, Some(b"G asdf\r\n"));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);
        write_expect(
            &mut c,
            &mut &mut fake_io,
            ClientWriteStat::Okay,
            b"PONG asdf\r\n",
        );

        // One more time
        replace_with(&mut fake_io, Some(b"PING :1234\r\n"));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);
        write_expect(
            &mut c,
            &mut &mut fake_io,
            ClientWriteStat::Okay,
            b"PONG :1234\r\n",
        );
    }

    #[test]
    fn irc_client_multiple_messages() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        // throw away greeter
        c.write_data(&mut fake_io).unwrap();

        let test_data = b"PING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\nPING :1234\r\n";
        let test_data_exp = b"PONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\nPONG :1234\r\n";
        replace_with(&mut fake_io, Some(test_data));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);
        write_expect(
            &mut c,
            &mut &mut fake_io,
            ClientWriteStat::Okay,
            test_data_exp,
        );
    }

    #[test]
    fn irc_client_unknown_cmd() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        // throw away greeter
        c.write_data(&mut fake_io).unwrap();

        replace_with(&mut fake_io, Some(b"UNKNOWN"));
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::Okay);
        write_expect(&mut c, &mut &mut fake_io, ClientWriteStat::Eof, b"");
    }

    #[test]
    fn irc_client_nick_conflict() {
        let conf = Config::from_str(DEFAULT_CONF).unwrap();
        let mut fake_io: Cursor<Vec<u8>> = Cursor::new(vec![]);
        let mut c = Client::new(&conf);
        // throw away greeter
        c.write_data(&mut fake_io).unwrap();

        replace_with(
            &mut fake_io,
            Some(b":bot!bot@bot.localhost 433 :name in use\r\n"),
        );
        read_expect(&mut c, &mut &mut fake_io, ClientReadStat::HasWritableData);

        let status = c.write_data(&mut fake_io).unwrap();
        assert_eq!(status, ClientWriteStat::Okay);
        let m = Message::new(&fake_io.get_ref()[..fake_io.get_ref().len() - 2]);
        assert_eq!(m.command.unwrap(), b"NICK");
        assert_eq!(&m.params.unwrap()[..4], b"bot_");
        assert_ne!(m.params.unwrap(), b"bot");
    }
}
