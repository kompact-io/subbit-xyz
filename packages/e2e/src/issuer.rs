//! Builds a real `reqwest_middleware::ClientWithMiddleware` wrapping
//! `subbit_issuer::Issuer`, the same `Arc<Issuer<C>>` + forwarding-
//! `Middleware` pattern `examples/echo/client` uses, so `.cache()`/
//! `.into_issuer()` stay reachable after the client moves into
//! `ClientBuilder`.

use std::sync::Arc;

use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{
    ClientBuilder, ClientWithMiddleware, Middleware, Next, Result as MwResult,
};
use subbit_core::{Duration, Tag};
use subbit_issuer::reqwest::Cost;
use subbit_issuer::reqwest::vanilla::Issuer as VanillaMiddleware;
use subbit_issuer::reqwest::x402::Issuer as X402Middleware;
use subbit_issuer::{Account, Issuer};

/// Derives the same keypair a `crate::Wallet` built from the same
/// `seed`/`tag` would, so the two name the same channel: `Wallet` to
/// push a fake-indexer backing, the `Issuer` built from this to drive
/// real requests against it.
pub fn account(seed: [u8; 32], tag: impl Into<Vec<u8>>) -> Account {
    Account::new(
        ed25519_dalek::SigningKey::from_bytes(&seed),
        Tag::from(tag.into()),
    )
}

pub fn issuer(seed: [u8; 32], tag: impl Into<Vec<u8>>, ttl: Duration) -> Issuer {
    Issuer::new(account(seed, tag), ttl)
}

struct ForwardVanilla<C>(Arc<VanillaMiddleware<C>>);

#[async_trait::async_trait]
impl<C: Cost + 'static> Middleware for ForwardVanilla<C> {
    async fn handle(
        &self,
        req: Request,
        ext: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, ext, next).await
    }
}

/// A real vanilla-dialect client: `issuer` drives `subbit-server`'s
/// `/v1/{spend,refund}/{path}` routes over real HTTP, costed by
/// `costing`. Returns the middleware alongside the client so its
/// `.cache()`/`.into_issuer()` stay reachable once the request's done.
pub fn vanilla_client<C>(
    issuer: Issuer,
    costing: C,
) -> (Arc<VanillaMiddleware<C>>, ClientWithMiddleware)
where
    C: Cost + 'static,
{
    let middleware = Arc::new(VanillaMiddleware::new(issuer, costing));
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(ForwardVanilla(middleware.clone()))
        .build();
    (middleware, client)
}

struct ForwardX402<C>(Arc<X402Middleware<C>>);

#[async_trait::async_trait]
impl<C: Cost + 'static> Middleware for ForwardX402<C> {
    async fn handle(
        &self,
        req: Request,
        ext: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, ext, next).await
    }
}

/// As [`vanilla_client`], but x402: the first request against any
/// given path goes out bare (nothing's been paid under yet) and comes
/// back `402`, teaching the middleware what to pay next time. Callers
/// must retry accordingly, see `subbit_issuer::reqwest::x402::Issuer`'s
/// own doc comment.
pub fn x402_client<C>(issuer: Issuer, costing: C) -> (Arc<X402Middleware<C>>, ClientWithMiddleware)
where
    C: Cost + 'static,
{
    let middleware = Arc::new(X402Middleware::new(issuer, costing));
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(ForwardX402(middleware.clone()))
        .build();
    (middleware, client)
}
