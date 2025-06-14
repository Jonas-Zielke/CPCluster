use cpcluster_common::config::Config;
use cpcluster_common::JoinInfo;
use cpcluster_node::node::run;
use std::{env, error::Error, fs, io};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let join_data = fs::read_to_string("join.json")
        .map_err(|e| io::Error::new(e.kind(), format!("failed to read join.json: {}", e)))?;
    let mut join_info: JoinInfo = serde_json::from_str(&join_data)?;
    if let Ok(token) = env::var("CPCLUSTER_TOKEN") {
        join_info.token = token;
    }
    let config = Config::load("config.json").unwrap_or_default();
    run(join_info, config).await
}
