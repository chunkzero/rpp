use clap::Args;
use crate::cli::DefaultArgs;

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    #[clap(flatten)]
    pub default_args: DefaultArgs
}