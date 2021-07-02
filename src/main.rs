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
use std::path::Path;

use config::cmdline::{ParsedArgs, ParsedArgsError};
use config::config_file::{Config, ConfigError};
use irc::net::event_loop;

#[derive(thiserror::Error, Debug)]
pub enum MainError {
    #[error("")]
    Cmdline(#[from] ParsedArgsError),
    #[error("")]
    Config(#[from] ConfigError),
    #[error("")]
    EvIo(#[from] io::Error),
    #[error("ERROR: {0}")]
    IrcProto(String),
}

fn main() -> Result<(), MainError> {
    let args = ParsedArgs::new()?;
    let config_path = Path::new(&args.config);
    let mut config = Config::from_path(config_path)?;
    event_loop(config_path, &mut config)?;

    Ok(())
}
