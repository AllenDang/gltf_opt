use gltf_opt::prelude::optimize;
use std::fs::File;
use std::io::{BufReader, BufWriter};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <input.glb> <output.glb>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    let file = File::open(input_path).expect("Failed to open input file");
    let mut reader = BufReader::new(file);

    // Optimize with center_pivot = true, no texture conversion
    let result = optimize(&mut reader, 1024, false, false, true)
        .expect("Failed to optimize");

    let output = File::create(output_path).expect("Failed to create output file");
    let mut writer = BufWriter::new(output);
    std::io::Write::write_all(&mut writer, &result).expect("Failed to write output");

    println!("Done! Output written to {}", output_path);
}
