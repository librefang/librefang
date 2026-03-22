// Stub — will be implemented by agent
use clap::Args;

#[derive(Args)]
pub struct SyncVersionsArgs {}

pub fn run(_args: SyncVersionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
