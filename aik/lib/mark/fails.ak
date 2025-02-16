use aiken/interval.{Finite, IntervalBound}
use subbit/logic.{do_add, do_close, do_end, do_expire, do_settle, do_sub}
use subbit/types as t

/// Testing expected failures
const c =
  t.Constants {
    subbit_id: #"b10da48cea4c0967",
    currency: t.ByHash(
      #"1234567890abcdef1234567890abcde1234567890abcdef123456789",
    ),
    iou_key: #"cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
    consumer: #"00000000000000000000000000000000000000000000000000000000",
    provider: #"00000000000000000000000000000000000000000000000000000001",
    close_period: 123456789,
  }

// This is lifted from ./iou.ak and belongs to the iou_key above 
// with corresponding subbit key
const iou =
  t.Iou {
    amount: 1482817706323250795,
    signature: #"7f96d963c32e225ae0a1fe1491466f277cd4cde77cd4f8335578fd888b61942af64972e02efc3a603241f2d84bb5df95b18f57e2f88e4d63c249716b068d8d01",
  }

test add_less() fail {
  // An add step in which the continuing output has less funds.
  let signers = [c.consumer]
  let funds_in = 1000
  let stage_in = t.Opened(c, 666)
  let funds_out = 1000
  let stage_out = t.Opened(c, 666)
  do_add(signers, funds_in, stage_in, funds_out, stage_out)
}

test add_not_singed() fail {
  let signers = [c.provider]
  let funds_in = 1000
  let stage_in = t.Opened(c, 666)
  let funds_out = 1000
  let stage_out = t.Opened(c, 667)
  do_add(signers, funds_in, stage_in, funds_out, stage_out)
}

test sub_too_much() fail {
  // A sub that in which the continuing output has less funds than should be allowed by the iou
  let signers = [c.provider]
  let funds_in = 1000
  let stage_in = t.Opened(c, iou.amount - 500)
  let funds_out = 0
  let stage_out = t.Opened(c, iou.amount + 500)
  do_sub(signers, funds_in, stage_in, funds_out, stage_out, iou)
}

test sub_not_signed() fail {
  let signers = [c.consumer]
  let funds_in = 1000
  let stage_in = t.Opened(c, iou.amount - 1000)
  let funds_out = 0
  let stage_out = t.Opened(c, iou.amount)
  do_sub(signers, funds_in, stage_in, funds_out, stage_out, iou)
}

test sub_bad_id() fail {
  // A sub in which the iou is for a different subbit
  let c = t.Constants { ..c, subbit_id: #"1234567890abcdef1234567890abcdef" }
  let signers = [c.provider]
  let funds_in = 2000
  let stage_in = t.Opened(c, iou.amount - 1000)
  let funds_out = 1000
  let stage_out = t.Opened(c, iou.amount)
  do_sub(signers, funds_in, stage_in, funds_out, stage_out, iou)
}

test sub_bad_sig() fail {
  // A sub in which the iou has a bad id
  let c =
    t.Constants {
      ..c,
      iou_key: #"cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d00000000",
    }
  let signers = [c.provider]
  let funds_in = 2000
  let stage_in = t.Opened(c, iou.amount - 1000)
  let funds_out = 1000
  let stage_out = t.Opened(c, iou.amount)
  do_sub(signers, funds_in, stage_in, funds_out, stage_out, iou)
}

test close_bad_data() fail {
  // A close in which the continuing output datum has changed, other than the stage
  let signers = [c.consumer]
  let funds_in = 2000
  let stage_in = t.Opened(c, 1000)
  let funds_out = 2000
  let upper_bound = 1234567890
  let expire_at = upper_bound + c.close_period + 1
  let stage_out = t.Closed(c, 999, expire_at)
  do_close(
    signers,
    IntervalBound(Finite(upper_bound), False),
    funds_in,
    stage_in,
    funds_out,
    stage_out,
  )
}

test close_bad_expire() fail {
  // A close in which the expire at timestamp is too soon
  let signers = [c.consumer]
  let funds_in = 2000
  let stage_in = t.Opened(c, 1000)
  let funds_out = 2000
  let upper_bound = 1234567890
  let expire_at = upper_bound + c.close_period - 1
  let stage_out = t.Closed(c, 1000, expire_at)
  do_close(
    signers,
    IntervalBound(Finite(upper_bound), False),
    funds_in,
    stage_in,
    funds_out,
    stage_out,
  )
}

test close_not_signed() fail {
  let signers = [c.provider]
  let funds_in = 2000
  let stage_in = t.Opened(c, 1000)
  let funds_out = 2000
  let upper_bound = 1234567890
  let expire_at = upper_bound + c.close_period + 1
  let stage_out = t.Closed(c, 1000, expire_at)
  do_close(
    signers,
    IntervalBound(Finite(upper_bound), False),
    funds_in,
    stage_in,
    funds_out,
    stage_out,
  )
}

test settle_not_signed() fail {
  let signers = [c.consumer]
  let funds_in = 1000
  let stage_in = t.Closed(c, iou.amount - 1000, 1000000)
  let funds_out = 0
  let stage_out = t.Settled(c.consumer)
  do_settle(signers, funds_in, stage_in, funds_out, stage_out, iou)
}

test expire_too_soon() fail {
  // An expire in which the time lower bound is not after expire at. 
  let signers = [c.consumer]
  let lower_bound = 1234567890
  let expire_at = lower_bound + 1
  let stage_in = t.Closed(c, 1000, expire_at)
  do_expire(signers, IntervalBound(Finite(lower_bound), False), stage_in)
}

test expire_not_signed() fail {
  let signers = [c.provider]
  let lower_bound = 1234567890
  let expire_at = lower_bound - 1
  let stage_in = t.Closed(c, 1000, expire_at)
  do_expire(signers, IntervalBound(Finite(lower_bound), False), stage_in)
}

test end_not_signed() fail {
  let signers = [c.provider]
  let stage_in = t.Settled(c.consumer)
  do_end(signers, stage_in)
}
