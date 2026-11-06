//! Rust code generator for [`hyperschema`](https://github.com/holepunchto/hyperschema)
//! `schema.json` files.
//!
//! The output is byte-compatible with the JS generated `index.js`: same field
//! order, same struct flags word, same version handling. Generated modules
//! depend on `compact_encoding` and `bytes` only.
//!
//! ```ignore
//! // build.rs
//! let opts = hyperschema::Options::new();
//! hyperschema::build("spec/hyperschema/schema.json", "hyperschema.rs", &opts).unwrap();
//! ```
//!
//! ```ignore
//! mod schema {
//!     include!(concat!(env!("OUT_DIR"), "/hyperschema.rs"));
//! }
//! ```
//!
//! The file must be included inside a module: type references across schema
//! namespaces are relative (`super::other_namespace::Type`).

#![forbid(unsafe_code)]

mod codegen;
mod error;
mod model;
mod naming;

use std::path::{Path, PathBuf};

pub use codegen::Options;
pub use error::{Error, Result};

/// Generates one Rust source file for a whole `schema.json`.
pub fn generate(schema_json: &str, opts: &Options) -> Result<String> {
    let model = model::parse(schema_json)?;
    codegen::generate(&model, opts)
}

/// Generates `OUT_DIR/<out_file>` from `schema_path` and asks cargo to rerun
/// the build script when the schema changes.
pub fn build(schema_path: impl AsRef<Path>, out_file: &str, opts: &Options) -> Result<()> {
    let path = schema_path.as_ref();
    let json = std::fs::read_to_string(path)?;
    let code = generate(&json, opts)?;
    let out_dir = std::env::var_os("OUT_DIR").ok_or(Error::NoOutDir)?;
    std::fs::write(PathBuf::from(out_dir).join(out_file), code)?;
    println!("cargo:rerun-if-changed={}", path.display());
    Ok(())
}
