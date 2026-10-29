use async_trait::async_trait;
use pingora::prelude::*;
use std::net::SocketAddr;

#[cfg(not(feature = "subbit"))]
mod without_subbit {
    use super::*;

    pub struct EchoProxy {
        upstream: SocketAddr,
    }

    impl EchoProxy {
        pub fn new(upstream: SocketAddr) -> Self {
            Self { upstream }
        }
    }

    #[async_trait]
    impl ProxyHttp for EchoProxy {
        type CTX = ();
        fn new_ctx(&self) -> Self::CTX {}

        async fn request_filter(
            &self,
            session: &mut Session,
            _ctx: &mut Self::CTX,
        ) -> Result<bool> {
            let path = session.req_header().uri.path().to_string();
            if let Some(result) = crate::static_assets::serve(session, &path).await {
                return result;
            }
            Ok(false)
        }

        async fn upstream_peer(
            &self,
            session: &mut Session,
            _ctx: &mut Self::CTX,
        ) -> Result<Box<HttpPeer>> {
            tracing::info!(path = %session.req_header().uri.path(), "request in");
            Ok(Box::new(HttpPeer::new(self.upstream, false, String::new())))
        }

        async fn logging(
            &self,
            session: &mut Session,
            _e: Option<&pingora::Error>,
            _ctx: &mut Self::CTX,
        ) {
            tracing::info!(
                status = session
                    .response_written()
                    .map(|r| r.status.as_u16())
                    .unwrap_or(0),
                "response given"
            );
        }
    }
}

#[cfg(feature = "subbit")]
mod with_subbit {
    use super::*;
    use crate::subbit;

    pub struct EchoProxy {
        upstream: SocketAddr,
        subbit: Option<subbit::Client>,
    }

    impl EchoProxy {
        pub fn new(upstream: SocketAddr, subbit: Option<subbit::Client>) -> Self {
            Self { upstream, subbit }
        }
    }

    #[derive(Default)]
    pub struct ProxyCtx {
        /// The consumer's own dialect auth header(s) from the spend
        /// call, reused unchanged for this same request's refund.
        auth_headers: http::HeaderMap,
        /// `subbit-gate`'s own reply header(s) to the spend call,
        /// relayed onto the real response.
        relay_headers: http::HeaderMap,
    }

    #[async_trait]
    impl ProxyHttp for EchoProxy {
        type CTX = ProxyCtx;
        fn new_ctx(&self) -> Self::CTX {
            ProxyCtx::default()
        }

        async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
            let path = session.req_header().uri.path().to_string();
            if let Some(result) = crate::static_assets::serve(session, &path).await {
                return result;
            }
            let Some(client) = &self.subbit else {
                return Ok(false);
            };
            if subbit::is_terms_path(session.req_header().uri.path()) {
                let (status, body) = subbit::terms(client).await.unwrap_or((502, Vec::new()));
                let mut resp = ResponseHeader::build(status, None)?;
                resp.insert_header("content-type", "application/json")?;
                session.set_keepalive(None);
                session.write_response_header(Box::new(resp), false).await?;
                session
                    .write_response_body(Some(bytes::Bytes::from(body)), true)
                    .await?;
                return Ok(true);
            }
            let (auth_headers, outcome) = subbit::check(client, session).await;
            match outcome {
                subbit::Outcome::Ok(reply_headers) => {
                    ctx.auth_headers = auth_headers;
                    ctx.relay_headers = subbit::relay_headers(&reply_headers);
                    Ok(false)
                }
                subbit::Outcome::Ko {
                    status,
                    headers,
                    body,
                } => {
                    let mut resp = ResponseHeader::build(status, None)?;
                    for (name, value) in &subbit::relay_headers(&headers) {
                        resp.insert_header(name.clone(), value.clone())?;
                    }
                    session.set_keepalive(None);
                    let send_body = body.map(bytes::Bytes::from);
                    session
                        .write_response_header(Box::new(resp), send_body.is_none())
                        .await?;
                    if let Some(body) = send_body {
                        session.write_response_body(Some(body), true).await?;
                    }
                    Ok(true)
                }
            }
        }

        async fn upstream_peer(
            &self,
            session: &mut Session,
            _ctx: &mut Self::CTX,
        ) -> Result<Box<HttpPeer>> {
            tracing::info!(path = %session.req_header().uri.path(), "request in");
            Ok(Box::new(HttpPeer::new(self.upstream, false, String::new())))
        }

        async fn response_filter(
            &self,
            session: &mut Session,
            resp: &mut ResponseHeader,
            ctx: &mut Self::CTX,
        ) -> Result<()> {
            for (name, value) in &ctx.relay_headers {
                resp.insert_header(name.clone(), value.clone())?;
            }
            if let Some(client) = &self.subbit {
                let path = session.req_header().uri.path().to_string();
                let mut settle_headers = ctx.auth_headers.clone();
                if let Some(len) = resp.headers.get(http::header::CONTENT_LENGTH) {
                    settle_headers.insert(http::header::CONTENT_LENGTH, len.clone());
                }
                if let Some(cost) = resp.headers.get(subbit_l2_dto::CLAIM_COST_HEADER) {
                    settle_headers.insert(
                        subbit_l2_dto::CLAIM_COST_HEADER
                            .parse::<http::HeaderName>()
                            .expect("valid header name"),
                        cost.clone(),
                    );
                }
                // Fire-and-forget: a failed refund never blocks or fails
                // the response already being served to the consumer.
                if let subbit::Outcome::Ko { status, .. } =
                    client.refund(&path, &settle_headers).await
                {
                    tracing::warn!(status, "refund call failed");
                }
            }
            Ok(())
        }

        async fn logging(
            &self,
            session: &mut Session,
            _e: Option<&pingora::Error>,
            _ctx: &mut Self::CTX,
        ) {
            tracing::info!(
                status = session
                    .response_written()
                    .map(|r| r.status.as_u16())
                    .unwrap_or(0),
                "response given"
            );
        }
    }
}

#[cfg(feature = "subbit")]
pub use with_subbit::EchoProxy;
#[cfg(not(feature = "subbit"))]
pub use without_subbit::EchoProxy;
