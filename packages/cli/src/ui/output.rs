use serde_json::Value;

pub fn print_json(value: Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub fn drop_status(dropped: bool) -> &'static str {
    if dropped { "dropped" } else { "not_found" }
}
