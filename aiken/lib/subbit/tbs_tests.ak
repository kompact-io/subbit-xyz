use subbit/fixtures as fx
use subbit/iou

test verify_accepts_matching_signature() {
  iou.verify(fx.iou_key, fx.tag, fx.owed_1000, fx.sig_for_1000)
}

test verify_rejects_wrong_amount() {
  !iou.verify(fx.iou_key, fx.tag, fx.owed_1000 + 1, fx.sig_for_1000)
}

test verify_rejects_wrong_tag() {
  !iou.verify(fx.iou_key, #"00", fx.owed_1000, fx.sig_for_1000)
}

test verify_rejects_wrong_key() {
  !iou.verify(fx.other_iou_key, fx.tag, fx.owed_1000, fx.sig_for_1000)
}

test verify_rejects_malformed_signature() {
  !iou.verify(fx.iou_key, fx.tag, fx.owed_1000, fx.bad_sig)
}
