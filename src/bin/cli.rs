use apexstore::cli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Ok(cli::main()?)
}
