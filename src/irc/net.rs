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

use std::{
    io::{self, Read},
    net::ToSocketAddrs,
    path::Path,
    usize,
};

use std::cmp::min;
use std::collections::VecDeque;
use std::io::Write;
use std::time::Duration;

use mio::net::TcpStream;
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;
use mio_signals::Signal;
use mio_signals::SignalSet;
use mio_signals::Signals;

use crate::{config::config_file::Config, MainError};

pub fn open_conn(conn_str: String) -> Result<TcpStream, io::Error> {
    let mut conn_details = conn_str.to_socket_addrs()?;
    let mut try_e = io::Error::new(io::ErrorKind::Other, "Should Never Happen.");
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
    let mut client = open_conn(config.connect_string())?;
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);
    let mut signals = Signals::new(SignalSet::all())?;

    let mut buffer = [0u8; 512 * 32];
    let mut writable_events: VecDeque<u8> = VecDeque::new();

    poll.registry().register(
        &mut client,
        IRC_CONN,
        Interest::READABLE | Interest::WRITABLE,
    )?;
    poll.registry()
        .register(&mut signals, SIGNAL_TOKEN, Interest::READABLE)?;

    'outer: loop {
        poll.poll(&mut events, Some(Duration::from_secs(1)))?;
        'inner: for event in &events {
            match event.token() {
                IRC_CONN => {
                    if event.is_readable() {
                        loop {
                            let s = match client.read(&mut buffer) {
                                // EOF
                                Ok(count) if count == 0 => break 'outer,
                                Ok(count) => count,
                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                                Err(e) => {
                                    return Err(e.into());
                                }
                            };
                            println!("{:?}", &buffer[..s]);
                            writable_events.extend(b"Hello, world!\r\n");
                            // we have stuff to write
                            poll.registry().reregister(
                                &mut client,
                                IRC_CONN,
                                Interest::READABLE | Interest::WRITABLE,
                            )?;
                        }
                    } else {
                        while !writable_events.is_empty() {
                            let m: usize = min((512 * 32) as usize, writable_events.len());
                            let wbuf = writable_events.drain(..m).collect::<Vec<u8>>();
                            match client.write(&wbuf) {
                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                    // no extend_front
                                    for byte in wbuf {
                                        writable_events.push_front(byte);
                                    }
                                    continue 'inner; // we have more to write
                                }
                                Err(e) => {
                                    return Err(e.into());
                                }
                                _ => (),
                            };
                        }
                        // We don't need writable events for now
                        poll.registry()
                            .reregister(&mut client, IRC_CONN, Interest::READABLE)?;
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
                _ => unreachable!(),
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
nick = "empty"
server = "localhost"
port = 9643
tls = false

[commands]
test = "./test"
"##;

    #[test]
    fn event_loop_test() {
        let inval = Path::new("testadsfads");
        let mut conf = Config::from_str(DEFAULT_CONF).unwrap();
        let serv = TcpListener::bind(conf.connect_string()).unwrap();
        let j = spawn(move || {
            let (mut stream, _) = serv.accept().unwrap();
            stream.write_all(b"test").unwrap();
            let mut b = [0u8; 64];
            let len = stream.read(&mut b).unwrap();
            assert_eq!(&b[0..len], b"Hello, world!\r\n");
        });

        event_loop(inval, &mut conf).unwrap();
        j.join().unwrap();
    }
}
