use subbit_claim::client::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = std::env::args()
        .nth(1)
        .expect("usage: dump_summaries <base_url>");
    let client = Client::new(base.parse()?);
    let summaries = client
        .channel_summaries()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("total channels known to gate: {}", summaries.len());
    let active = summaries.values().filter(|s| s.is_active).count();
    let backed = summaries.values().filter(|s| s.backing.is_some()).count();
    println!("is_active=true: {active}, backing=Some: {backed}");
    let mut ious: Vec<u64> = summaries
        .values()
        .filter_map(|s| s.iou.as_ref().map(|i| i.amount()))
        .collect();
    ious.sort();
    println!("iou amounts (sorted): {ious:?}");
    Ok(())
}
