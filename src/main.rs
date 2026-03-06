use apexstore::{LsmConfig, LsmEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = LsmConfig::builder()
        .dir_path("/var/lib/apexstore/data")
        .build()?;

    let _engine = LsmEngine::new(config)?;
    Ok(())
}
