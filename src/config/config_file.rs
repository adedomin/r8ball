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
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub general: General,
    // List of prefix and their associated plugins
    pub commands: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
pub struct General {
    pub nick: String,
    server: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_tls")]
    pub tls: bool,
    #[serde(default = "default_prefix")]
    pub command_prefix: String,
    #[serde(default)]
    server_password: String,
    #[serde(default)]
    sasl_password: String,
    #[serde(default)]
    pub nickserv_password: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub invite_file: String,
}

fn default_port() -> u16 {
    6667
}

fn default_prefix() -> String {
    ".!".to_string()
}

fn default_tls() -> bool {
    false
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("Could not open/read config file: {0}")]
    IO(#[from] io::Error),
    #[error("Could not parse config file: {0}")]
    Toml(#[from] toml::de::Error),
}

impl Config {
    pub fn from_str(c: &str) -> Result<Config, ConfigError> {
        toml::from_str::<Config>(&c).map_err(|e| e.into())
    }

    pub fn from_path(p: &Path) -> Result<Config, ConfigError> {
        let mut f = File::open(&p)?;
        let mut c = String::new();
        f.read_to_string(&mut c)?;
        Config::from_str(c.as_ref())
    }

    pub fn connect_string(&self) -> String {
        format!("{}:{}", self.general.server, self.general.port)
    }
}
