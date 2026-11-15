use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use tdlib_rs_parser::errors::ParseError;
use tdlib_rs_parser::tl::Definition;

const BUILTINS_END: &str = "vector {t:Type} # [ t ] = Vector t;";

const STATIC_LIBS: [&str; 14] = [
    "tdjson_static",
    "tdjson_private",
    "tdclient",
    "tdcore",
    "tdapi",
    "tdmtproto",
    "tdnet",
    "tddb",
    "tdsqlite",
    "tde2e",
    "tdactor",
    "tdutils",
    "ssl",
    "crypto",
];

fn main() {
    println!("cargo:rerun-if-env-changed=TDLIB_DIR");
    let tdlib =
        PathBuf::from(env::var("TDLIB_DIR").expect("TDLIB_DIR must point at the output of scripts/build-tdlib.sh"));
    let schema = tdlib.join("src/td/generate/scheme/td_api.tl");
    println!("cargo:rerun-if-changed={}", schema.display());
    let source = fs::read_to_string(&schema).unwrap_or_else(|e| panic!("cannot read {}: {e}", schema.display()));
    let builtins =
        format!("{}{BUILTINS_END}", source.split_once(BUILTINS_END).expect("TDLib schema has no builtin header").0);
    let (definitions, skipped) = parse(source);
    assert_eq!(skipped, parse(builtins).1, "TDLib schema has unparseable definitions");
    let mut out =
        BufWriter::new(File::create(PathBuf::from(env::var("OUT_DIR").unwrap()).join("generated.rs")).unwrap());
    td_gen::generate_rust_code(&mut out, &definitions, false).unwrap();
    out.flush().unwrap();

    println!("cargo:rustc-link-search=native={}", tdlib.join("install/lib").display());
    for lib in STATIC_LIBS {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    println!("cargo:rustc-link-lib=dylib=z");
    println!("cargo:rustc-link-lib=dylib=c++");
}

fn parse(source: String) -> (Vec<Definition>, usize) {
    let mut skipped = 0;
    let definitions = tdlib_rs_parser::parse_tl_file(source)
        .filter_map(|result| match result {
            Ok(definition) => Some(definition),
            Err(ParseError::NotImplemented) => {
                skipped += 1;
                None
            }
            Err(e) => panic!("invalid TDLib schema: {e:?}"),
        })
        .collect();
    (definitions, skipped)
}
