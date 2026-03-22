// Stub — will be implemented by agent
use clap::Args;

#[derive(Args)]
pub struct CiArgs {}

pub fn run(_args: CiArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
