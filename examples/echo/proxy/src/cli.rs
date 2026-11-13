use std::path::Path;

use clap::{Parser, Subcommand};
use pingora::prelude::*;

use crate::config::Config;
use crate::proxy::EchoProxy;

#[derive(Parser)]
pub struct Cli {
    #[command(flatten)]
    sources: subbit_config::Args,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter config to file.
    Init,
    /// Run the proxy.
    Run,
}

impl Cli {
    pub fn run(self) {
        let sources = self
            .sources
            .into_sources(Path::new("echo-proxy-config.toml"));

        if let Cmd::Init = self.cmd {
            Config::write_default(sources.base).expect("write config");
            println!("wrote starter config to {}", sources.base.display());
            return;
        }

        let config: Config = sources.load().expect("failed to load config");

        let server_conf = pingora::server::configuration::ServerConf {
            // Pingora's own default is 300s -- nothing here holds a
            // long-lived connection worth draining that long for.
            grace_period_seconds: Some(1),
            graceful_shutdown_timeout_seconds: Some(1),
            ..Default::default()
        };
        let mut server = Server::new_with_opt_and_conf(None, server_conf);
        server.bootstrap();

        #[cfg(feature = "subbit")]
        let proxy_impl = EchoProxy::new(config.upstream, config.subbit.map(|s| s.into_client()));
        #[cfg(not(feature = "subbit"))]
        let proxy_impl = EchoProxy::new(config.upstream);

        let mut proxy = http_proxy_service(&server.configuration, proxy_impl);
        proxy.add_tcp(&config.listen.to_string());
        server.add_service(proxy);

        tracing::info!(listen = %config.listen, upstream = %config.upstream, "Starting");
        server.run_forever();
    }
}
