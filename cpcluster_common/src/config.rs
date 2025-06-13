use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub min_port: u16,
    pub max_port: u16,
    pub failover_timeout_ms: u64,
    pub master_addresses: Vec<String>,
    pub ca_cert_path: Option<String>,
    pub ca_cert: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            min_port: 55001,
            max_port: 55999,
            failover_timeout_ms: 5000,
            master_addresses: vec!["127.0.0.1:55000".to_string()],
            ca_cert_path: None,
            ca_cert: None,
            cert_path: None,
            key_path: None,
        }
    }
}

impl Config {
    pub fn load(path: &str) -> std::io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data)
                .map_err(std::io::Error::other),
            Err(_) => Ok(Config::default()),
        }
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}
