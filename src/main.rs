use apexstore::{LsmConfig, LsmEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = LsmConfig::builder()
        .dir_path("/var/lib/apexstore/data")
        .build()?;

    let _engine = LsmEngine::new_from_config(&config, apexstore::storage::cache::GlobalBlockCache::new(100, 4096))?;
    Ok(())
}
