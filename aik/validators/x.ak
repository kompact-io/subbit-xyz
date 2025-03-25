use subbit/types as t

/// For dev purposes only.
/// These are just to have them exposed at top level
/// of plutus.json, and thus picked up by blueprint tooling. 
validator currency {
  mint(_0: t.Currency, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator constants {
  mint(_0: t.Constants, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator stage {
  mint(_0: t.Stage, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator steps {
  mint(_0: t.Steps, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator step {
  mint(_0: t.Step, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator cont {
  mint(_0: t.Cont, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator eol {
  mint(_0: t.Eol, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator iou {
  mint(_0: t.Iou, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}

validator iou_message {
  mint(_0: t.IouMessage, _1: Data, _2: data) {
    False
  }

  else(_) {
    fail
  }
}
