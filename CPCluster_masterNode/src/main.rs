fn parse_config_path<I: Iterator<Item = String>>(mut args: I) -> String {
    args.next();
    for arg in args {
        if !arg.starts_with("--") {
            return arg;
        }
    }
    "config.json".to_string()
}

fn parse_log_level<I: Iterator<Item = String>>(mut args: I) -> log::LevelFilter {
    args.next();
    let mut expect_level = false;
    while let Some(arg) = args.next() {
        if expect_level {
            return arg.parse().unwrap_or(log::LevelFilter::Info);
        }
        if let Some(level) = arg.strip_prefix("--log-level=") {
            return level.parse().unwrap_or(log::LevelFilter::Info);
        }
        if arg == "--log-level" {
            expect_level = true;
        }
    }
    log::LevelFilter::Info
}

fn join_path() -> String {
    std::env::var("CPCLUSTER_JOIN").unwrap_or_else(|_| "join.json".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = parse_config_path(args.clone().into_iter());
    let level = parse_log_level(args.into_iter());
    env_logger::Builder::new().filter_level(level).init();
    let join = join_path();
    cpcluster_masternode::run(&config_path, &join).await
}

#[cfg(test)]
mod tests {
    use super::{join_path, parse_config_path, parse_log_level};

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

    #[test]
    fn config_after_flag() {
        let args = vec![
            "prog".to_string(),
            "--log-level=debug".to_string(),
            "master.json".to_string(),
        ];
        assert_eq!(parse_config_path(args.into_iter()), "master.json");
    }

    #[test]
    fn join_env_default() {
        std::env::remove_var("CPCLUSTER_JOIN");
        assert_eq!(join_path(), "join.json");
    }

    #[test]
    fn join_env_custom() {
        std::env::set_var("CPCLUSTER_JOIN", "data/join.json");
        assert_eq!(join_path(), "data/join.json");
        std::env::remove_var("CPCLUSTER_JOIN");
    }

    #[test]
    fn log_level_flag() {
        let args = vec![
            "prog".to_string(),
            "--log-level".to_string(),
            "debug".to_string(),
        ];
        assert_eq!(parse_log_level(args.into_iter()), log::LevelFilter::Debug);
    }

    #[test]
    fn log_level_equals() {
        let args = vec!["prog".to_string(), "--log-level=error".to_string()];
        assert_eq!(parse_log_level(args.into_iter()), log::LevelFilter::Error);
    }
}
