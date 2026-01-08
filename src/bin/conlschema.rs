//! CLI tool for validating CONL documents against a schema.
//!
//! Usage: conlschema --schema <schema> [input]
//!
//! If no input file is provided, reads from stdin.

use conl::schema::Schema;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut schema_path: Option<String> = None;
    let mut input_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--schema" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --schema requires a path argument");
                    print_usage();
                    process::exit(1);
                }
                schema_path = Some(args[i + 1].clone());
                i += 2;
            }
            "-h" | "--help" => {
                print_usage();
                process::exit(0);
            }
            arg if arg.starts_with('-') => {
                eprintln!("Error: unknown option: {}", arg);
                print_usage();
                process::exit(1);
            }
            _ => {
                input_path = Some(args[i].clone());
                i += 1;
            }
        }
    }

    let schema_path = match schema_path {
        Some(p) => p,
        None => {
            eprintln!("Error: --schema is required");
            print_usage();
            process::exit(1);
        }
    };

    // Read and parse schema
    let schema_content = match fs::read(&schema_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading schema file '{}': {}", schema_path, e);
            process::exit(1);
        }
    };

    let schema = match Schema::parse(&schema_content) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error parsing schema: {}", e);
            process::exit(1);
        }
    };

    // Read input
    let input = match input_path {
        Some(path) => match fs::read(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error reading input file '{}': {}", path, e);
                process::exit(1);
            }
        },
        None => {
            let mut buffer = Vec::new();
            match io::stdin().read_to_end(&mut buffer) {
                Ok(_) => buffer,
                Err(e) => {
                    eprintln!("Error reading stdin: {}", e);
                    process::exit(1);
                }
            }
        }
    };

    // Validate
    let result = schema.validate(&input);
    let errors = result.errors();

    if errors.is_empty() {
        process::exit(0);
    }

    for error in &errors {
        println!("{}", error);
    }

    process::exit(errors.len().min(255) as i32);
}

fn print_usage() {
    eprintln!("Usage: conlschema --schema <schema> [input]");
    eprintln!();
    eprintln!("Validate a CONL document against a schema.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -s, --schema <path>  Path to the schema file (required)");
    eprintln!("  -h, --help           Show this help message");
    eprintln!();
    eprintln!("If no input file is provided, reads from stdin.");
}
