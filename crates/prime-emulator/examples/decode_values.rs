//! Replay bounded captured objects without firmware, USB or guest execution.
use clap::Parser;
use prime_emulator::runtime_value::{self, RepresentationGraph, ValueEvidence};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Deserialize)]
struct Input {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    representation: RepresentationGraph,
}

#[derive(Serialize)]
struct Decoded {
    id: String,
    evidence: ValueEvidence,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let input: Input = serde_json::from_slice(&std::fs::read(args.input)?)?;
    let results: Vec<_> = input
        .cases
        .into_iter()
        .map(|case| Decoded {
            id: case.id,
            evidence: runtime_value::decode_representation(case.representation),
        })
        .collect();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(args.output)?
        .write_all(&serde_json::to_vec_pretty(&results)?)?;
    println!("Replayed {} captured value(s)", results.len());
    Ok(())
}
