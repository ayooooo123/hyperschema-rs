// Generates the codecs and fixture expectations the golden tests consume.
#![allow(dead_code)]

#[path = "src/codegen.rs"]
mod codegen;
#[path = "src/error.rs"]
mod error;
#[path = "src/model.rs"]
mod model;
#[path = "src/naming.rs"]
mod naming;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use codegen::Options;
use error::Result;
use model::{Kind, Model, Prim, Ty};
use serde_json::Value;

/// The five modules PearTube ships plus one schema that exercises features the
/// real ones do not use.
const SCHEMAS: &[&str] = &[
    "hyperbee2",
    "autobee",
    "protomux-wakeup",
    "hypercore-storage",
    "hyperdht-address",
    "synthetic",
];

const AUTOBEE_EXTERNALS: &[(&str, &str)] = &[
    ("BootRecordV0", "crate::legacy::BootRecordV0"),
    ("OplogMessageV0", "crate::legacy::OplogMessageV0"),
    ("OplogMessageV1", "crate::legacy::OplogMessageV1"),
    ("SystemWriterV0", "crate::legacy::SystemWriterV0"),
];

fn main() {
    println!("cargo:rerun-if-changed=fixtures");
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    for name in SCHEMAS {
        if let Err(e) = generate(name, &root) {
            panic!("generating {name}: {e}");
        }
    }
}

fn schema_module(name: &str) -> String {
    format!("{}_spec", name.replace('-', "_"))
}

fn generate(name: &str, root: &Path) -> Result<()> {
    let schema_path = root
        .join("fixtures")
        .join("schemas")
        .join(name)
        .join("schema.json");
    println!("cargo:rerun-if-changed={}", schema_path.display());

    let json = std::fs::read_to_string(&schema_path)?;
    let m = model::parse(&json)?;

    let mut opts = Options::new();
    if name == "autobee" {
        for (external, path) in AUTOBEE_EXTERNALS {
            opts = opts.external(*external, *path);
        }
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(
        out_dir.join(format!("{name}.rs")),
        codegen::generate(&m, &opts)?,
    )?;

    let cases_path = root.join("fixtures").join(format!("{name}.json"));
    if !cases_path.exists() {
        return Ok(());
    }
    println!("cargo:rerun-if-changed={}", cases_path.display());
    let cases = std::fs::read_to_string(&cases_path)?;
    let cases = Cases::parse(&cases)?;
    std::fs::write(
        out_dir.join(format!("cases_{name}.rs")),
        emit_cases(&m, name, &cases)?,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fixtures/<schema>.json

struct Cases {
    cases: Vec<Case>,
}

struct Case {
    name: String,
    version: u64,
    hex: String,
    value: Value,
    /// The schema version to decode with, when it differs from the one the
    /// bytes were written with (an older writer read by a newer codec).
    decode_version: Option<u64>,
    /// The bytes the value re-encodes to, when that differs from `hex`
    /// (a frame the current codec writes differently).
    reencode: Option<String>,
    /// The bytes must fail to decode.
    error: bool,
    note: Option<String>,
}

impl Cases {
    fn parse(text: &str) -> Result<Self> {
        let json: Value = serde_json::from_str(text)?;
        let cases = json["cases"]
            .as_array()
            .ok_or_else(|| error::Error::Schema("fixture file without cases".into()))?
            .iter()
            .map(|c| Case {
                name: c["name"].as_str().unwrap_or_default().to_string(),
                version: c["version"].as_u64().unwrap_or(1),
                hex: c["hex"].as_str().unwrap_or_default().to_string(),
                value: c["value"].clone(),
                decode_version: c["decodeVersion"].as_u64(),
                reencode: c["reencode"].as_str().map(str::to_string),
                error: c["error"].as_bool().unwrap_or(false),
                note: c["note"].as_str().map(str::to_string),
            })
            .collect();
        Ok(Self { cases })
    }
}

fn emit_cases(m: &Model, schema: &str, cases: &Cases) -> Result<String> {
    let module = schema_module(schema);
    let mut cases_body = String::new();
    let mut impls: BTreeSet<usize> = BTreeSet::new();

    for case in &cases.cases {
        let ty = find(m, &case.name).ok_or_else(|| {
            error::Error::Schema(format!("fixture names unknown type `{}`", case.name))
        })?;
        let idx = type_index(&ty).ok_or_else(|| {
            error::Error::Schema(format!("`{}` has no generated type", case.name))
        })?;
        let label = match &case.note {
            Some(note) => format!("{} v{} ({note})", case.name, case.version),
            None => format!("{} v{}", case.name, case.version),
        };
        let hex = hex_literal(&case.hex);
        let versioned = matches!(ty, Ty::Struct(_) | Ty::Versioned(_));
        let mut decl = String::new();
        let check = if case.error {
            // A broken frame must be an error, not a panic or a wrong value.
            impls.insert(idx);
            format!(
                "crate::check_invalid::<{}>({hex}, {});",
                path(m, &module, idx),
                case.version
            )
        } else {
            let mut value = String::new();
            value_literal(m, &module, &ty, &case.value, &mut value)?;
            decl = format!("let expected = {value};\n{IND}{IND}{IND}{IND}");
            if versioned {
                impls.insert(idx);
                let decode = case.decode_version.unwrap_or(case.version);
                let reencode = match &case.reencode {
                    Some(hex) => hex_literal(hex),
                    None => hex.clone(),
                };
                format!(
                    "crate::check(&expected, {}, {decode}, {hex}, {reencode});",
                    case.version
                )
            } else {
                format!("crate::check_codec(&expected, {hex});")
            }
        };
        let _ = writeln!(
            cases_body,
            "{IND}{IND}Case {{\n\
             {IND}{IND}{IND}name: {label:?},\n\
             {IND}{IND}{IND}run: Box::new(|| {{\n\
             {IND}{IND}{IND}{IND}{decl}{check}\n\
             {IND}{IND}{IND}}}),\n\
             {IND}{IND}}},"
        );
    }

    let mut impl_body = String::new();
    for idx in &impls {
        let path = path(m, &module, *idx);
        let _ = writeln!(
            impl_body,
            "\nimpl VersionedValue for {path} {{\n\
             {IND}fn encoded_len_at(&self, version: u64) -> usize {{\n\
             {IND}{IND}self.encoded_len_version(version)\n\
             {IND}}}\n\
             {IND}fn encode_at(&self, version: u64, w: &mut Writer) {{\n\
             {IND}{IND}self.encode_version(version, w);\n\
             {IND}}}\n\
             {IND}fn decode_at(r: &mut Reader, version: u64) -> compact_encoding::Result<Self> {{\n\
             {IND}{IND}Self::decode_version(r, version)\n\
             {IND}}}\n\
             }}"
        );
    }

    let mut out = String::new();
    let _ = writeln!(out, "// Generated by build.rs from fixtures/{schema}.json.");
    let _ = writeln!(out, "use crate::Case;");
    if !impls.is_empty() {
        let _ = writeln!(out, "use crate::VersionedValue;");
    }
    if cases_body.contains("Bytes::") {
        let _ = writeln!(out, "use bytes::Bytes;");
    }
    if !impls.is_empty() {
        let _ = writeln!(out, "use compact_encoding::{{Reader, Writer}};");
    }
    let net: Vec<&str> = [
        "IpAddr",
        "Ipv4Addr",
        "Ipv6Addr",
        "SocketAddr",
        "SocketAddrV4",
        "SocketAddrV6",
    ]
    .into_iter()
    .filter(|n| cases_body.contains(&format!("{n}::")))
    .collect();
    if !net.is_empty() {
        let _ = writeln!(out, "use std::net::{{{}}};", net.join(", "));
    }

    let _ = writeln!(out, "\npub fn cases() -> Vec<Case> {{");
    let _ = writeln!(out, "{IND}vec![");
    out.push_str(&cases_body);
    let _ = writeln!(out, "{IND}]");
    let _ = writeln!(out, "}}");
    out.push_str(&impl_body);
    Ok(out)
}

fn type_index(ty: &Ty) -> Option<usize> {
    match ty {
        Ty::Struct(i)
        | Ty::Enum(i)
        | Ty::Array(i)
        | Ty::Record(i)
        | Ty::Versioned(i)
        | Ty::External(i) => Some(*i),
        Ty::Prim(_) => None,
    }
}

const IND: &str = "    ";

fn hex_literal(hex: &str) -> String {
    format!("crate::hex({hex:?})")
}

fn find(m: &Model, fqn: &str) -> Option<Ty> {
    m.types
        .iter()
        .position(|t| t.fqn == fqn)
        .map(|i| named_ty(m, i))
}

fn named_ty(m: &Model, index: usize) -> Ty {
    match &m.types[index].kind {
        Kind::Struct(_) => Ty::Struct(index),
        Kind::Enum(_) => Ty::Enum(index),
        Kind::Array(_) => Ty::Array(index),
        Kind::Record(_) => Ty::Record(index),
        Kind::Alias(a) => a.target.clone(),
        Kind::Versioned(_) => Ty::Versioned(index),
        Kind::External(_) => Ty::External(index),
    }
}

fn path(m: &Model, module: &str, index: usize) -> String {
    let t = &m.types[index];
    format!(
        "crate::{module}::{}::{}",
        m.namespaces[t.ns].module, t.rust_name
    )
}

/// Renders the fixture value as a Rust expression of the generated type.
fn value_literal(m: &Model, module: &str, ty: &Ty, json: &Value, out: &mut String) -> Result<()> {
    match ty {
        Ty::Prim(p) => prim_literal(*p, json, out),
        Ty::Struct(i) => struct_literal(m, module, *i, json, out),
        Ty::Enum(i) => {
            let e = m.as_enum(*i);
            let variant = if let Some(key) = json.as_str() {
                e.variants
                    .iter()
                    .find(|(_, k)| k == key)
                    .map(|(rust, _)| rust.clone())
            } else {
                let n = json.as_u64().unwrap_or(0);
                e.variants
                    .get(n.wrapping_sub(e.offset) as usize)
                    .map(|(rust, _)| rust.clone())
            };
            let variant = variant.ok_or_else(|| {
                error::Error::Schema(format!(
                    "no enum variant for `{json}` in {}",
                    m.types[*i].fqn
                ))
            })?;
            let _ = write!(out, "{}::{variant}", path(m, module, *i));
            Ok(())
        }
        Ty::Array(i) => {
            let a = m.as_array(*i);
            let mut body = String::new();
            array_literal(m, module, &a.elem, json, &mut body)?;
            if a.newtype {
                let _ = write!(out, "{}({body})", path(m, module, *i));
            } else {
                // A `Vec` alias fixes the element type of an empty list.
                let _ = write!(out, "{}::from({body})", path(m, module, *i));
            }
            Ok(())
        }
        Ty::Record(i) => {
            let r = m.as_record(*i);
            let pairs = json
                .as_array()
                .ok_or_else(|| error::Error::Schema("record value is not a list".into()))?;
            let mut body = String::from("vec![");
            for pair in pairs {
                let pair = pair
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .ok_or_else(|| error::Error::Schema("record entry is not a pair".into()))?;
                body.push('(');
                value_literal(m, module, &r.key, &pair[0], &mut body)?;
                body.push_str(", ");
                value_literal(m, module, &r.value, &pair[1], &mut body)?;
                body.push_str("), ");
            }
            body.push(']');
            let _ = write!(out, "{}({body})", path(m, module, *i));
            Ok(())
        }
        Ty::Versioned(i) => {
            let v = m.as_versioned(*i);
            let version = json["version"].as_u64().unwrap_or(0);
            let def = v
                .defs
                .iter()
                .find(|d| d.version >= version)
                .ok_or_else(|| error::Error::Schema("version outside the schema".into()))?;
            let _ = write!(out, "{}::V{}(", path(m, module, *i), def.version);
            value_literal(m, module, &def.ty, json, out)?;
            out.push(')');
            Ok(())
        }
        Ty::External(i) => Err(error::Error::Schema(format!(
            "`{}` is external; its value cannot be built from a fixture",
            m.types[*i].fqn
        ))),
    }
}

fn struct_literal(
    m: &Model,
    module: &str,
    idx: usize,
    json: &Value,
    out: &mut String,
) -> Result<()> {
    let s = m.as_struct(idx);
    let object = json.as_object().ok_or_else(|| {
        error::Error::Schema(format!(
            "{} value is not an object: {}",
            m.types[idx].fqn, json
        ))
    })?;
    let _ = write!(out, "{} {{ ", path(m, module, idx));
    for f in &s.fields {
        let _ = write!(out, "{}: ", f.rust_name);
        if let Some(constant) = &f.constant {
            out.push_str(&constant_literal(constant));
            out.push_str(", ");
            continue;
        }
        let value = object.get(&f.name).unwrap_or(&Value::Null);
        if f.is_flag_field() {
            value_literal(m, module, &f.ty, value, out)?;
            out.push_str(", ");
            continue;
        }
        if f.required {
            if f.boxed {
                out.push_str("Box::new(");
            }
            field_value(m, module, f, value, out)?;
            if f.boxed {
                out.push(')');
            }
            out.push_str(", ");
            continue;
        }
        if is_absent(&f.ty, value) {
            out.push_str("None, ");
        } else if m.option_flavored(&f.ty) && !f.array {
            field_value(m, module, f, value, out)?;
            out.push_str(", ");
        } else {
            out.push_str(if f.boxed { "Some(Box::new(" } else { "Some(" });
            field_value(m, module, f, value, out)?;
            out.push_str(if f.boxed { ")), " } else { "), " });
        }
    }
    out.push('}');
    Ok(())
}

/// JS decodes a missing optional number as `0` and a missing optional enum as
/// `0`, which the Rust side models as `None`.
fn is_absent(ty: &Ty, value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Number(n) if n.as_f64() == Some(0.0) => match ty {
            Ty::Prim(Prim::Bool) => true,
            Ty::Prim(p) => !matches!(p, Prim::String | Prim::Buffer | Prim::OptionalBuffer),
            Ty::Enum(_) => true,
            _ => false,
        },
        _ => false,
    }
}

/// Array fields hold a list of the field's type; the element type is `f.ty`.
fn field_value(
    m: &Model,
    module: &str,
    f: &model::Field,
    json: &Value,
    out: &mut String,
) -> Result<()> {
    if f.array {
        array_literal(m, module, &f.ty, json, out)
    } else {
        value_literal(m, module, &f.ty, json, out)
    }
}

fn array_literal(m: &Model, module: &str, elem: &Ty, json: &Value, out: &mut String) -> Result<()> {
    let items = json
        .as_array()
        .ok_or_else(|| error::Error::Schema(format!("array value is not a list: {json}")))?;
    out.push_str("vec![");
    for item in items {
        value_literal(m, module, elem, item, out)?;
        out.push_str(", ");
    }
    out.push(']');
    Ok(())
}

fn prim_literal(p: Prim, json: &Value, out: &mut String) -> Result<()> {
    match p {
        Prim::Bool => out.push_str(if json.as_bool() == Some(true) {
            "true"
        } else {
            "false"
        }),
        Prim::String => {
            let s = json.as_str().unwrap_or_default();
            let _ = write!(out, "{s:?}.to_string()");
        }
        Prim::Buffer | Prim::Raw => {
            let bytes = decode_hex(json.as_str().unwrap_or_default())?;
            let _ = write!(out, "Bytes::from_static({})", byte_slice(&bytes));
        }
        Prim::OptionalBuffer => match json.as_str() {
            Some(s) if !s.is_empty() => {
                let bytes = decode_hex(s)?;
                let _ = write!(out, "Some(Bytes::from_static({}))", byte_slice(&bytes));
            }
            _ => out.push_str("None"),
        },
        Prim::Fixed(n) => {
            let bytes = decode_hex(json.as_str().unwrap_or_default())?;
            if bytes.len() != n {
                return Err(error::Error::Schema(format!(
                    "fixed{n} fixture has {} bytes",
                    bytes.len()
                )));
            }
            out.push_str(&byte_array(&bytes));
        }
        Prim::Float32 | Prim::Float64 => {
            let f = json.as_f64().unwrap_or_default();
            let _ = write!(out, "{f:?}");
        }
        Prim::None => out.push_str("()"),
        Prim::Ip => {
            let ip: IpAddr = json
                .as_str()
                .unwrap_or_default()
                .parse()
                .map_err(|_| error::Error::Schema("bad ip".into()))?;
            match ip {
                IpAddr::V4(a) => {
                    let _ = write!(
                        out,
                        "IpAddr::V4(Ipv4Addr::new({}, {}, {}, {}))",
                        a.octets()[0],
                        a.octets()[1],
                        a.octets()[2],
                        a.octets()[3]
                    );
                }
                IpAddr::V6(a) => {
                    let _ = write!(out, "IpAddr::V6({})", ipv6_literal(&a));
                }
            }
        }
        Prim::Ipv4 | Prim::Ipv4Address => {
            let host = json["host"].as_str().or(json.as_str()).unwrap_or_default();
            let ip: Ipv4Addr = host
                .parse()
                .map_err(|_| error::Error::Schema("bad ipv4 host".into()))?;
            let o = ip.octets();
            let inner = format!("Ipv4Addr::new({}, {}, {}, {})", o[0], o[1], o[2], o[3]);
            if matches!(p, Prim::Ipv4) {
                out.push_str(&inner);
            } else {
                let port = json["port"].as_u64().unwrap_or_default();
                let _ = write!(out, "SocketAddrV4::new({inner}, {port})");
            }
        }
        Prim::Ipv6 | Prim::Ipv6Address => {
            let host = json["host"].as_str().or(json.as_str()).unwrap_or_default();
            let ip: Ipv6Addr = host
                .parse()
                .map_err(|_| error::Error::Schema("bad ipv6 host".into()))?;
            let inner = ipv6_literal(&ip);
            if matches!(p, Prim::Ipv6) {
                out.push_str(&inner);
            } else {
                let port = json["port"].as_u64().unwrap_or_default();
                let _ = write!(out, "SocketAddrV6::new({inner}, {port}, 0, 0)");
            }
        }
        Prim::IpAddress => {
            let ip: IpAddr = json["host"]
                .as_str()
                .unwrap_or_default()
                .parse()
                .map_err(|_| error::Error::Schema("bad ip".into()))?;
            let port = json["port"].as_u64().unwrap_or_default();
            let inner = match ip {
                IpAddr::V4(a) => {
                    let o = a.octets();
                    format!(
                        "SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new({}, {}, {}, {}), {port}))",
                        o[0], o[1], o[2], o[3]
                    )
                }
                IpAddr::V6(a) => format!(
                    "SocketAddr::V6(SocketAddrV6::new({}, {port}, 0, 0))",
                    ipv6_literal(&a)
                ),
            };
            out.push_str(&inner);
        }
        _ => {
            let n = json.as_i64().unwrap_or_default();
            let _ = write!(out, "{n}");
        }
    }
    Ok(())
}

fn ipv6_literal(ip: &Ipv6Addr) -> String {
    let s = &ip.segments();
    format!(
        "Ipv6Addr::new({}, {}, {}, {}, {}, {}, {}, {})",
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]
    )
}

fn constant_literal(value: &Value) -> String {
    if value.is_null() {
        return "None".to_string();
    }
    if let Some(b) = value.as_bool() {
        return b.to_string();
    }
    if let Some(i) = value.as_i64() {
        return i.to_string();
    }
    format!("{:?}.to_string()", value.as_str().unwrap_or_default())
}

fn byte_array(bytes: &[u8]) -> String {
    let mut out = String::from("[");
    for (i, b) in bytes.iter().enumerate() {
        if i % 16 == 0 && i != 0 {
            out.push_str("\n            ");
        }
        let _ = write!(out, "{b}, ");
    }
    out.push(']');
    out
}

fn byte_slice(bytes: &[u8]) -> String {
    let mut out = String::from("&[");
    for (i, b) in bytes.iter().enumerate() {
        if i % 16 == 0 && i != 0 {
            out.push_str("\n            ");
        }
        let _ = write!(out, "{b}, ");
    }
    out.push(']');
    out
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(error::Error::Schema(format!("odd hex length: {s}")));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks(2) {
        let text =
            std::str::from_utf8(pair).map_err(|_| error::Error::Schema(format!("bad hex: {s}")))?;
        out.push(
            u8::from_str_radix(text, 16)
                .map_err(|_| error::Error::Schema(format!("bad hex: {s}")))?,
        );
    }
    Ok(out)
}
