use clap::Args;
use tracing::info;
use crate::cli::DefaultArgs;

#[derive(Args, Debug, Clone)]
pub struct BuildCommand {
    #[clap(flatten)]
    pub default_args: DefaultArgs
}

impl BuildCommand {
    pub fn run(self) -> anyhow::Result<()> {
        info!("Building");
        Ok(())
    }
}