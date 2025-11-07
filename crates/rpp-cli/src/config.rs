use std::{net::Ipv4Addr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub rpp: RppSection,
    pub server: ServerSection,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RppSection {
    pub root_dir: PathBuf,
    pub plugins: Vec<String>,
    pub packs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerSection {
    pub host: Ipv4Addr,
    pub port: u16,
    pub default_pack: String,
}
/*
* # Config documentation can be found at https://rpp.oglass.dev/docs/config
[rpp]
root_dir = "{{ root }}"
plugins = [
    # Example: using a local plugin
    # "local:example"
]

[server]
host = "127.0.0.1"
port = 6741
default_pack = "idk"

*/
