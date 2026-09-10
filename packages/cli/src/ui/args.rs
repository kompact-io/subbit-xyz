use anyhow::anyhow;

pub fn parse_arg<T: std::str::FromStr>(raw: &str, what: &str) -> anyhow::Result<T> {
    raw.parse()
        .map_err(|_| anyhow!("couldn't parse {what} {raw:?}"))
}

pub fn resolve_json_arg(arg: &str) -> anyhow::Result<String> {
    match arg.strip_prefix('@') {
        Some(path) => Ok(std::fs::read_to_string(path)?),
        None => Ok(arg.to_string()),
    }
}
