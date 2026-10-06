use anyhow::anyhow;
use serde_json::Value;

pub fn print_json(value: Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub fn drop_status(dropped: bool) -> &'static str {
    if dropped { "dropped" } else { "not_found" }
}

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
