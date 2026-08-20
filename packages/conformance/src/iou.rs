use proptest::prelude::*;
use proptest::strategy::{BoxedStrategy, Strategy};

use cryptoxide::ed25519::{self, PRIVATE_KEY_LENGTH, SIGNATURE_LENGTH, keypair};

use subbit_core::{self, TagTbs};

use crate::AikenFn;

pub type SigningKey = [u8; PRIVATE_KEY_LENGTH];
pub type Signature = [u8; SIGNATURE_LENGTH];

/// Sign `message` with `key` (the expanded secret key).
pub fn sign(key: &SigningKey, message: &[u8]) -> Signature {
    ed25519::signature(message, &keypair(key).0)
}

#[derive(Debug, Clone)]
pub struct SignIou {
    key: SigningKey,
    tag: subbit_core::Tag,
    amount: u64,
}

impl Arbitrary for SignIou {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        (any::<[u8; 32]>(), any::<Vec<u8>>(), any::<u64>())
            .prop_map(|(key, tag_bytes, amount)| {
                let tag = subbit_core::Tag::from(tag_bytes); // swap for your actual constructor
                SignIou { key, tag, amount }
            })
            .boxed()
    }
}

impl Default for SignIou {
    fn default() -> Self {
        Self {
            key: Default::default(),
            tag: subbit_core::Tag::from("subbit is awesome!".as_bytes()),
            amount: 0,
        }
    }
}

impl SignIou {
    fn tbs(&self) -> TagTbs {
        TagTbs::new(self.tag.clone(), self.amount)
    }

    fn sign(&self) -> subbit_core::Signature {
        sign(&self.key, &self.tbs().to_vec()).into()
    }

    fn to_verify(&self) -> Verify {
        let key = keypair(&self.key).1.into();

        Verify {
            key,
            tag: self.tag.clone(),
            amount: self.amount,
            signature: self.sign(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Verify {
    key: subbit_core::VerifyingKey,
    tag: subbit_core::Tag,
    amount: u64,
    signature: subbit_core::Signature,
}

impl<C> minicbor::Encode<C> for Verify {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.tag(minicbor::data::Tag::new(121))?;
        e.begin_array()?;
        e.encode_with(self.key, ctx)?;
        e.encode_with(&self.tag, ctx)?;
        e.encode_with(self.amount, ctx)?;
        e.encode_with(self.signature, ctx)?;
        e.end()?;
        Ok(())
    }
}

impl Verify {
    pub fn corrupt(self) -> Self {
        let Self {
            key,
            tag,
            amount,
            signature,
        } = self;
        Self {
            key,
            tag,
            amount: amount + 1,
            signature,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Key,
    Tag,
    Amount,
    Signature,
}

impl Arbitrary for Field {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;
    fn arbitrary_with(_args: ()) -> Self::Strategy {
        prop_oneof![
            Just(Field::Key),
            Just(Field::Tag),
            Just(Field::Amount),
            Just(Field::Signature),
        ]
        .boxed()
    }
}

impl Verify {
    /// Replace exactly one field with an independently-generated value.
    fn corrupt_field(
        mut self,
        field: Field,
        alt_seed: [u8; 32],
        alt_tag_bytes: Vec<u8>,
        alt_sig_bytes: [u8; SIGNATURE_LENGTH],
    ) -> Self {
        match field {
            Field::Amount => self.amount = self.amount.wrapping_add(1),
            Field::Key => self.key = ed25519::keypair(&alt_seed).1.into(),
            Field::Tag => self.tag = subbit_core::Tag::from(alt_tag_bytes),
            Field::Signature => self.signature = alt_sig_bytes.into(),
        }
        self
    }
}

fn iou_verify_fn() -> AikenFn {
    AikenFn::from_shortcut("iou/verify")
}

#[test]
fn iou_def() {
    assert!(iou_verify_fn().eval_true(&SignIou::default().to_verify()));
}

#[test]
fn iou_corrupt() {
    assert!(iou_verify_fn().eval_false(&SignIou::default().to_verify().corrupt()));
}

proptest! {
    #[test]
    fn prop_iou_conforms(sign_iou: SignIou) {
        assert!(iou_verify_fn().eval_true(&sign_iou.to_verify()));
    }
    #[test]
    fn prop_iou_corrupt_fails(
        sign_iou: SignIou,
        field in any::<Field>(),
        alt_seed: [u8; 32],
        alt_tag_bytes: Vec<u8>,
        alt_sig_bytes: [u8; SIGNATURE_LENGTH],
    ) {
        let corrupted = sign_iou.to_verify()
            .corrupt_field(field, alt_seed, alt_tag_bytes, alt_sig_bytes);
        prop_assert!(iou_verify_fn().eval_false(&corrupted));
    }
}
