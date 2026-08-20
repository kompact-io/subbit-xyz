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
        subbit_header: Option<String>,
    }

    #[async_trait]
    impl ProxyHttp for EchoProxy {
        type CTX = ProxyCtx;
        fn new_ctx(&self) -> Self::CTX {
            ProxyCtx::default()
        }

        async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
            let Some(client) = &self.subbit else {
                return Ok(false);
            };
            match subbit::check(client, session).await {
                subbit::Outcome::Ok(header) => {
                    ctx.subbit_header = header;
                    Ok(false)
                }
                subbit::Outcome::Ko { status, header } => {
                    let mut resp = ResponseHeader::build(status, None)?;
                    if let Some(v) = header {
                        resp.insert_header(subbit::HEADER, v)?;
                    }
                    session.set_keepalive(None);
                    session.write_response_header(Box::new(resp), true).await?;
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
            _session: &mut Session,
            resp: &mut ResponseHeader,
            ctx: &mut Self::CTX,
        ) -> Result<()> {
            if let Some(v) = &ctx.subbit_header {
                resp.insert_header(subbit::HEADER, v.clone())?;
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
