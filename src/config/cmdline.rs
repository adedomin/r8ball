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

use core::fmt;
use std::env;

use ParseState::{Boolarg, Config, LogFile};

const HELP_MESSAGE: &str = r#"neo8ball [-c|--config=] [-o|--log-output=] [-t|--timestamp] [-h|--help]

-c --config=str       The Config File to use.
-o --log-output=str   Log Output to file instead of stdout.
-t --timestamp        Timestamp logs using RFC 3339. (YYYY-MM-DD HH:MM:SS[+/-TZ]).
-h --help             This message.
"#;

#[derive(PartialEq)]
enum ParseState {
    Boolarg,
    Config,
    LogFile,
}

#[derive(thiserror::Error, Debug)]
pub struct ParsedArgsError(String);

impl fmt::Display for ParsedArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct ParsedArgs {
    pub config: String,
    pub log_file: String,
    pub timestamp_logs: bool,
    pub mock: bool,
}

impl Default for ParsedArgs {
    fn default() -> Self {
        ParsedArgs {
            config: "./r8ball.conf".to_owned(),
            log_file: "".to_owned(),
            timestamp_logs: false,
            mock: false,
        }
    }
}

impl ParsedArgs {
    pub fn new() -> Result<ParsedArgs, ParsedArgsError> {
        let mut ret = ParsedArgs::default();
        let mut arg_state = ParseState::Boolarg;
        let mut itr = env::args();
        itr.next(); // throw away first arg
        for arg in itr {
            let (flag, val) = if arg_state != Boolarg {
                (arg.as_str(), "")
            } else if let Some(idx) = arg.as_str().find('=') {
                arg.split_at(idx + 1usize)
            } else {
                (arg.as_str(), "")
            };

            arg_state = match flag {
                "-t" | "--timestamp" => {
                    ret.timestamp_logs = true;
                    Boolarg
                }
                "-c" | "--config" => Config,
                "--config=" => {
                    ret.config = val.to_string();
                    Boolarg
                }
                "-o" | "--log-output" => LogFile,
                "--log-output=" => {
                    ret.log_file = val.to_string();
                    Boolarg
                }
                "-h" | "--help" => return Err(ParsedArgsError(HELP_MESSAGE.to_string())),
                _ => match arg_state {
                    Boolarg => {
                        return Err(ParsedArgsError(format!(
                            "Unknown option passed ({}), see --help",
                            flag,
                        )))
                    }
                    Config => {
                        ret.config = flag.to_string();
                        Boolarg
                    }
                    LogFile => {
                        ret.log_file = flag.to_string();
                        Boolarg
                    }
                },
            }
        }
        Ok(ret)
    }
}
