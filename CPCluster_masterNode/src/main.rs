fn parse_config_path<I: Iterator<Item = String>>(mut args: I) -> String {
    args.nth(1).unwrap_or_else(|| "config.json".to_string())
}

fn join_path() -> String {
    std::env::var("CPCLUSTER_JOIN").unwrap_or_else(|_| "join.json".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config_path = parse_config_path(std::env::args());
    let join = join_path();
    cpcluster_masternode::run(&config_path, &join).await
}

#[cfg(test)]
mod tests {
    use super::{join_path, parse_config_path};

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
}
