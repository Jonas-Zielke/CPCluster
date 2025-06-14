fn parse_config_path<I: Iterator<Item = String>>(mut args: I) -> String {
    args.nth(1).unwrap_or_else(|| "config.json".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config_path = parse_config_path(std::env::args());
    cpcluster_masternode::run(&config_path).await
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
