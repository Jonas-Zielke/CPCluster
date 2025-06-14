use crate::NodeRole;
use serde::{Deserialize, Serialize};
use std::{error::Error, fs};

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
    /// Ports bound by Internet nodes for outgoing network requests
    pub internet_ports: Option<Vec<u16>>,
    /// Directory used for on-disk storage by nodes
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,
    #[serde(default = "default_state_file")]
    pub state_file: String,
    #[serde(default = "default_disk_space")]
    pub disk_space_mb: u64,
    #[serde(default)]
    pub role: NodeRole,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
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
            internet_ports: None,
            storage_dir: default_storage_dir(),
            state_file: default_state_file(),
            disk_space_mb: default_disk_space(),
            role: NodeRole::Worker,
            max_retries: default_max_retries(),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> std::io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).map_err(std::io::Error::other),
            Err(_) => Ok(Config::default()),
        }
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }
}

fn default_storage_dir() -> String {
    "storage".to_string()
}

fn default_disk_space() -> u64 {
    1024
}

fn default_max_retries() -> u32 {
    5
}

fn default_state_file() -> String {
    "master_state.json".to_string()
}
