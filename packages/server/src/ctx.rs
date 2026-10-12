use std::{collections::BTreeMap, sync::Arc};

use http::HeaderMap;
use subbit_core::Iou;
use subbit_l2_dto::{
    ChannelStatus, Commit, CoreError, KeyTagTtl, MacTtl, PopMacTtl, Request, Terms,
};

use crate::{
    Backing, Channel, Config, Db, Keytag,
    catalog::{self, ResourceInfo},
    channel,
    channel_ops::{apply_backing, apply_iou, apply_refund, apply_spend},
    crypto::mac,
    db,
    dialect::{Dialect, Quote},
    mutual::{self, MutualError, MutualService},
    now,
    wire::Wire,
};

impl From<channel::Error> for CoreError {
    fn from(value: channel::Error) -> Self {
        match value {
            channel::Error::Other => Self::Other,
            channel::Error::Inactive => Self::Inactive,
            channel::Error::OldIou => Self::OldIou,
            channel::Error::InvalidIou => Self::InvalidIou,
            channel::Error::Unspendable => Self::Unspendable,
        }
    }
}

impl From<db::Error> for CoreError {
    fn from(value: db::Error) -> Self {
        match value {
            db::Error::Backend(_) => Self::Other,
            db::Error::NotFound => Self::NoChannel,
            db::Error::Channel(error) => error.into(),
        }
    }
}

impl From<MutualError> for CoreError {
    fn from(value: MutualError) -> Self {
        match value {
            MutualError::NotSupported => Self::MutualNotSupported,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FromConfigError {
    #[error(transparent)]
    Db(#[from] db::Error),
    #[error(transparent)]
    Catalog(#[from] catalog::Error),
}

pub struct Ctx {
    /// Db connector
    db: Arc<Db>,
    /// Mac secret
    mac: Arc<mac::Mac>,
    /// Cost + resource info, by path
    catalog: Arc<catalog::Catalog>,
    /// Provider Terms
    terms: Arc<Terms>,
    /// Mutual service: validate, complete, and submit mutual txs
    mutual: Arc<dyn MutualService>,
    /// Configured wire dialects, priority order: the first one that
    /// recognizes a request's headers handles it, and the first one
    /// overall speaks for the fallback "payment required" when none
    /// do. See `Self::handle_spend`/`Self::handle_refund`.
    dialects: Vec<Arc<dyn Dialect>>,
    /// This provider's own public URL, if configured. See
    /// `Self::resource_url`.
    public_base_url: Option<String>,
}

impl Ctx {
    pub fn from_config(config: Config) -> Result<Self, FromConfigError> {
        let mac = Arc::new(mac::Mac::new(config.mac_key));
        let db = Arc::new(Db::open(config.db)?);
        let catalog = Arc::new(catalog::Catalog::new(config.terms.catalog.clone())?);
        let terms = Arc::new(config.terms);
        // No `MutualService` implementation exists yet, see
        // `mutual::Unsupported`'s own doc comment.
        let mutual: Arc<dyn MutualService> = Arc::new(mutual::Unsupported);
        let dialects = Self::configured_dialects(config.dialects);

        // config.limiter isn't consumed yet, see the `// Todo :: Handle
        // buckets!` marker below. Wire it in here once that lands.

        // for k in db.keys().unwrap().iter() {
        //     tracing::info!("keys : {}", k);
        // }

        Ok(Self::new(
            db,
            mac,
            catalog,
            terms,
            mutual,
            dialects,
            config.public_base_url,
        ))
    }

    /// x402 first (it wins when both headers are present, and it's
    /// the one with a real discovery/challenge shape to fall back to),
    /// then vanilla; either or both may be disabled at build time (the
    /// Cargo feature) or run time (`config.dialects`).
    fn configured_dialects(dialects: crate::config::Dialects) -> Vec<Arc<dyn Dialect>> {
        let mut out: Vec<Arc<dyn Dialect>> = Vec::new();
        #[cfg(feature = "x402")]
        if dialects.x402 {
            out.push(Arc::new(crate::dialect::x402::X402));
        }
        #[cfg(feature = "vanilla")]
        if dialects.vanilla {
            out.push(Arc::new(crate::dialect::vanilla::Vanilla));
        }
        out
    }

    pub fn new(
        db: Arc<Db>,
        mac: Arc<mac::Mac>,
        catalog: Arc<catalog::Catalog>,
        terms: Arc<Terms>,
        mutual: Arc<dyn MutualService>,
        dialects: Vec<Arc<dyn Dialect>>,
        public_base_url: Option<String>,
    ) -> Self {
        Self {
            db,
            mac,
            catalog,
            terms,
            mutual,
            dialects,
            public_base_url,
        }
    }

    /// The HTTP boundary's one entry point for a spend: select a
    /// dialect from `headers` (falling back to the first configured
    /// one's "payment required" when none recognize it), decode its
    /// payload, resolve `url`'s cost off the catalog, and apply it.
    /// Everything past header-selection is dialect-agnostic business
    /// logic; `main.rs` does nothing but call this and translate the
    /// `Wire` it returns into a real HTTP response.
    pub fn handle_spend(&self, headers: &HeaderMap, url: &str) -> Wire {
        let url = catalog::normalize(url);
        tracing::info!(method = "spend", url = %url, "request in");
        let wire = self.spend_wire(headers, &url);
        self.log_response("spend", &url, &wire);
        wire
    }

    fn spend_wire(&self, headers: &HeaderMap, url: &str) -> Wire {
        let Some(dialect) = self.dialect_for(headers) else {
            return self.fallback_challenge(url);
        };
        let quote = self.quote(url);
        let payload = match dialect.decode(headers, quote.clone()) {
            Ok(payload) => payload,
            Err(wire) => return wire,
        };
        match self.handle_payload(&payload, quote.cost) {
            Ok((status, mac)) => {
                let keytag = Self::keytag_of(&payload);
                dialect.spend_ok(quote, &keytag, status, mac)
            }
            Err(err) => self.dialect_error(dialect, err, quote, &payload),
        }
    }

    /// The HTTP boundary's one entry point for a refund: same dialect
    /// selection as `Self::handle_spend`, then resolve how much of the
    /// quote `url` already charged is actually owed back (off the
    /// catalog again, since a spend's exact cost is never recorded, see
    /// `Self::refund_amount`'s own doc comment) and give it back.
    /// Takes the same `headers` `Self::handle_spend` does, not a fixed
    /// set of pre-extracted fields: whatever a cost model needs to
    /// settle up (today, `ByEgress`'s `Content-Length` or `ByClaim`'s
    /// own claim header) is read straight off them, by `Catalog`
    /// itself (`Catalog::actual_cost`), so a future cost model needing
    /// yet another header never has to change this signature.
    pub fn handle_refund(&self, headers: &HeaderMap, url: &str) -> Wire {
        let url = catalog::normalize(url);
        tracing::info!(method = "refund", url = %url, "request in");
        let wire = self.refund_wire(headers, &url);
        self.log_response("refund", &url, &wire);
        wire
    }

    fn refund_wire(&self, headers: &HeaderMap, url: &str) -> Wire {
        let Some(dialect) = self.dialect_for(headers) else {
            return self.fallback_challenge(url);
        };
        let quote = self.quote(url);
        let payload = match dialect.decode(headers, quote.clone()) {
            Ok(payload) => payload,
            Err(wire) => return wire,
        };
        let amount = self.refund_amount(url, headers);
        let keytag = Self::keytag_of(&payload);
        match self.refund(&keytag, amount) {
            Ok(status) => dialect.refund_ok(quote, &keytag, status),
            Err(err) => self.dialect_error(dialect, err, quote, &payload),
        }
    }

    fn dialect_for(&self, headers: &HeaderMap) -> Option<&Arc<dyn Dialect>> {
        self.dialects.iter().find(|d| d.recognizes(headers))
    }

    fn fallback_challenge(&self, url: &str) -> Wire {
        match self.dialects.first() {
            Some(dialect) => dialect.challenge(self.quote(url)),
            None => Wire::no_supported_dialect(),
        }
    }

    /// `current` (the channel's standing right now) is only ever
    /// looked up for `Unspendable`; anything else has no corrective
    /// shape to size, in any dialect.
    fn dialect_error(
        &self,
        dialect: &Arc<dyn Dialect>,
        err: CoreError,
        quote: Quote<'_>,
        payload: &Request,
    ) -> Wire {
        let current = matches!(err, CoreError::Unspendable)
            .then(|| self.status(&Self::keytag_of(payload)).ok())
            .flatten();
        dialect.error(err, quote, current)
    }

    /// `url`'s catalog/terms data, bundled once for a `Dialect` to
    /// read from rather than passed as several loose parameters (see
    /// `Quote`'s own doc comment).
    fn quote<'a>(&'a self, url: &'a str) -> Quote<'a> {
        Quote {
            resource_url: self.resource_url(url),
            cost: self.cost(url),
            terms: &self.terms,
            resource: self.resource_info(url),
        }
    }

    /// `url` the way x402's own `resource` field wants it: prefixed
    /// with this provider's `public_base_url` into a fully qualified
    /// URL when one is configured, the bare local path otherwise (not
    /// spec-compliant, but still the only identifier available).
    fn resource_url(&self, url: &str) -> String {
        match &self.public_base_url {
            Some(base) => format!("{}{url}", base.trim_end_matches('/')),
            None => url.to_string(),
        }
    }

    fn log_response(&self, method: &str, url: &str, wire: &Wire) {
        if wire.status.is_success() {
            tracing::info!(method, url, status = wire.status.as_u16(), "response given");
        } else {
            tracing::warn!(method, url, status = wire.status.as_u16(), "response given");
        }
    }

    /// The per-resource info a dialect needs beyond cost itself
    /// (x402's `description`/`mimeType`/`maxTimeoutSeconds`).
    pub(crate) fn resource_info(&self, url: &str) -> ResourceInfo {
        self.catalog.info(url)
    }

    /// Cooperatively close the channel `keytag` names, using the
    /// consumer-built `transaction` it sent. See `MutualService`.
    pub(crate) fn close_mutual(
        &self,
        keytag: &Keytag,
        transaction: &[u8],
    ) -> Result<(), MutualError> {
        self.mutual.close(keytag, transaction)
    }

    pub(crate) fn pop_body(&self, pop: &subbit_l2_dto::PopTtl) -> Result<KeyTagTtl, CoreError> {
        if !subbit_core::crypto::verify(&pop.body.key, pop.body.tbs().to_vec().as_ref(), &pop.sig) {
            return Err(CoreError::InvalidAuth);
        }
        Ok(pop.body.clone())
    }

    pub(crate) fn mac_body(&self, mac: &MacTtl) -> Result<KeyTagTtl, CoreError> {
        if !self
            .mac
            .verify(mac.body.tbs().to_vec().as_ref(), &mac.sig.to_bytes().into())
        {
            return Err(CoreError::InvalidAuth);
        }
        Ok(mac.body.clone())
    }

    /// Single entry point for authenticating a request body: verifies the
    /// signature (Pop or Mac) and checks the TTL. Returns the authenticated
    /// body. Shared by both dialects: vanilla's `Auth` and x402's `PopMac`
    /// payload are the exact same type (`subbit_l2_dto::PopMacTtl`), used
    /// here directly rather than through either dialect, since this is
    /// `Ctx`'s core, not an adapter.
    pub(crate) fn auth_body(&self, auth: &PopMacTtl) -> Result<KeyTagTtl, CoreError> {
        let body = match auth {
            PopMacTtl::Pop(pop) => self.pop_body(pop),
            PopMacTtl::Mac(mac) => self.mac_body(mac),
        }?;
        if now() > body.ttl {
            return Err(CoreError::InvalidAuth);
        }
        Ok(body)
    }

    /// Sign a body into a server-issued Mac, so a Pop-authenticated caller
    /// can be handed a Mac for cheaper follow-up requests.
    pub(crate) fn issue_mac(&self, body: &KeyTagTtl) -> MacTtl {
        let sig = (*self.mac.sign(body.tbs().as_ref())).into();
        MacTtl {
            body: body.clone(),
            sig,
        }
    }

    /// What a request against `url` costs.
    pub(crate) fn cost(&self, url: &str) -> u64 {
        self.catalog.cost(url)
    }

    /// How much to give back on a refund against `url`. `Catalog`
    /// reads whatever header its configured model needs straight off
    /// `headers` (see `Catalog::actual_cost`); nothing relevant
    /// present at all means nothing was served, refunding the full
    /// quote, the same way `Flat`/`ByRoute` routes always have
    /// (there's nothing partial to compute for them). Never refunds
    /// more than the quote.
    pub(crate) fn refund_amount(&self, url: &str, headers: &HeaderMap) -> u64 {
        let quoted = self.catalog.cost(url);
        match self.catalog.actual_cost(url, headers) {
            None => quoted,
            Some(actual) => quoted.saturating_sub(actual.min(quoted)),
        }
    }

    fn get(&self, keytag: &Keytag) -> Result<Channel, CoreError> {
        self.db
            .get(keytag)
            .map_err(CoreError::from)?
            .ok_or(CoreError::NoChannel)
    }

    pub fn keytags(&self) -> Result<Vec<Keytag>, db::Error> {
        self.db.keys()
    }

    pub fn channels(&self) -> Result<BTreeMap<Keytag, Channel>, CoreError> {
        let keys = self.db.keys().map_err(CoreError::from)?;
        let mut x = BTreeMap::new();
        for k in keys.into_iter() {
            let v = self.get(&k)?;
            x.insert(k.clone(), v);
        }
        Ok(x)
    }

    pub fn ious(&self) -> Result<BTreeMap<Keytag, Option<Iou>>, CoreError> {
        Ok(self
            .channels()?
            .into_iter()
            .map(|(k, v)| (k, v.iou().cloned()))
            .collect())
    }

    // TODO :: insert buckets
    fn default_channel(&self) -> Channel {
        Channel::new(None, None, 0, Default::default(), Default::default())
    }

    pub fn apply_backings(
        &self,
        backings: BTreeMap<Keytag, Option<Backing>>,
    ) -> Result<(), db::Error> {
        self.db.upsert_batch(
            self.default_channel(),
            backings.into_iter().map(|(k, v)| (k, apply_backing(v))),
        )?;
        Ok(())
    }

    fn channel_status(channel: &Channel) -> Result<ChannelStatus, CoreError> {
        Ok(ChannelStatus {
            iou: channel.iou().ok_or(CoreError::NoIou)?.clone(),
            spendable: channel.spendable(),
            uncommitted: channel.uncommitted(),
        })
    }

    /// Read a channel's current status without changing it. Used to
    /// build a corrective 402 after a `spend` was rejected: the client
    /// needs to see where it actually stands, not the rejected attempt.
    pub(crate) fn status(&self, keytag: &Keytag) -> Result<ChannelStatus, CoreError> {
        Self::channel_status(&self.get(keytag)?)
    }

    /// Verify-and-apply `iou` if present, then apply `cost` against the
    /// channel's spendable balance. Takes a bare `Keytag`: callers that
    /// need to prove liveness first (an `Auth`) do so before calling
    /// this; an `Iou`'s own signature is already its authorization.
    pub(crate) fn spend(
        &self,
        keytag: &Keytag,
        iou: Option<Iou>,
        cost: u64,
    ) -> Result<ChannelStatus, CoreError> {
        if let Some(iou) = iou {
            let (key, tag) = keytag.clone().split();
            if !subbit_core::crypto::verify_iou(&key, &tag, &iou) {
                return Err(CoreError::InvalidIou);
            }
            self.db
                .update_exposed(keytag, apply_iou(iou))
                .map_err(CoreError::from)?;
        }

        let channel = self
            .db
            .update_exposed(keytag, apply_spend(cost))
            .map_err(CoreError::from)?;
        Self::channel_status(&channel)
    }

    /// Undo a previous `spend` of up to `cost` off the running spent
    /// total. Internal accounting only, not a payer-facing refund: used
    /// when the metered request failed, or cost less than what was
    /// pre-spent for it.
    pub(crate) fn refund(&self, keytag: &Keytag, cost: u64) -> Result<ChannelStatus, CoreError> {
        if cost != 0 {
            self.db
                .update(keytag, apply_refund(cost))
                .map_err(CoreError::from)?;
        }
        Self::channel_status(&self.get(keytag)?)
    }

    /// Decode-and-apply one `Request`: the single dispatch both dialects'
    /// wire adapters feed into directly (vanilla re-exports the crate
    /// root's `Request` as-is; x402's own `Request` wraps it as its
    /// `payload` field). `Request` itself carries nothing dialect-
    /// specific. Returns the resulting status, plus a fresh session
    /// `Mac` when (and only when) a Pop-authenticated `ByPopMac`
    /// payload was accepted.
    pub(crate) fn handle_payload(
        &self,
        payload: &Request,
        cost: u64,
    ) -> Result<(ChannelStatus, Option<MacTtl>), CoreError> {
        let keytag = Self::keytag_of(payload);
        let (commit, mac) = match payload {
            Request::ById { commit, .. } => (Some(commit), None),
            Request::ByPopMac { auth, commit } => {
                let body = self.auth_body(auth)?;
                let mac = matches!(auth, PopMacTtl::Pop(_)).then(|| self.issue_mac(&body));
                (commit.as_ref(), mac)
            }
        };
        match commit {
            Some(Commit::Mutual(mutual)) => {
                self.close_mutual(&keytag, &mutual.transaction)?;
                unreachable!("no MutualService implementation exists yet to return Ok from")
            }
            Some(Commit::Iou(iou)) => {
                let status = self.spend(&keytag, Some(iou.clone()), cost)?;
                Ok((status, mac))
            }
            None => {
                let status = self.spend(&keytag, None, cost)?;
                Ok((status, mac))
            }
        }
    }

    /// The channel a `Request` names, read straight off it: no
    /// verification, just structure. Every `Request` names exactly one
    /// channel, directly (`ById`) or via `auth`'s own key/tag
    /// (`ByPopMac`). Used to build a corrective 402 after a rejected
    /// `spend` (the client needs *current* status, not the rejected
    /// attempt's) and, for a refund, as the only identity check that
    /// request gets: a refund never independently verifies its
    /// caller.
    pub(crate) fn keytag_of(payload: &Request) -> Keytag {
        match payload {
            Request::ById { channel_id, .. } => {
                Keytag::new(channel_id.iou_key, channel_id.tag.clone())
            }
            Request::ByPopMac { auth, .. } => {
                let body = match auth {
                    PopMacTtl::Pop(pop) => &pop.body,
                    PopMacTtl::Mac(mac) => &mac.body,
                };
                Keytag::new(body.key, body.tag.clone())
            }
        }
    }

    /// Test-only: create `keytag` with `committed` already on it (an
    /// `Iou` of that amount, no real signature needed, this bypasses
    /// `spend`'s verification, it's not exercising it). Dialect adapter
    /// tests use this to get a spendable channel without needing a real
    /// signed `Iou` just to set the fixture up.
    #[cfg(test)]
    pub(crate) fn seed_channel_for_test(&self, keytag: &Keytag, committed: u64) {
        let iou = Iou::new(committed, subbit_core::Signature::from([0u8; 64]));
        self.db
            .upsert(
                self.default_channel(),
                keytag,
                move |mut channel: Channel| {
                    channel.apply_iou(iou)?;
                    Ok(channel)
                },
            )
            .unwrap();
    }
}
