use anyhow::Result;
use clap::Subcommand;

use cardano_sdk::{Credential, Hash};

use crate::ctx::Ctx;
use crate::session::{build, print_status};

#[derive(Subcommand)]
pub enum SessionCmd {
    /// Rebuild the session against current chain state and print a summary.
    Status,
    /// Upload the subbit validator's reference script to the wallet.
    Upload,
    /// Reclaim a tracked reference script back into the wallet.
    Teardown { hash: Hash<28> },
    /// Start tracking channels at an additional delegation.
    AddDelegation { credential: Credential },
    /// Stop tracking channels at a delegation.
    RemoveDelegation { credential: Credential },
}

impl SessionCmd {
    pub async fn run(self, mut ctx: Ctx) -> Result<()> {
        let mut session = build(&ctx.config.session).await?;
        match self {
            SessionCmd::Status => {
                print_status(&session);
                Ok(())
            }
            SessionCmd::Upload => {
                session.upload().await?;
                println!("validator ref script uploaded and confirmed");
                Ok(())
            }
            SessionCmd::Teardown { hash } => {
                let id = session.teardown(&hash).await?;
                println!("teardown submitted: {id}");
                Ok(())
            }
            SessionCmd::AddDelegation { credential } => {
                let added = session.add_delegation(credential).await?;

                ctx.config.session.delegations = session.delegations().clone();
                ctx.config.session.script_host = session.script_host().cloned();
                ctx.save()?;

                println!(
                    "{}",
                    if added {
                        "delegation added"
                    } else {
                        "delegation already present"
                    }
                );
                Ok(())
            }
            SessionCmd::RemoveDelegation { credential } => {
                let removed = session.remove_delegation(&credential)?;

                ctx.config.session.delegations = session.delegations().clone();
                ctx.config.session.script_host = session.script_host().cloned();
                ctx.save()?;

                println!(
                    "{}",
                    if removed {
                        "delegation removed"
                    } else {
                        "delegation was not tracked"
                    }
                );
                Ok(())
            }
        }
    }
}
