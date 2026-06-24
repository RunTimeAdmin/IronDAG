fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile protobuf files with tonic-build (generates gRPC server code)
    // v1: Hex string encoding (JSON-RPC compatible)
    // v2: Binary encoding (3.3x faster internal gRPC)
    tonic_build::configure()
        .build_server(true)
        .build_client(false) // We only need server
        .compile(&["proto/rpc.proto", "proto/rpc_v2.proto"], &["proto/"])?;

    // Re-run if proto files change
    println!("cargo:rerun-if-changed=proto/rpc.proto");
    println!("cargo:rerun-if-changed=proto/rpc_v2.proto");

    Ok(())
}
