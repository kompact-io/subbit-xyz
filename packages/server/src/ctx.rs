use std::{collections::BTreeMap, sync::Arc};

use subbit_core::{
    Auth, Iou,
    envelope::{self, Body, Error, Request, Response, Status},
};

use crate::{
    Backing, Channel, Config, Db, Keytag,
    channel_ops::{apply_backing, apply_iou, apply_refund, apply_spend},
    costings,
    crypto::{ed25519, mac},
    db, iou, now,
};

#[derive(Debug, thiserror::Error)]
pub enum CtxError {
    #[error("other")]
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum FromConfigError {
    #[error(transparent)]
    Db(#[from] db::Error),
    #[error(transparent)]
    Costings(#[from] costings::Error),
}

pub struct Ctx {
    db: Arc<Db>,
    mac: Arc<mac::Mac>,
    costings: Arc<costings::Costings>,
}

impl Ctx {
    pub fn from_config(config: Config) -> Result<Self, FromConfigError> {
        let mac = Arc::new(mac::Mac::new(config.mac_key));
        let db = Arc::new(Db::open(config.db)?);
        let costings = Arc::new(costings::Costings::new(config.costings)?);

        // config.limiter isn't consumed yet — see the `// Todo :: Handle
        // buckets!` marker below. Wire it in here once that lands.

        for k in db.keys().unwrap().iter() {
            tracing::info!("keys : {}", k);
        }

        Ok(Self::new(db, mac, costings))
    }

    pub fn new(db: Arc<Db>, mac: Arc<mac::Mac>, costings: Arc<costings::Costings>) -> Self {
        Self { db, mac, costings }
    }

    fn pop_body(&self, pop: &subbit_core::Pop<Body>) -> Result<Body, Error> {
        if !ed25519::verify(&pop.body.key, pop.body.tbs().to_vec().as_ref(), &pop.sig) {
            return Err(Error::InvalidAuth);
        }
        Ok(pop.body.clone())
    }

    fn mac_body(&self, mac: &subbit_core::Mac<Body>) -> Result<Body, Error> {
        if !self
            .mac
            .verify(mac.body.tbs().to_vec().as_ref(), &mac.sig.into())
        {
            return Err(Error::InvalidAuth);
        }
        Ok(mac.body.clone())
    }

    /// Single entry point for authenticating a request body: verifies the
    /// signature (Pop or Mac) and checks the TTL. Returns the authenticated body.
    fn auth_body(&self, auth: &Auth<Body, Body>) -> Result<Body, Error> {
        let body = match auth {
            Auth::Pop(pop) => self.pop_body(pop),
            Auth::Mac(mac) => self.mac_body(mac),
        }?;
        if now() > body.ttl {
            return Err(Error::InvalidAuth);
        }
        Ok(body)
    }

    // Get keytag without checks
    fn get_keytag(&self, auth: &Auth<Body, Body>) -> Keytag {
        let body = match auth {
            Auth::Pop(x) => &x.body,
            Auth::Mac(x) => &x.body,
        };
        Keytag::new(body.key, body.tag.clone())
    }

    /// Sign a body into a server-issued Mac, so a Pop-authenticated caller
    /// can be handed a Mac for cheaper follow-up requests.
    fn issue_mac(&self, body: &Body) -> subbit_core::Mac<Body> {
        let sig = self.mac.sign(body.tbs().as_ref()).into();
        subbit_core::Signed {
            body: body.clone(),
            sig,
        }
    }

    fn get(&self, keytag: &Keytag) -> Result<Channel, Error> {
        self.db
            .get(keytag)
            .map_err(|_| Error::Other)?
            .ok_or(Error::NoChannel)
    }

    pub fn keytags(&self) -> Result<Vec<Keytag>, Error> {
        Ok(self.db.keys()?)
    }

    pub fn channels(&self) -> Result<BTreeMap<Keytag, Channel>, Error> {
        let keys = self.db.keys()?;
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

    pub fn spend(&self, req: Request, url: &str) -> Response {
        let spend = self.costings.lookup(url);
        let envelope::Request { auth, iou } = req;

        // Handle auth
        let body = self.auth_body(&auth)?;
        // Todo :: Handle buckets!
        let mac = matches!(auth, Auth::Pop(_)).then(|| self.issue_mac(&body));
        // Get account
        let keytag = Keytag::new(body.key, body.tag.clone());

        // Handle iou if exist
        if let Some(iou) = iou {
            if !iou::is_valid(&body.key, &body.tag, &iou) {
                return Err(Error::InvalidIou);
            }
            self.db.update_exposed(&keytag, apply_iou(iou))?;
        }

        // Apply spend
        if spend > 0 {
            self.db.update_exposed(&keytag, apply_spend(spend))?;
        }

        // Get l2 state
        let channel = self.get(&keytag)?;
        Ok(Status {
            iou: channel.iou().ok_or(Error::NoIou)?.clone(),
            spendable: channel.spendable(),
            uncommitted: channel.uncommitted(),
            mac,
        })
    }

    pub fn refund(&self, req: Request, url: &str) -> Response {
        let spend = self.costings.lookup(url);
        let envelope::Request { auth, .. } = req;
        let keytag = self.get_keytag(&auth);
        if spend != 0 {
            self.db.update(&keytag, apply_refund(spend))?;
        }
        let channel = self.get(&keytag)?;
        Ok(Status {
            iou: channel.iou().ok_or(Error::NoIou)?.clone(),
            spendable: channel.spendable(),
            uncommitted: channel.uncommitted(),
            mac: None,
        })
    }
}
