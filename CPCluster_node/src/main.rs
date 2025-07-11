use cpcluster_common::config::Config;
use cpcluster_common::JoinInfo;
use cpcluster_node::node::run;
use std::{env, error::Error, fs, io};

const DEFAULT_CONFIG_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/config/config.json");
const DEFAULT_JOIN_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/join.json");

fn parse_config_path<I: Iterator<Item = String>>(mut args: I) -> String {
    args.next();
    for arg in args {
        if !arg.starts_with("--") {
            return arg;
        }
    }
    DEFAULT_CONFIG_PATH.to_string()
}

fn parse_log_level<I: Iterator<Item = String>>(mut args: I) -> Option<log::LevelFilter> {
    args.next();
    let mut expect_level = false;
    for arg in args {
        if expect_level {
            return arg.parse().ok();
        }
        if let Some(level) = arg.strip_prefix("--log-level=") {
            return level.parse().ok();
        }
        if arg == "--log-level" {
            expect_level = true;
        }
    }
    None
}

fn join_path() -> String {
    env::var("CPCLUSTER_JOIN").unwrap_or_else(|_| DEFAULT_JOIN_PATH.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();
    let cli_level = parse_log_level(args.clone().into_iter());
    let config_path = parse_config_path(args.clone().into_iter());
    let config = Config::load(&config_path).unwrap_or_default();
    let level = cli_level
        .or_else(|| config.log_level.as_deref().and_then(|l| l.parse().ok()))
        .unwrap_or(log::LevelFilter::Info);
    env_logger::Builder::new().filter_level(level).init();
    let path = join_path();
    let join_data = fs::read_to_string(&path)
        .map_err(|e| io::Error::new(e.kind(), format!("failed to read {}: {}", path, e)))?;
    let mut join_info: JoinInfo = serde_json::from_str(&join_data)?;
    if let Ok(token) = env::var("CPCLUSTER_TOKEN") {
        join_info.token = token;
    }
    run(join_info, config).await
}

#[cfg(test)]
mod tests {
    use super::{
        join_path, parse_config_path, parse_log_level, DEFAULT_CONFIG_PATH, DEFAULT_JOIN_PATH,
    };

    #[test]
    fn default_path() {
        let args = vec!["prog".to_string()];
        assert_eq!(parse_config_path(args.into_iter()), DEFAULT_CONFIG_PATH);
    }

    #[test]
    fn custom_path() {
        let args = vec!["prog".to_string(), "alt.json".to_string()];
        assert_eq!(parse_config_path(args.into_iter()), "alt.json");
    }

    #[test]
    fn config_after_flag() {
        let args = vec![
            "prog".to_string(),
            "--log-level=info".to_string(),
            "node.json".to_string(),
        ];
        assert_eq!(parse_config_path(args.into_iter()), "node.json");
    }

    #[test]
    fn join_env_default() {
        std::env::remove_var("CPCLUSTER_JOIN");
        assert_eq!(join_path(), DEFAULT_JOIN_PATH);
    }

    #[test]
    fn join_env_custom() {
        std::env::set_var("CPCLUSTER_JOIN", "test/join.json");
        assert_eq!(join_path(), "test/join.json");
        std::env::remove_var("CPCLUSTER_JOIN");
    }

    #[test]
    fn log_level_flag() {
        let args = vec![
            "prog".to_string(),
            "--log-level".to_string(),
            "trace".to_string(),
        ];
        assert_eq!(
            parse_log_level(args.into_iter()),
            Some(log::LevelFilter::Trace)
        );
    }

    #[test]
    fn log_level_equals() {
        let args = vec!["prog".to_string(), "--log-level=warn".to_string()];
        assert_eq!(
            parse_log_level(args.into_iter()),
            Some(log::LevelFilter::Warn)
        );
    }

    #[test]
    fn log_level_none() {
        let args = vec!["prog".to_string()];
        assert_eq!(parse_log_level(args.into_iter()), None);
    }
}
