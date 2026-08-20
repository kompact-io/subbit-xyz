use super::{Backing, Channel, channel::Error};

use subbit_core::Iou;

/// Apply an already-verified `Iou`. Amount returned is the increase
/// over the previous commitment. Exposed — use with
/// `Db::update_exposed`.
pub fn apply_iou(iou: Iou) -> impl FnOnce(Channel) -> Result<(Channel, u64), Error> {
    move |mut channel| {
        let delta = channel.apply_iou(iou)?;
        Ok((channel, delta))
    }
}

/// Replace/top-up the channel's backing. No exposure — use with
/// `Db::update`.
pub fn apply_backing(backing: Option<Backing>) -> impl FnOnce(Channel) -> Result<Channel, Error> {
    move |mut channel| {
        channel.apply_backing(backing);
        Ok(channel)
    }
}

/// Checked spend — fails if `cost` exceeds what's spendable. `cost`
/// itself is the exposure: a lost spend that already triggered
/// external delivery/settlement is the same failure mode
/// `update_exposed` exists to bound. Use with `Db::update_exposed`.
pub fn apply_spend(cost: u64) -> impl FnOnce(Channel) -> Result<(Channel, u64), Error> {
    move |mut channel| {
        channel.apply_spend(cost)?;
        Ok((channel, cost))
    }
}

/// Refund `amount` off the running spent total. No exposure — use
/// with `Db::update`.
pub fn apply_refund(amount: u64) -> impl FnOnce(Channel) -> Result<Channel, Error> {
    move |mut channel| {
        channel.apply_refund(amount);
        Ok(channel)
    }
}
