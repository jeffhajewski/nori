//! Build script to compile protobuf definitions.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the proto file from context/40_protocols.proto
    tonic_build::compile_protos("../../context/40_protocols.proto")?;
    Ok(())
}
