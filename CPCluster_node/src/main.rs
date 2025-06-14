use cpcluster_common::config::Config;
use cpcluster_common::JoinInfo;
use cpcluster_node::node::run;
use std::{env, error::Error, fs};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let mut join_info: JoinInfo = serde_json::from_str(&fs::read_to_string("join.json")?)?;
    if let Ok(token) = env::var("CPCLUSTER_TOKEN") {
        join_info.token = token;
    }
    let config = Config::load("config.json").unwrap_or_default();
    run(join_info, config).await
}
