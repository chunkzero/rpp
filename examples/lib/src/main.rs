use rpp::build::BuildEngine;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example using the new BuildEngine API
    let mut engine = BuildEngine::builder()
        .source_dir("./examples/sample_pack")
        .output_dir("./examples/sample_pack/dist")
        .build()?;

    let result = engine.build()?;

    println!("Build complete: {} files processed", result.files_processed);

    Ok(())
}
