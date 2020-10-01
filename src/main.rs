use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use cbindgen;
use clap::{App, Arg, SubCommand};

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
    let matches = App::new("unreal_rust_compile")
        .version("0.1")
        .author("Elliott Mahler <jointogethe.r@gmail.com>")
        .about("Runs cargo and cbindgen on a crate. Intended for use with Unreal Engine's build system.")
        .subcommand(SubCommand::with_name("gen-bindings")
            .about("Generate bindings using cbindgen")
            .version("0.1")
            .arg(Arg::with_name("CRATE_DIR").long("crate_dir").required(true).takes_value(true).help("Input crate directory"))
            .arg(Arg::with_name("OUTPUT_HEADER_FILE").long("output_header_file").required(true).takes_value(true).help("Destination filename for the generated C header")
            )
        )
        .subcommand(SubCommand::with_name("rustc")
            .about("Compile crate")
            .version("0.1")
            .arg(Arg::with_name("OUTPUT_LINKER_FILE").long("output_linker_file").required(true).takes_value(true).help("Path to output linker args at"))
            .arg(Arg::with_name("OUTPUT_LIB_LINK_FILE").long("output_lib_link_file").required(true).takes_value(true).help("Path to output library linker (LIB.EXE) args at"))
            .arg(Arg::with_name("CARGO_ARGS").multiple(true).last(true).allow_hyphen_values(true).help("Arguments to cargo. Cargo will be run with \"crate_dir\" as the working directory."))
        )
        .subcommand(SubCommand::with_name("source-files")
            .about("Get a list of all source files required to compile the crate")
            .version("0.1")
            .arg(Arg::with_name("CARGO_ARGS").multiple(true).last(true).required(false).allow_hyphen_values(true).help("Arguments to cargo. Cargo will be run with \"crate_dir\" as the working directory."))
        )
	.get_matches();

    // pull arguments from the argument parser
    if let Some(matches) = matches.subcommand_matches("gen-bindings") {
        let crate_dir: PathBuf = matches
            .value_of("CRATE_DIR")
            .expect("crate_dir not provided")
            .into();
        let header_path: &str = matches
            .value_of("OUTPUT_HEADER_FILE")
            .expect("output_header_file not provided");
        let generated = cbindgen::generate(&crate_dir).expect("Couldn't generate headers.");
        let mut new_data: Vec<u8> = Vec::new();
        let mut existing_data: Vec<u8> = Vec::new();

        generated.write(&mut new_data);
        if let Ok(mut current_header) = std::fs::File::open(&header_path) {
            use std::io::Read;
            current_header.read_to_end(&mut existing_data)?;
        }
        std::fs::create_dir_all(PathBuf::from(header_path).parent().unwrap())?;
        if new_data != existing_data {
            std::fs::create_dir_all(PathBuf::from(header_path).parent().unwrap())?;
            std::fs::write(&header_path, new_data)?;
            println!("Header changed");
        }
    }

    if let Some(matches) = matches.subcommand_matches("rustc") {
        let output_linker_file: &str = matches
            .value_of("OUTPUT_LINKER_FILE")
            .expect("output_linker_file not provided");
        let output_linker_file: PathBuf = output_linker_file.into();
        let output_lib_link_file: PathBuf = matches
            .value_of("OUTPUT_LIB_LINK_FILE")
            .expect("output_lib_link_file not provided")
            .into();
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
        let rustc_arg = Vec::from(["rustc"]);
        let compile_result = Command::new("cargo")
            .args(
                rustc_arg
                    .into_iter()
                    .chain(cargo_args.chain(extra_cargo_args.into_iter())),
            )
            .output();

        // If the cargo command completed with errors, return a nonzero status code
        let command_success = match compile_result {
            Ok(output) => {
                let text = std::str::from_utf8(&output.stderr).expect("Cargo did not output utf8");
                println!("{}", text); // output the compiler output
                let stdout =
                    std::str::from_utf8(&output.stdout).expect("Cargo did not output utf8");
                let mut success = false;
                // println!("stdout {}", stdout);
                if let Some(last_line) = stdout.lines().last() {
                    if last_line.contains(".def") {
                        success = true;
                        let mut output_linker_file = std::fs::File::create(&output_linker_file)?;
                        let mut output_lib_file = std::fs::File::create(&output_lib_link_file)?;
                        let args = parse_quotes(&last_line);
                        if let Some(linker_flavor) = args.first() {
                            let linker_flavor_path = PathBuf::from(linker_flavor);
                            let linker_flavor_filename = linker_flavor_path
                                .file_name()
                                .unwrap_or(std::ffi::OsStr::new(""));
                            match linker_flavor_filename.to_str().unwrap_or("") {
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
                                        let def_file_path =
                                            output_lib_link_file.with_file_name("build_def.def");
                                        if let Ok(_metadata) = std::fs::metadata(option_arg) {
                                            std::fs::copy(option_arg, &def_file_path)
                                                .expect("Failed to copy def file");
                                        }
                                        // include DEF file for both linker and lib
                                        writeln!(
                                            &mut output_linker_file,
                                            "/DEF:\"{}\"",
                                            def_file_path.to_string_lossy()
                                        )?;
                                        writeln!(
                                            &mut output_lib_file,
                                            "/DEF:\"{}\"",
                                            def_file_path.to_string_lossy()
                                        )?;
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
                success
            }
            Err(err) => {
                eprintln!("Compile error: {}", err);
                false
            }
        };
        if !command_success {
            std::process::exit(1);
        }
    }
    Ok(())
}
