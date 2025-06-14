use cpcluster_common::config::Config;
use cpcluster_common::JoinInfo;
use cpcluster_node::node::run;
use std::{env, error::Error, fs, io};

fn parse_config_path<I: Iterator<Item = String>>(mut args: I) -> String {
    args.nth(1).unwrap_or_else(|| "config.json".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let join_data = fs::read_to_string("join.json")
        .map_err(|e| io::Error::new(e.kind(), format!("failed to read join.json: {}", e)))?;
    let mut join_info: JoinInfo = serde_json::from_str(&join_data)?;
    if let Ok(token) = env::var("CPCLUSTER_TOKEN") {
        join_info.token = token;
    }
    let config_path = parse_config_path(env::args());
    let config = Config::load(&config_path).unwrap_or_default();
    run(join_info, config).await
}

#[cfg(test)]
mod tests {
    use super::parse_config_path;

    #[test]
    fn default_path() {
        let args = vec!["prog".to_string()];
        assert_eq!(parse_config_path(args.into_iter()), "config.json");
    }

    #[test]
    fn custom_path() {
        let args = vec!["prog".to_string(), "alt.json".to_string()];
        assert_eq!(parse_config_path(args.into_iter()), "alt.json");
    }
}
