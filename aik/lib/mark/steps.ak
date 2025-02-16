use aiken/collection/list
use aiken/primitive/bytearray
use mark/gen.{MultiStep, TestStep}
use subbit/logic.{batch, main}

const own_hash =
  #"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    |> bytearray.take(28)

test test_add(args via gen.add(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

test test_sub(args via gen.sub(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

test test_close(args via gen.close(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

test test_settle(args via gen.settle(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

test test_end(args via gen.end(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

test test_expire(args via gen.expire(own_hash)) {
  let TestStep { datum, step, own_oref, tx } = args
  main(datum, [step], own_oref, tx)
}

bench test_multi_subs(args via gen.multi_subs(own_hash, _)) {
  let MultiStep { triples, tx } = args
  when triples is {
    [triple, ..triples] -> {
      let steps =
        triples |> list.map(fn(t) { t.step }) |> list.push(triple.step)
      expect main(triple.datum, steps, triple.own_oref, tx)
      triples
        |> list.all(fn(triple) { batch(own_hash, triple.own_oref, tx.inputs) })
    }
    [] -> True
  }
}
