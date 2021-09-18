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

use std::collections::HashMap;
use std::{io, net::ToSocketAddrs, path::Path};

use std::time::Duration;

use mio::net::TcpStream;
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;
use mio_signals::Signal;
use mio_signals::SignalSet;
use mio_signals::Signals;

use crate::irc::client::{ClientReadStat, ClientWriteStat};
use crate::{config::config_file::Config, MainError};

use super::client::Client;
use super::plugin::Plugin;

fn open_conn(conn_str: String) -> Result<TcpStream, io::Error> {
    let mut conn_details = conn_str.to_socket_addrs()?;
    let mut try_e = io::Error::new(io::ErrorKind::Other, "Should} Never Happen.");
    Ok(loop {
        if let Some(addr) = conn_details.next() {
            match TcpStream::connect(addr) {
                Ok(conn) => break conn,
                Err(e) => try_e = e,
            }
        } else {
            return Err(try_e);
        }
    })
}

const IRC_CONN: mio::Token = Token(0);
const SIGNAL_TOKEN: mio::Token = Token(1);

pub fn event_loop(config_path: &Path, config: &mut Config) -> Result<(), MainError> {
    let mut conn = open_conn(config.connect_string())?;
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);
    let mut signals = Signals::new(SignalSet::all())?;

    let mut irc_client = Client::new(config);
    let mut plugin_recv = HashMap::<Token, Plugin>::new();

    poll.registry()
        .register(&mut conn, IRC_CONN, Interest::READABLE | Interest::WRITABLE)?;
    poll.registry()
        .register(&mut signals, SIGNAL_TOKEN, Interest::READABLE)?;

    'outer: loop {
        poll.poll(&mut events, Some(Duration::from_secs(1)))?;
        for event in &events {
            match event.token() {
                IRC_CONN => {
                    if event.is_readable() {
                        loop {
                            match irc_client.receive_data(&mut conn)? {
                                ClientReadStat::ReadBufferFull => panic!(
                                    "Our read buffer is full and we aren't processing events!"
                                ),
                                ClientReadStat::HasWritableData => {
                                    // we have stuff to write
                                    poll.registry().reregister(
                                        &mut conn,
                                        IRC_CONN,
                                        Interest::READABLE | Interest::WRITABLE,
                                    )?;
                                    break;
                                }
                                ClientReadStat::Blocked => break,
                                ClientReadStat::Okay => (),
                                ClientReadStat::Eof => break 'outer,
                                ClientReadStat::Error(err) => return Err(MainError::IrcProto(err)),
                            }
                        }
                    } else if event.is_writable() {
                        loop {
                            match irc_client.write_data(&mut conn)? {
                                ClientWriteStat::Blocked => break,
                                ClientWriteStat::Okay => (),
                                ClientWriteStat::Eof => {
                                    poll.registry().reregister(
                                        &mut conn,
                                        IRC_CONN,
                                        Interest::READABLE,
                                    )?;
                                    break;
                                }
                            }
                        }
                    } else {
                        break 'outer;
                    }
                }
                SIGNAL_TOKEN => loop {
                    match signals.receive()? {
                        Some(Signal::Interrupt) | Some(Signal::Terminate) | Some(Signal::Quit) => {
                            break 'outer
                        }
                        Some(Signal::User1) | Some(Signal::User2) => {
                            *config = Config::from_path(config_path)?;
                            println!("{:?}", config);
                        }
                        None => break,
                    }
                },
                _ => {
                    let ev_tok = event.token();
                    if let Some(plug) = plugin_recv.get_mut(&ev_tok) {
                        // If true, we have writable data
                        if irc_client.process_plugin(plug)? {
                            poll.registry().reregister(
                                &mut conn,
                                IRC_CONN,
                                Interest::READABLE | Interest::WRITABLE,
                            )?;
                        }

                        if event.is_read_closed() {
                            plugin_recv.remove(&ev_tok).expect("Cannot remove plugin!");
                        }
                    } else {
                        panic!("We got a token that we should not have!");
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::Path,
        thread::spawn,
    };

    use crate::config::config_file::Config;

    use super::event_loop;

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
    fn event_loop_test() {
        let inval = Path::new("testadsfads");
        let mut conf = Config::from_str(DEFAULT_CONF).unwrap();
        let serv = TcpListener::bind(conf.connect_string()).unwrap();
        let j = spawn(move || {
            let (mut stream, _) = serv.accept().unwrap();
            let mut b = [0u8; 64];
            let len = stream.read(&mut b).unwrap();
            assert_eq!(&b[0..len], DEFAULT_GREETER.as_bytes());
            stream.write_all(b"PING :xyz\r\n").unwrap();
            let len = stream.read(&mut b).unwrap();
            assert_eq!(&b[0..len], b"PONG :xyz\r\n");
        });

        event_loop(inval, &mut conf).unwrap();
        j.join().unwrap();
    }
}
