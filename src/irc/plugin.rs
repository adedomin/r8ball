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
    os::unix::prelude::{FromRawFd, IntoRawFd},
    process::{self, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
};

use mio::{event::Source, unix::pipe};

use super::iter::BufIterator;

pub enum PluginReadStat {
    Okay,
    Eof,
    Blocked,
}

/// An r8b plugin, its receiver and exit status.
pub struct Plugin {
    /// The exit status of the plugin.
    /// You can use the is_read_closed() event in mio to know when this field should be set.
    pub exit_code: Arc<Mutex<Option<io::Result<ExitStatus>>>>,
    pub read_buf: [u8; 512],
    read_len: usize,
    pipe: pipe::Receiver,
}

impl Plugin {
    pub fn new(command: String, args: Vec<String>) -> io::Result<Self> {
        let (send, recv) = pipe::new()?;
        let exit_code = Arc::new(Mutex::new(None));
        let ecode_2 = exit_code.clone();

        thread::spawn(move || {
            let ecode = match process::Command::new(command)
                .stdin(Stdio::null())
                .stderr(Stdio::inherit())
                .stdout(unsafe { Stdio::from_raw_fd(send.into_raw_fd()) })
                .args(args)
                .spawn()
            {
                Ok(mut child) => child.wait(),
                Err(e) => Err(e),
            };

            let mut e = ecode_2.lock().expect("Could not lock plugin status field.");
            *e = Some(ecode);
        });

        Ok(Plugin {
            exit_code,
            read_buf: [0u8; 512],
            read_len: 0,
            pipe: recv,
        })
    }

    pub fn receive(&mut self) -> io::Result<PluginReadStat> {
        let size = match self.pipe.read(&mut self.read_buf[self.read_len..]) {
            Ok(s) if s == 0 => return Ok(PluginReadStat::Eof),
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(PluginReadStat::Blocked);
            }
            Err(e) => return Err(e),
        };

        self.read_len += size;

        Ok(PluginReadStat::Okay)
    }

    pub fn iter(&self) -> BufIterator {
        BufIterator::new(&self.read_buf[..self.read_len])
    }

    /// Useful helper to move line content which is not delimited.
    pub fn move_to_front(&mut self, slice: &[u8]) {
        let start = self.read_buf.as_ptr() as usize - slice.as_ptr() as usize;
        let mv = &mut self.read_buf[0..slice.len() + start];
        mv.copy_within(start.., 0);
        // truncate remaining content.
        self.read_len = slice.len();
    }
}

impl Source for Plugin {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        registry.register(&mut self.pipe, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        registry.reregister(&mut self.pipe, token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        registry.deregister(&mut self.pipe)
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::irc::{iter::TruncStatus, plugin::PluginReadStat};

    use super::Plugin;
    use mio::{Events, Interest, Poll, Token};

    #[test]
    fn spawn_example_and_read() {
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1);
        let plugin_file = format!("{}/examples/plugins/test.sh", env!("CARGO_MANIFEST_DIR"));
        let mut plug = Plugin::new(plugin_file, vec!["--reply=#chan".to_owned()]).unwrap();

        let tok = Token(127);

        poll.registry()
            .register(&mut plug, tok, Interest::READABLE)
            .unwrap();

        'outer: loop {
            poll.poll(&mut events, Some(Duration::from_secs(10)))
                .unwrap();

            for event in events.iter() {
                match event.token() {
                    Token(127) => {
                        if event.is_readable() {
                            loop {
                                match plug.receive().unwrap() {
                                    PluginReadStat::Okay => (),
                                    PluginReadStat::Eof => break 'outer,
                                    PluginReadStat::Blocked => break,
                                }
                            }
                        } else if event.is_read_closed() {
                            match plug.exit_code.lock().unwrap().as_ref().unwrap() {
                                Ok(status) => assert_eq!(status.code(), Some(0)),
                                Err(e) => panic!("Our Plugin had an io::Error: {:?}", e),
                            }
                            break 'outer;
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }

        let mut has_output = false;
        for msg in plug.iter() {
            if let TruncStatus::Full(m) = msg {
                has_output = true;
                assert_eq!(m, b"PRIVMSG #chan :Hello, World!");
            } else {
                panic!("truncated output.");
            }
        }
        assert!(has_output);
    }
}
