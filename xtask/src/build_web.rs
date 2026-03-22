// Stub — will be implemented by agent
use clap::Args;

#[derive(Args)]
pub struct BuildWebArgs {}

pub fn run(_args: BuildWebArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
