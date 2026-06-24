//! ZK Trusted Setup Binary
//!
//! Generates proving and verifying keys for the StateTransitionCircuit.
//!
//! Usage:
//!   cargo run --bin zk_setup --features privacy -- --batch-size 100 --output-dir data/zk/
//!
//! This generates:
//!   - {output_dir}/proving_key.bin   - Proving key (secret, for provers)
//!   - {output_dir}/verifying_key.bin - Verifying key (public, for verifiers)

#[cfg(feature = "privacy")]
use tracing::{error, info, warn};

#[cfg(feature = "privacy")]
use ark_bn254::Bn254;
#[cfg(feature = "privacy")]
use ark_groth16::Groth16;
#[cfg(feature = "privacy")]
use ark_serialize::CanonicalSerialize;
#[cfg(feature = "privacy")]
use ark_snark::SNARK;
#[cfg(feature = "privacy")]
use ark_std::rand::{rngs::StdRng, RngCore, SeedableRng};
#[cfg(feature = "privacy")]
use std::fs::{self, File};
#[cfg(feature = "privacy")]
use std::io::BufWriter;
#[cfg(feature = "privacy")]
use std::path::Path;

#[cfg(feature = "privacy")]
use irondag::zk::StateTransitionCircuit;

#[cfg(feature = "privacy")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("irondag=info".parse().unwrap()),
        )
        .init();

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();

    let mut batch_size: usize = 100;
    let mut output_dir = "data/zk".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--batch-size" => {
                if i + 1 < args.len() {
                    batch_size = args[i + 1].parse()?;
                    i += 2;
                } else {
                    error!("--batch-size requires a value");
                    std::process::exit(1);
                }
            }
            "--output-dir" => {
                if i + 1 < args.len() {
                    output_dir = args[i + 1].clone();
                    i += 2;
                } else {
                    error!("--output-dir requires a value");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                info!("ZK Trusted Setup - Generate proving and verifying keys");
                info!("");
                info!("Usage: cargo run --bin zk_setup --features privacy -- [OPTIONS]");
                info!("");
                info!("Options:");
                info!("    --batch-size <N>     Number of transactions per batch (default: 100)");
                info!("    --output-dir <PATH>  Output directory for keys (default: data/zk/)");
                info!("    -h, --help           Show this help message");
                info!("");
                info!("Output files:");
                info!(
                    "    {}/proving_key.bin   - Proving key (secret, for provers)",
                    output_dir
                );
                info!(
                    "    {}/verifying_key.bin - Verifying key (public, for verifiers)",
                    output_dir
                );
                std::process::exit(0);
            }
            _ => {
                i += 1;
            }
        }
    }

    info!("ZK Trusted Setup - Key Generation");
    info!("Configuration:");
    info!("Batch size: {}", batch_size);
    info!("Output directory: {}", output_dir);

    // Create output directory if it doesn't exist
    let output_path = Path::new(&output_dir);
    if !output_path.exists() {
        fs::create_dir_all(output_path)?;
        info!("Created output directory: {}", output_dir);
    }

    // Create a deterministic RNG seeded from OsRng for testnet setup
    // This ensures reproducibility while still being unpredictable
    let mut os_rng = ark_std::rand::rngs::OsRng;
    let seed = os_rng.next_u64();
    let mut rng = StdRng::seed_from_u64(seed);
    info!("Using seeded RNG (seed from OsRng for testnet setup)");
    info!("Seed: {} (save for reproducibility)", seed);

    // Create a dummy circuit with the given batch size
    // All witnesses are set to None - this is the "shape" circuit for setup
    info!("Creating circuit with {} transactions...", batch_size);
    let circuit = StateTransitionCircuit::<ark_bn254::Fr>::new_batch(batch_size);

    // Perform circuit-specific setup (trusted setup)
    info!("Generating proving and verifying keys...");
    info!("This may take several minutes for large batch sizes...");

    let start = std::time::Instant::now();
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(circuit.clone(), &mut rng)
        .map_err(|e| format!("Circuit setup failed: {:?}", e))?;
    let duration = start.elapsed();

    info!("Key generation completed in {:.2?}", duration);

    // Serialize proving key
    let pk_path = output_path.join("proving_key.bin");
    info!("Serializing proving key to: {}", pk_path.display());

    let pk_file = File::create(&pk_path)?;
    let mut pk_writer = BufWriter::new(pk_file);
    pk.serialize_uncompressed(&mut pk_writer)
        .map_err(|e| format!("Failed to serialize proving key: {:?}", e))?;

    // Get file size
    let pk_metadata = fs::metadata(&pk_path)?;
    let pk_size_kb = pk_metadata.len() as f64 / 1024.0;

    // Serialize verifying key
    let vk_path = output_path.join("verifying_key.bin");
    info!("Serializing verifying key to: {}", vk_path.display());

    let vk_file = File::create(&vk_path)?;
    let mut vk_writer = BufWriter::new(vk_file);
    vk.serialize_uncompressed(&mut vk_writer)
        .map_err(|e| format!("Failed to serialize verifying key: {:?}", e))?;

    // Get file size
    let vk_metadata = fs::metadata(&vk_path)?;
    let vk_size_kb = vk_metadata.len() as f64 / 1024.0;

    info!("Key Generation Summary");
    info!("Proving Key:   {:.2} KB", pk_size_kb);
    info!("Verifying Key: {:.2} KB", vk_size_kb);
    info!("Batch Size:    {} transactions", batch_size);
    info!("Files:");
    info!("  {}", pk_path.display());
    info!("  {}", vk_path.display());
    info!("IMPORTANT:");
    info!("- Keep proving_key.bin SECRET (used by provers)");
    info!("- verifying_key.bin can be PUBLIC (used by verifiers)");
    info!("- Both keys are specific to batch size {}", batch_size);
    info!("- Regenerate keys if you change the circuit structure");

    Ok(())
}

#[cfg(not(feature = "privacy"))]
fn main() {
    eprintln!("Error: This binary requires the 'privacy' feature to be enabled.");
    eprintln!("Run with: cargo run --bin zk_setup --features privacy -- [OPTIONS]");
    std::process::exit(1);
}
