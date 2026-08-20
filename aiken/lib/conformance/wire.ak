use aiken/collection/list
use aiken/primitive/bytearray
use subbit/types as t

fn x(y: ByteArray) {
  expect bytearray.length(y) >= 0
}

pub fn constants(constants: t.Constants) -> Bool {
  let (tag, currency_, iou_key, consumer, provider, close_period) = constants
  trace @"constants:tag": tag
  x(tag)
  expect currency(currency_)
  trace @"constants:iou_key": iou_key
  x(iou_key)
  trace @"constants:consumer": consumer
  x(consumer)
  trace @"constants:provider": provider
  x(provider)
  trace @"constants:close_period": close_period
  close_period >= 0
}

pub fn currency(currency: t.Currency) -> Bool {
  when currency is {
    t.Ada -> {
      trace @"currency:Ada"
      True
    }
    t.Asset { hash, name } -> {
      trace @"currency:Asset:hash": hash
      x(hash)
      trace @"currency:Asset:name": name
      x(name)
      True
    }
  }
}

pub fn datum(datum: t.Datum) -> Bool {
  let (own_hash, constants_, stage_) = datum
  trace @"datum:own_hash": own_hash
  x(own_hash)
  expect constants(constants_)
  expect stage(stage_)
  True
}

pub fn redeemer(redeemer: t.Redeemer) -> Bool {
  when redeemer is {
    t.Defer -> {
      trace @"redeemer:Defer"
      True
    }
    t.Main(steps) -> {
      trace @"redeemer:Main"
      expect list.all(steps, step)
      True
    }
    t.Mutual -> {
      trace @"redeemer:Mutual"
      True
    }
  }
}

// https://github.com/aiken-lang/aiken/issues/1333
pub fn stage(stage: t.Stage) -> Bool {
  when stage is {
    t.Opened { subbed } -> {
      trace @"stage:Opened:subbed": subbed
      expect subbed >= 0
      True
    }
    t.Closed { subbed, elapse_at } -> {
      trace @"stage:Closed:subbed": subbed
      expect subbed >= 0
      trace @"stage:Closed:elapse_at": elapse_at
      expect elapse_at >= 0
      True
    }
    t.Settled -> {
      trace @"stage:Settled"
      True
    }
  }
}

pub fn step(step: t.Step) -> Bool {
  when step is {
    t.StepCont(cont) ->
      when cont is {
        t.Add -> {
          trace @"step:Cont:Add"
          True
        }
        t.Sub { n, sig } -> {
          trace @"step:Cont:Sub:n": n
          expect n >= 0
          trace @"step:Cont:Sub:sig": sig
          x(sig)
          True
        }
        t.Close -> {
          trace @"step:Cont:Close"
          True
        }
        t.Settle { n, sig } -> {
          trace @"step:Cont:Settle:n": n
          expect n >= 0
          trace @"step:Cont:Settle:sig": sig
          x(sig)
          True
        }
      }
    t.StepEol(eol) ->
      when eol is {
        t.End -> {
          trace @"step:Eol:End"
          True
        }
        t.Elapse -> {
          trace @"step:Eol:Expire"
          True
        }
      }
  }
}
