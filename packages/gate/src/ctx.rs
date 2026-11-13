use std::{collections::BTreeMap, sync::Arc};

use http::HeaderMap;
use subbit_core::Iou;
use subbit_l2_dto::{ChannelStatus, Commit, Error, KeyTagTtl, MacTtl, PopMacTtl, Request, Terms};

use crate::{
    Backing, Channel, ChannelSummary, Config, Db, Keytag,
    catalog::{self, ResourceInfo},
    channel,
    channel_ops::{apply_backing, apply_index_decision, apply_iou, apply_refund, apply_spend},
    crypto::mac,
    db,
    dialect::{Dialect, Quote},
    index_output,
    mutual::{self, MutualError, MutualService},
    now,
    wire::Wire,
};

impl From<channel::Error> for Error {
    fn from(value: channel::Error) -> Self {
        match value {
            channel::Error::Other => Self::Other,
            channel::Error::Inactive => Self::Inactive,
            channel::Error::OldIou => Self::OldIou,
            channel::Error::InvalidIou => Self::InvalidIou,
            channel::Error::NoBacking => Self::NoChannel,
            channel::Error::InsufficientFunds => Self::InsufficientFunds,
        }
    }
}

impl From<db::Error> for Error {
    fn from(value: db::Error) -> Self {
        match value {
            db::Error::Backend(_) => Self::Other,
            db::Error::NotFound => Self::NoChannel,
            db::Error::Channel(error) => error.into(),
        }
    }
}

impl From<MutualError> for Error {
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
    /// See `Config::finality_depth`'s own doc.
    finality_depth: u64,
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

        // `config.limiter` isn't consumed yet: no `Limiter` is built
        // here, and `default_channel` below never seeds a channel's
        // `Bucket` from it, so rate limiting is defined but inert.

        Ok(Self::new(
            db,
            mac,
            catalog,
            terms,
            mutual,
            dialects,
            config.public_base_url,
            config.finality_depth,
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

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    // A plain field-for-field constructor (`from_config` is the real
    // entry point; this exists for tests/benches that need to swap in
    // a custom catalog/terms/dialects) -- no behavior to split out of,
    // so a builder would just move the same list of fields elsewhere.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Db>,
        mac: Arc<mac::Mac>,
        catalog: Arc<catalog::Catalog>,
        terms: Arc<Terms>,
        mutual: Arc<dyn MutualService>,
        dialects: Vec<Arc<dyn Dialect>>,
        public_base_url: Option<String>,
        finality_depth: u64,
    ) -> Self {
        Self {
            db,
            mac,
            catalog,
            terms,
            mutual,
            dialects,
            public_base_url,
            finality_depth,
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
        // FIXME :: Use instrument??
        tracing::info!(method = "spend", url = %url, "request in");
        let wire = self.spend_wire(headers, url);
        self.log_response("spend", url, &wire);
        wire
    }

    fn spend_wire(&self, headers: &HeaderMap, url: &str) -> Wire {
        let Some(dialect) = self.dialect_for(headers) else {
            tracing::debug!("spend_wire: no dialect recognized headers");
            return self.fallback_challenge(url);
        };
        let quote = self.quote(url);
        let payload = match dialect.decode(headers, quote.clone()) {
            Ok(payload) => payload,
            Err(wire) => {
                tracing::debug!(status = wire.status.as_u16(), "spend_wire: decode failed");
                return wire;
            }
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
        // FIXME :: Use instrument??
        tracing::info!(method = "refund", url = %url, "request in");
        let wire = self.refund_wire(headers, url);
        self.log_response("refund", url, &wire);
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

    /// `current` (the channel's standing right now) is looked up whenever
    /// `err` still has a real, backed channel behind it to report on --
    /// `NoChannel` (no channel at all) and `Inactive` (channel closed, not
    /// backed any more) don't, so there's nothing to look up; `Other` is
    /// too ambiguous to assume one either. Everything else (`OldIou`,
    /// `InvalidIou`, `OldAuth`, `InvalidAuth`, `NoIou`, `InsufficientFunds`,
    /// `RateLimited`, `MutualNotSupported`) does, and the client needs
    /// exactly this to resync instead of repeating the same now-stale
    /// amount forever (see `subbit-stamp`'s own `apply_status` callers).
    fn dialect_error(
        &self,
        dialect: &Arc<dyn Dialect>,
        err: Error,
        quote: Quote<'_>,
        payload: &Request,
    ) -> Wire {
        let current = (!matches!(err, Error::NoChannel | Error::Inactive | Error::Other))
            .then(|| self.status(&Self::keytag_of(payload)).ok())
            .flatten();
        tracing::debug!(?err, current_amount = ?current.as_ref().map(|s| s.iou.amount()), "dialect error");
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
    /// (x402's `description`/`mimeType`/`maxTimeoutSeconds`). Panics
    /// if `url` matches nothing in the catalog at all: every
    /// provider-configured `Catalog` is expected to register a
    /// catch-all (`"/{*rest}"`) for blanket coverage (see `Catalog`'s
    /// own doc comment) -- an unmatched `url` here means either a
    /// provider config that genuinely forgot one, or a route this
    /// server serves but never published pricing for, both startup-
    /// time configuration mistakes rather than a per-request failure
    /// mode worth a wire-level error of their own.
    pub(crate) fn resource_info(&self, url: &str) -> ResourceInfo {
        Self::require_catalog(url, self.catalog.resolve(url).cloned())
    }

    /// Unwraps a catalog lookup for `url`, panicking with a consistent
    /// message when it misses. See `resource_info`'s own doc comment
    /// on why an unmatched `url` panics rather than returning some
    /// fallback.
    fn require_catalog<T>(url: &str, found: Option<T>) -> T {
        found.unwrap_or_else(|| panic!("no catalog entry for {url}; register a catch-all route"))
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

    pub(crate) fn pop_body(&self, pop: &subbit_l2_dto::PopTtl) -> Result<KeyTagTtl, Error> {
        if !subbit_core::crypto::verify(&pop.body.key, pop.body.tbs().to_vec().as_ref(), &pop.sig) {
            return Err(Error::InvalidAuth);
        }
        Ok(pop.body.clone())
    }

    pub(crate) fn mac_body(&self, mac: &MacTtl) -> Result<KeyTagTtl, Error> {
        if !self
            .mac
            .verify(mac.body.tbs().to_vec().as_ref(), &mac.sig.to_bytes().into())
        {
            return Err(Error::InvalidAuth);
        }
        Ok(mac.body.clone())
    }

    /// Single entry point for authenticating a request body: verifies the
    /// signature (Pop or Mac) and checks the TTL. Returns the authenticated
    /// body. Shared by both dialects: vanilla's `Auth` and x402's `PopMac`
    /// payload are the exact same type (`subbit_l2_dto::PopMacTtl`), used
    /// here directly rather than through either dialect, since this is
    /// `Ctx`'s core, not an adapter.
    pub(crate) fn auth_body(&self, auth: &PopMacTtl) -> Result<KeyTagTtl, Error> {
        let body = match auth {
            PopMacTtl::Pop(pop) => self.pop_body(pop),
            PopMacTtl::Mac(mac) => self.mac_body(mac),
        }?;
        if now() > body.expires_at {
            return Err(Error::InvalidAuth);
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

    /// What a request against `url` costs. See `resource_info`'s own
    /// doc comment on why an unmatched `url` panics rather than
    /// returning some fallback amount.
    pub(crate) fn cost(&self, url: &str) -> u64 {
        Self::require_catalog(url, self.catalog.cost(url))
    }

    /// How much to give back on a refund against `url`.
    /// `catalog::actual_cost` reads whatever header the resolved
    /// model needs straight off `headers`; nothing relevant present
    /// at all means nothing was served, refunding the full quote.
    /// Never refunds more than the quote.
    pub(crate) fn refund_amount(&self, url: &str, headers: &HeaderMap) -> u64 {
        let quoted = self.cost(url);
        match catalog::actual_cost(&self.catalog, url, headers) {
            None => quoted,
            Some(actual) => quoted.saturating_sub(actual.min(quoted)),
        }
    }

    fn get(&self, keytag: &Keytag) -> Result<Channel, Error> {
        self.db
            .get(keytag)
            .map_err(Error::from)?
            .ok_or(Error::NoChannel)
    }

    pub fn keytags(&self) -> Result<Vec<Keytag>, db::Error> {
        self.db.keys()
    }

    pub fn channels(&self) -> Result<BTreeMap<Keytag, Channel>, Error> {
        let keys = self.db.keys().map_err(Error::from)?;
        let mut x = BTreeMap::new();
        for k in keys.into_iter() {
            let v = self.get(&k)?;
            x.insert(k.clone(), v);
        }
        Ok(x)
    }

    pub fn ious(&self) -> Result<BTreeMap<Keytag, Option<Iou>>, Error> {
        Ok(self
            .channels()?
            .into_iter()
            .map(|(k, v)| (k, v.iou().cloned()))
            .collect())
    }

    /// Per-channel `{is_active, iou}`, for an external actor (e.g. a
    /// provider tx service) deciding what to claim on-chain -- it never
    /// writes `is_active` itself, only reads it.
    pub fn channel_summaries(&self) -> Result<BTreeMap<Keytag, ChannelSummary>, Error> {
        Ok(self
            .channels()?
            .into_iter()
            .map(|(k, v)| (k, v.summary()))
            .collect())
    }

    fn default_channel(&self) -> Channel {
        Channel::new(None, None, 0, Default::default(), Default::default(), None)
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

    /// The real entry point for the index service's lineage report:
    /// an `Output` may be provider-agnostic (every channel at the
    /// subbit validator, from any provider), so filtering down to
    /// this provider's own (`index_output::ProviderFilter`, off
    /// `self.terms`) is this server's job, same as the actual
    /// `Backing`/`safe_to_settle` choosing
    /// (`index_output::choose_backings`, this server's own
    /// "Downstream" responsibility -- see that module's doc).
    ///
    /// Reads each matching channel's current cumulative IOU off the db
    /// first -- `choose_backings` needs it to weigh claimable funds. A
    /// channel this `Output` reports may not have a row here yet at
    /// all (or one with no IOU committed yet), which is not an error,
    /// just nothing to add to the map.
    pub fn apply_output(&self, output: subbit_index_dto::Channels) -> Result<(), db::Error> {
        tracing::debug!(
            tip_slot = output.tip.slot,
            tip_height = output.tip.height,
            channels_in_output = output.channels.len(),
            finality_depth = self.finality_depth,
            "apply_output: lineage report received"
        );
        let filter = index_output::ProviderFilter {
            provider: self.terms.provider,
            asset: self.terms.asset.clone(),
            close_period: self.terms.close_period,
        };
        let mut cumulative_owed = BTreeMap::new();
        for channel in &output.channels {
            let keytag = index_output::keytag_of(&channel.constants_ext.constants);
            if let Some(owed) = self.db.get(&keytag)?.and_then(|c| c.committed()) {
                cumulative_owed.insert(keytag, owed);
            }
        }
        let decisions =
            index_output::choose_backings(&output, &filter, self.finality_depth, &cumulative_owed);
        // Diagnostic only (see `plan-canary-l2-oldiou-backoff.md`'s `NoChannel`-flapping
        // investigation): log every backing *transition*, keyed by whether it changed and
        // which direction, so a live run's trace shows exactly when/why a channel drops out.
        for (keytag, decision) in &decisions {
            let previous = self
                .db
                .get(keytag)
                .ok()
                .flatten()
                .and_then(|c| c.backing().cloned());
            if previous.as_ref() != decision.backing.as_ref() {
                tracing::debug!(
                    ?keytag,
                    tip_height = output.tip.height,
                    previous_backing = ?previous,
                    new_backing = ?decision.backing,
                    "apply_output: backing changed"
                );
            }
        }
        self.db.upsert_batch(
            self.default_channel(),
            decisions
                .into_iter()
                .map(|(k, d)| (k, apply_index_decision(d.backing, d.safe_to_settle))),
        )?;
        Ok(())
    }

    fn channel_status(channel: &Channel) -> Result<ChannelStatus, Error> {
        Ok(ChannelStatus {
            iou: channel.iou().ok_or(Error::NoIou)?.clone(),
            spendable: channel.spendable(),
            uncommitted: channel.uncommitted(),
        })
    }

    /// Read a channel's current status without changing it. Used to
    /// build a corrective 402 after a `spend` was rejected: the client
    /// needs to see where it actually stands, not the rejected attempt.
    pub(crate) fn status(&self, keytag: &Keytag) -> Result<ChannelStatus, Error> {
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
    ) -> Result<ChannelStatus, Error> {
        if let Some(iou) = iou {
            let (key, tag) = keytag.clone().split();
            if !subbit_core::crypto::verify_iou(&key, &tag, &iou) {
                return Err(Error::InvalidIou);
            }
            self.db
                .update_exposed(keytag, apply_iou(iou))
                .map_err(Error::from)?;
        }

        let channel = self
            .db
            .update_exposed(keytag, apply_spend(cost))
            .map_err(Error::from)?;
        Self::channel_status(&channel)
    }

    /// Undo a previous `spend` of up to `cost` off the running spent
    /// total. Internal accounting only, not a payer-facing refund: used
    /// when the metered request failed, or cost less than what was
    /// pre-spent for it.
    pub(crate) fn refund(&self, keytag: &Keytag, cost: u64) -> Result<ChannelStatus, Error> {
        if cost != 0 {
            self.db
                .update(keytag, apply_refund(cost))
                .map_err(Error::from)?;
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
    ) -> Result<(ChannelStatus, Option<MacTtl>), Error> {
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
    /// signed `Iou` just to set the fixture up. The channel gets a backing
    /// generous enough that no test's later `Iou` ever trips
    /// `apply_iou`'s own backing check -- that check has its own tests in
    /// `channel.rs`; here it's just fixture.
    #[cfg(test)]
    pub(crate) fn seed_channel_for_test(&self, keytag: &Keytag, committed: u64) {
        const TEST_BACKING: u64 = 1_000_000_000_000;
        let iou = Iou::new(committed, subbit_core::Signature::from([0u8; 64]));
        self.db
            .upsert(
                self.default_channel(),
                keytag,
                move |mut channel: Channel| {
                    channel.apply_backing(Some(Backing::new(TEST_BACKING, 0)));
                    channel.apply_iou(iou)?;
                    Ok(channel)
                },
            )
            .unwrap();
    }
}
