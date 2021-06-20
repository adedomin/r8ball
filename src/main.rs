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

mod config;
mod irc;

use std::io;
use std::io::Read;
use std::io::Write;
use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::Duration;

use config::cmdline::{ParsedArgs, ParsedArgsError};
use config::config_file::{Config, ConfigError};
use mio::net::TcpStream;
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;
use mio_signals::SignalSet;
use mio_signals::Signals;

#[derive(thiserror::Error, Debug)]
enum MainError {
    #[error("")]
    Cmdline(#[from] ParsedArgsError),
    #[error("")]
    InitConfig(#[from] ConfigError),
    #[error("Event Loop IO error.")]
    EvIo(io::Result<()>),
}

const IRC_CONN: mio::Token = Token(0);
const SIGNAL_TOKEN: mio::Token = Token(1);

fn open_conn(conn_str: String) -> Result<TcpStream, io::Error> {
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

fn event_loop(config_path: &Path, config: &mut Config) -> io::Result<()> {
    let mut client = open_conn(config.connect_string())?;
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);
    let mut signals = Signals::new(SignalSet::all())?;

    poll.registry()
        .register(&mut client, IRC_CONN, Interest::WRITABLE)?;
    poll.registry()
        .register(&mut signals, SIGNAL_TOKEN, Interest::READABLE)?;

    'outer: loop {
        poll.poll(&mut events, Some(Duration::from_secs(1)))?;
        for event in &events {
            let mut interest = Interest::READABLE;
            match event.token() {
                IRC_CONN => {
                    if event.is_readable() {
                        let mut b = [0u8; 512];
                        let s = client.read(&mut b)?;
                        if s == 0 {
                            break 'outer;
                        }
                        println!("{:?}", &b[..s]);
                        interest |= Interest::WRITABLE;
                    } else {
                        client.write_all(b"Hello. World\r\n")?
                    }
                }
                SIGNAL_TOKEN => break 'outer,
                _ => unreachable!(),
            }
            poll.registry()
                .reregister(&mut client, IRC_CONN, interest)?;
        }
    }
    Ok(())
}

fn main() -> Result<(), MainError> {
    let args = ParsedArgs::new()?;
    let config_path = Path::new(&args.config);
    let mut config = Config::from_path(config_path)?;
    event_loop(config_path, &mut config).map_err(|e| MainError::EvIo(Err(e)))?;

    Ok(())
}
