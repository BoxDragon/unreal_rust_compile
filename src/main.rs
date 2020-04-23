use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use cbindgen;
use clap::clap_app;

pub fn parse_quotes(s: impl AsRef<str>) -> Vec<String> {
    let s = s.as_ref();
    let mut args = vec![];
    let mut in_string = false;
    let mut escaping = false;
    let mut current_str = String::default();

    for x in s.chars() {
        if in_string {
            if x == '\\' && !escaping {
                escaping = true;
            } else if x == '"' && !escaping {
                if !current_str.is_empty() {
                    args.push(current_str);
                }

                current_str = String::default();
                in_string = false;
            } else {
                current_str.push(x);
                escaping = false;
            }
        } else if x == ' ' {
            if !current_str.is_empty() {
                args.push(current_str.clone());
            }

            current_str = String::default();
        } else if x == '"' {
            if !current_str.is_empty() {
                args.push(current_str.clone());
            }

            in_string = true;
            current_str = String::default();
        } else {
            current_str.push(x);
        }
    }

    if !current_str.is_empty() {
        args.push(current_str);
    }

    args
}

fn main() -> Result<()> {
    let matches = clap_app!(unreal_rust_compile =>
        (version: "0.1")
        (author: "Elliott Mahler <jointogethe.r@gmail.com>")
        (about: "Runs cargo and cbindgen on a crate. Intended for use with Unreal Engine's build system.")
        (@arg CRATE_DIR: --crate_dir +required +takes_value "Input crate directory")
        (@arg OUTPUT_HEADER_FILE: --output_header_file +takes_value "Destination filename for the generated header")
        (@arg OUTPUT_LINKER_FILE: --output_linker_file +takes_value "Path to output linker args at")
        (@arg OUTPUT_LIB_LINK_FILE: --output_lib_link_file +takes_value "Path to output library linker (LIB.EXE) args at")
        (@arg CARGO_ARGS: +multiple +last +allow_hyphen_values "Arguments to cargo. Cargo will be run with \"crate_dir\" as the working directory.")
	).get_matches();

    // pull arguments from the argument parser
    let crate_dir: PathBuf = matches
        .value_of("CRATE_DIR")
        .expect("crate_dir not provided")
        .into();
    let header_path: Option<&str> = matches.value_of("OUTPUT_HEADER_FILE");
    if let Some(header_path) = header_path {
        let generated = cbindgen::generate(&crate_dir).expect("Couldn't generate headers.");
        generated.write_to_file(&header_path);
    }
    let output_linker_file: Option<&str> = matches.value_of("OUTPUT_LINKER_FILE");
    if output_linker_file.is_none() {
        return Ok(());
    }
    let output_linker_file: PathBuf = output_linker_file.unwrap().into();
    println!(
        "output linker file {}",
        output_linker_file.to_string_lossy()
    );
    let output_lib_link_file: PathBuf = matches
        .value_of("OUTPUT_LIB_LINK_FILE")
        .expect("output_lib_link_file not provided")
        .into();
    println!(
        "output lib link file {}",
        output_lib_link_file.to_string_lossy()
    );
    let cargo_args = matches
        .values_of("CARGO_ARGS")
        .expect("No cargo args provided");
    use itertools::join;

    eprintln!("Cargo args {}", join(cargo_args.clone(), ", "));
    eprintln!("env args {}", join(std::env::args(), ", "));

    let mut extra_cargo_args = Vec::new();
    let rand_arg = format!("-Clink-arg=/VERSION:{}", rand::random::<u16>());
    extra_cargo_args.extend(&["-Z", "print-link-args", "-C", "save-temps", &rand_arg]);
    // generate the header file data and write it into a vec of bytes
    // Build the cargo command from the args
    let compile_result = Command::new("cargo")
        .args(cargo_args.chain(extra_cargo_args.into_iter()))
        .output();

    // If the cargo command completed with errors, return a nonzero status code
    let command_success = match compile_result {
        Ok(output) => {
            let text = std::str::from_utf8(&output.stderr).expect("Cargo did not output utf8");
            println!("{}", text); // output the compiler output
            let stdout = std::str::from_utf8(&output.stdout).expect("Cargo did not output utf8");
            // println!("stdout {}", stdout);
            if let Some(last_line) = stdout.lines().last() {
                if last_line.contains(".def") {
                    let mut output_linker_file = std::fs::File::create(&output_linker_file)?;
                    let mut output_lib_file = std::fs::File::create(&output_lib_link_file)?;
                    let args = parse_quotes(&last_line);
                    if let Some(linker_flavor) = args.first() {
                        match linker_flavor.as_str() {
                            "link.exe" | "lld-link.exe" => {}
                            _ => panic!("Unrecognized linker flavor {}", linker_flavor),
                        }
                    } else {
                        panic!("No linker args found!");
                    }
                    for arg in &args[1..] {
                        if arg.starts_with("/") || arg.starts_with("-") {
                            let arg_end_idx = arg.find(":");
                            let option_name = &arg[1..arg_end_idx.unwrap_or(arg.len())];
                            let option_arg = if let Some(arg_end_idx) = arg_end_idx {
                                &arg[(1 + arg_end_idx)..]
                            } else {
                                ""
                            };
                            match option_name {
                                "LIBPATH" | "IMPLIB" => {
                                    writeln!(
                                        &mut output_linker_file,
                                        "/{}:\"{}\"",
                                        option_name, option_arg
                                    )?;
                                }
                                "DEF" => {
                                    let def_file_path = output_lib_link_file.with_file_name("build_def.def");
                                    std::fs::copy(option_arg, &def_file_path).expect("Failed to copy def file");
                                    // include DEF file for both linker and lib
                                    writeln!(&mut output_linker_file, "/DEF:\"{}\"", def_file_path.to_string_lossy())?;
                                    writeln!(&mut output_lib_file, "/DEF:\"{}\"", def_file_path.to_string_lossy())?;
                                }
                                _ => {}
                            }
                        } else {
                            if arg.ends_with(".o") || arg.ends_with(".rlib") {
                                // include only object/rlib files in lib file
                                writeln!(&mut output_lib_file, "\"{}\"", arg)?;
                            } else {
                                writeln!(&mut output_linker_file, "\"{}\"", arg)?;
                            }
                        }
                    }
                } else {
                    println!("NO LINKER ARGS");
                }
            }
            true
        }
        Err(_) => false,
    };
    if !command_success {
        std::process::exit(1);
    } else {
        Ok(())
    }
}
