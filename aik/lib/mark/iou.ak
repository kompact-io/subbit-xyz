use subbit/logic.{verify_iou}
use subbit/types as t

pub type IouTest {
  vkey: ByteArray,
  subbit_id: ByteArray,
  amount: Int,
  msg: ByteArray,
  sig: ByteArray,
}

pub const i0 =
  IouTest {
    vkey: #"3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
    subbit_id: #"6c3b9aa767f785b5",
    amount: 8594738769458413623,
    msg: #"d8799f486c3b9aa767f785b51b7746a55fbad8c037ff",
    sig: #"8bb7cb5fc82b23fac9114e9a4913f3b01c4db608e6189f01eeb431c7cd0c8c6e28484cdcc25aa895a715622598110754ede40f390797179a70e373856450ce03",
  }

test test_i0() {
  let i = i0
  verify_iou(i.vkey, i.subbit_id, t.Iou(i.amount, i.sig))
}

pub const i1 =
  IouTest {
    vkey: #"cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
    subbit_id: #"b10da48cea4c0967",
    amount: 1482817706323250795,
    msg: #"d8799f48b10da48cea4c09671b149406d8fc0e8e6bff",
    sig: #"7f96d963c32e225ae0a1fe1491466f277cd4cde77cd4f8335578fd888b61942af64972e02efc3a603241f2d84bb5df95b18f57e2f88e4d63c249716b068d8d01",
  }

test test_i1() {
  let i = i1
  verify_iou(i.vkey, i.subbit_id, t.Iou(i.amount, i.sig))
}

pub const i2 =
  IouTest {
    vkey: #"6b79c57e6a095239282c04818e96112f3f03a4001ba97a564c23852a3f1ea5fc",
    subbit_id: #"c51b8a31c98b9fe1",
    amount: 10116765682372994352,
    msg: #"d8799f48c51b8a31c98b9fe11b8c65f8c985b46530ff",
    sig: #"7e6161aab7aff7d73afe83b176d0f3549dc81a5c69ff35aaa7576417e69d7eb81b070d8aa4ae115a6beff99851be1b6266b66d4b2a3d56b4ae499bbc66d8f306",
  }

test test_i2() {
  let i = i2
  verify_iou(i.vkey, i.subbit_id, t.Iou(i.amount, i.sig))
}

pub const i3 =
  IouTest {
    vkey: #"dadbd184a2d526f1ebdd5c06fdad9359b228759b4d7f79d66689fa254aad8546",
    subbit_id: #"5de9bc1bb4cb7a9f",
    amount: 1499751601245649308,
    msg: #"d8799f485de9bc1bb4cb7a9f1b14d03022a391f99cff",
    sig: #"c6ca9a4e75691b030d416bb8316d4cf43601b8764d88f8cdbd36207136334abbd819cd00ac77962f7a8dbc494d253f147b5c9d78bf3a77fb6435d0f318d32d09",
  }

test test_i3() {
  let i = i3
  verify_iou(i.vkey, i.subbit_id, t.Iou(i.amount, i.sig))
}

pub const i4 =
  IouTest {
    vkey: #"9be3287795907809407e14439ff198d5bfc7dce6f9bc743cb369146f610b4801",
    subbit_id: #"22cfaefc92e4edb9",
    amount: 1287912097577021104,
    msg: #"d8799f4822cfaefc92e4edb91b11df953aa601aeb0ff",
    sig: #"8775af7f57f681bddb8ad3682f662dcfd0fbb09b0a0fe652d880fc3d96f60f66225f4d5bc31e7a1acb729ffebec5ed0666b93e4acbc26392103a1e7878638f0e",
  }

test test_i4() {
  let i = i4
  verify_iou(i.vkey, i.subbit_id, t.Iou(i.amount, i.sig))
}
