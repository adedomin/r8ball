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
    process::{self, Child, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
};

use mio::{event::Source, unix::pipe};

use super::iter::BufIterator;

pub enum PluginReadStat {
    Okay,
    Eof,
    Blocked,
    ReadBufferFull,
}

/// An r8b plugin, its receiver and exit status.
pub struct Plugin {
    /// The exit status of the plugin.
    /// You can use the is_read_closed() event in mio to know when this field should be set.
    pub exit_code: Arc<Mutex<Option<io::Result<ExitStatus>>>>,
    read_buf: [u8; 512],
    read_start: usize,
    read_len: usize,
    pipe: pipe::Receiver,
    discard_out: bool,
}

impl Plugin {
    pub fn new(command: String, args: Vec<String>) -> io::Result<Self> {
        let (send, recv) = pipe::new()?;
        let exit_code = Arc::new(Mutex::new(None));
        let thread_ecode = exit_code.clone();

        thread::spawn(move || {
            let mut ecode = thread_ecode
                .lock()
                .expect("Could not lock plugin status field.");
            *ecode = Some(
                process::Command::new(command)
                    .stdin(Stdio::null())
                    .stderr(Stdio::inherit())
                    .stdout(unsafe { Stdio::from_raw_fd(send.into_raw_fd()) })
                    .args(args)
                    .spawn()
                    .and_then(|mut child: Child| -> io::Result<ExitStatus> { child.wait() }),
            );
        });

        Ok(Plugin {
            exit_code,
            read_buf: [0u8; 512],
            read_start: 0,
            read_len: 0,
            pipe: recv,
            discard_out: false,
        })
    }

    pub fn get_buf(&self) -> &[u8] {
        &self.read_buf[..self.read_len]
    }

    pub fn receive(&mut self) -> io::Result<PluginReadStat> {
        if self.read_len == self.read_buf.len() {
            // We cannot continue if the whole buffer cannot be processed
            // We check if it can be, else we attach a newline to the body.
            // this may cause gibberish to be sent to the server, but it is better
            // than deadlocking.
            if !self.read_buf.iter().any(|&chr| chr == b'\n') {
                self.read_buf
                    .last_mut()
                    .and_then(|refer: &mut u8| {
                        *refer = b'\n';
                        Some(())
                    })
                    .unwrap();
                // Because the rest of the output may have been broken by the above,
                // we set this flag that tells us to discard the remaining undelimited content.
                self.discard_out = true;
            }
            // We can't read more til we process this buffer.
            return Ok(PluginReadStat::ReadBufferFull);
        }

        if self.read_start != 0 {
            self.read_buf.copy_within(self.read_start..self.read_len, 0);
            self.read_len -= self.read_start;
            self.read_start = 0;
        }

        let size = match self.pipe.read(&mut self.read_buf[self.read_len..]) {
            Ok(s) if s == 0 => return Ok(PluginReadStat::Eof),
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(PluginReadStat::Blocked);
            }
            Err(e) => return Err(e),
        };

        if !self.discard_out {
            self.read_len += size;
        } else if self.discard_out {
            // We never increment read_len when discarding. safe to assume it is 0
            if let Some(pos) = self.read_buf[self.read_len..size]
                .iter()
                .position(|&chr| chr == b'\n')
            {
                self.read_buf.copy_within(pos..size, 0);
                // subtract the read size by the amount of bytes we cut off.
                self.read_len = size - pos;
                self.discard_out = false;
            }
        }

        Ok(PluginReadStat::Okay)
    }

    pub fn split_at(&mut self, pos: usize) {
        if pos == 0 {
            self.reset_buf();
        } else {
            self.read_start = pos;
        }
    }

    pub fn get_slice_pos(&self, slice: &[u8]) -> usize {
        self.read_buf.as_ptr() as usize - slice.as_ptr() as usize
    }

    pub fn iter(&self) -> BufIterator {
        BufIterator::new(&self.read_buf[..self.read_len])
    }

    pub fn reset_buf(&mut self) {
        self.read_len = 0;
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

    use crate::irc::{iter::TruncStatus, parse::Message, plugin::PluginReadStat};

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
                                    PluginReadStat::ReadBufferFull => break,
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

    #[test]
    fn large_output_truncation() {
        let plugin_file = format!(
            "{}/examples/plugins/big_output.sh",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut plug = Plugin::new(plugin_file, vec![]).unwrap();

        loop {
            match plug.receive().unwrap() {
                PluginReadStat::Okay => (),
                PluginReadStat::Eof => break,
                PluginReadStat::Blocked => (),
                PluginReadStat::ReadBufferFull => {
                    for out in plug.iter() {
                        match out {
                            TruncStatus::Full(out) => {
                                let m = Message::new(out);
                                let p = m.parameters().collect::<Vec<&[u8]>>();

                                assert_eq!(m.command.as_deref(), Some(&b"PRIVMSG"[..]));
                                assert_eq!(p[0], b"#test");
                                // trailing a should not be in this message.
                                assert!(!p[1].iter().any(|&chr| chr != b' '));
                            }
                            TruncStatus::Part(_) => {
                                panic!("We should have truncated output and appended a newline!")
                            }
                        };
                    }
                    plug.reset_buf();
                }
            }
        }

        for out in plug.iter() {
            match out {
                TruncStatus::Full(out) => {
                    let m = Message::new(out);
                    let p = m.parameters().collect::<Vec<&[u8]>>();

                    assert_eq!(m.command.as_deref(), Some(&b"PRIVMSG"[..]));
                    assert_eq!(p[0], b"#test");
                    assert_eq!(p[1], b"Hello, World!");
                }
                TruncStatus::Part(_) => {
                    panic!("We should have truncated output and appended a newline!")
                }
            }
        }
    }

    #[test]
    fn test_partial_trunc() {
        let plugin_file = format!(
            "{}/examples/plugins/truncated_read.sh",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut plug = Plugin::new(plugin_file, vec![]).unwrap();

        loop {
            match plug.receive().unwrap() {
                PluginReadStat::Okay => (),
                PluginReadStat::Eof => break,
                PluginReadStat::Blocked => (),
                PluginReadStat::ReadBufferFull => {
                    let mut split_at = 0usize;
                    for out in plug.iter() {
                        match out {
                            TruncStatus::Full(out) => {
                                let m = Message::new(out);
                                let p = m.parameters().collect::<Vec<&[u8]>>();

                                assert_eq!(m.command.as_deref(), Some(&b"PRIVMSG"[..]));
                                assert_eq!(p[0], b"#test");
                                // trailing a should not be in this message.
                                assert!(!p[1]
                                    .iter()
                                    .last()
                                    .and_then(|&chr| if chr == b'a' { Some(()) } else { None })
                                    .is_some());
                            }
                            TruncStatus::Part(out) => {
                                split_at = plug.get_slice_pos(out);
                            }
                        };
                    }
                    // we should have truncated data.
                    assert!(split_at != 0);
                    plug.split_at(split_at);
                }
            }
        }

        for out in plug.iter() {
            match out {
                TruncStatus::Full(out) => {
                    let m = Message::new(out);
                    let p = m.parameters().collect::<Vec<&[u8]>>();

                    assert_eq!(m.command.as_deref(), Some(&b"PRIVMSG"[..]));
                    assert_eq!(p[0], b"#test");
                    assert_eq!(p[1], b"Hello, World!");
                }
                TruncStatus::Part(_) => {
                    panic!("We should not have truncated output!")
                }
            }
        }
    }
}
