use crate::cli::DefaultArgs;
use clap::Args;
use tracing::info;

#[derive(Args, Debug, Clone)]
pub struct BuildCommand {
    #[clap(flatten)]
    pub default_args: DefaultArgs,
}

impl BuildCommand {
    pub fn run(self) -> anyhow::Result<()> {
        info!("Building");
        Ok(())
    }
}
