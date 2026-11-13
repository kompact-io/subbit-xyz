/// IOU failed verification against the channel's `iou_key`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("IOU failed verification")]
pub struct Error;
