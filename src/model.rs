//! Parses `schema.json` and resolves it into the model the emitter consumes:
//! names, field types, struct flag layouts and versioned type dispatch.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::error::{Error, Result};
use super::naming::{camel_case, module_name, snake_case, type_name};

/// The JS codec keeps flags in a safe integer; more bits than this cannot be
/// expressed without its multiplication fallbacks.
pub const MAX_FLAG_BITS: u32 = 64;

/// Primitives of `hyperschema`'s `types.js` that this generator implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Uint,
    Bitwise(u32),
    Uint8,
    Uint16,
    Uint24,
    Uint32,
    Uint40,
    Uint48,
    Uint56,
    Uint64,
    Int,
    Int8,
    Int16,
    Int24,
    Int32,
    Int40,
    Int48,
    Int56,
    Int64,
    Float32,
    Float64,
    Port,
    String,
    BigUint64,
    BigInt64,
    Bool,
    Buffer,
    OptionalBuffer,
    Fixed(usize),
    Raw,
    None,
    Ip,
    Ipv4,
    Ipv6,
    IpAddress,
    Ipv4Address,
    Ipv6Address,
}

impl Prim {
    /// `Some(Ok(..))` is supported, `Some(Err(..))` is a known primitive that
    /// generated code cannot depend on, `None` is not a primitive at all.
    pub fn from_name(name: &str) -> Option<std::result::Result<Prim, &'static str>> {
        let prim = match name {
            "uint" => Prim::Uint,
            "uint1" => Prim::Bitwise(1),
            "uint2" => Prim::Bitwise(2),
            "uint3" => Prim::Bitwise(3),
            "uint4" => Prim::Bitwise(4),
            "uint5" => Prim::Bitwise(5),
            "uint6" => Prim::Bitwise(6),
            "uint7" => Prim::Bitwise(7),
            "uint8" => Prim::Uint8,
            "uint16" => Prim::Uint16,
            "uint24" => Prim::Uint24,
            "uint32" => Prim::Uint32,
            "uint40" => Prim::Uint40,
            "uint48" => Prim::Uint48,
            "uint56" => Prim::Uint56,
            "uint64" => Prim::Uint64,
            "int" => Prim::Int,
            "int8" => Prim::Int8,
            "int16" => Prim::Int16,
            "int24" => Prim::Int24,
            "int32" => Prim::Int32,
            "int40" => Prim::Int40,
            "int48" => Prim::Int48,
            "int56" => Prim::Int56,
            "int64" => Prim::Int64,
            "float32" => Prim::Float32,
            "float64" => Prim::Float64,
            "port" => Prim::Port,
            "string" | "utf8" => Prim::String,
            "biguint64" => Prim::BigUint64,
            "bigint64" => Prim::BigInt64,
            "bool" => Prim::Bool,
            "buffer" => Prim::Buffer,
            "optionalBuffer" => Prim::OptionalBuffer,
            "fixed8" => Prim::Fixed(8),
            "fixed16" => Prim::Fixed(16),
            "fixed24" => Prim::Fixed(24),
            "fixed32" => Prim::Fixed(32),
            "fixed64" => Prim::Fixed(64),
            "raw" => Prim::Raw,
            "none" => Prim::None,
            "ip" => Prim::Ip,
            "ipv4" => Prim::Ipv4,
            "ipv6" => Prim::Ipv6,
            "ipAddress" => Prim::IpAddress,
            "ipv4Address" => Prim::Ipv4Address,
            "ipv6Address" => Prim::Ipv6Address,
            "lexint" => return Some(Err("the `lexint` primitive is not ported")),
            "bigint" => return Some(Err("the unbounded `bigint` primitive is not ported")),
            "json" => return Some(Err("the `json` primitive is not ported")),
            "date" => return Some(Err("the `date` primitive is not ported")),
            "ascii" => return Some(Err("the `ascii` string primitive is not ported")),
            "hex" => return Some(Err("the `hex` string primitive is not ported")),
            _ => return None,
        };
        Some(Ok(prim))
    }
}

/// A type reference after aliases are resolved away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Prim(Prim),
    Struct(usize),
    Enum(usize),
    Array(usize),
    Record(usize),
    Versioned(usize),
    External(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Struct,
    Enum,
    Array,
    Record,
    Alias,
    Versioned,
    External,
}

impl Tag {
    fn ty(self, index: usize) -> Ty {
        match self {
            Tag::Struct => Ty::Struct(index),
            Tag::Enum => Ty::Enum(index),
            Tag::Array => Ty::Array(index),
            Tag::Record => Ty::Record(index),
            Tag::Alias => unreachable!("aliases are erased"),
            Tag::Versioned => Ty::Versioned(index),
            Tag::External => Ty::External(index),
        }
    }
}

pub struct Model {
    pub version: u64,
    pub namespaces: Vec<Namespace>,
    pub types: Vec<Type>,
}

pub struct Namespace {
    pub name: String,
    pub module: String,
}

pub struct Type {
    pub fqn: String,
    pub rust_name: String,
    pub ns: usize,
    pub kind: Kind,
}

pub enum Kind {
    Struct(Struct),
    Enum(EnumType),
    Array(ArrayType),
    Record(RecordType),
    Alias(AliasType),
    Versioned(VersionedType),
    External(ExternalType),
}

pub struct Struct {
    pub compact: bool,
    /// Index of the field whose position carries the flags word, `-1` for none.
    pub flags_position: i64,
    pub fields: Vec<Field>,
    /// Total bits of the flags word.
    pub flags_bits: u32,
    /// Set when some struct embeds this one inline; inline helpers are emitted.
    pub inlined: bool,
    /// Flags can only be elided when no preceding field was added in a later
    /// version, mirroring codegen.js `alwaysOptional`.
    pub always_optional: bool,
}

#[derive(Clone)]
pub struct Field {
    pub name: String,
    pub rust_name: String,
    pub ty: Ty,
    /// Set when the JS encoder folds this scalar into the flags word.
    pub flag_kind: FlagKind,
    pub array: bool,
    pub inline: bool,
    pub required: bool,
    pub constant: Option<serde_json::Value>,
    pub version: u64,
    /// Bit index of this field's own presence bit or bitwise value.
    pub flag_bit: u32,
    /// Bit offset of an inline child's own flags word.
    pub child_offset: u32,
    pub child: Option<usize>,
    /// A field that closes a type cycle has to be boxed to have a size.
    pub boxed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagKind {
    None,
    Bool,
    Bitwise(u32),
}

impl Field {
    /// Bools and bitwise uints live in the flags word, never on the wire.
    pub fn is_flag_field(&self) -> bool {
        !self.array && !matches!(self.flag_kind, FlagKind::None)
    }
}

pub struct EnumType {
    pub offset: u64,
    /// `(rust variant name, schema key)`, in schema order.
    pub variants: Vec<(String, String)>,
}

pub struct ArrayType {
    pub elem: Ty,
    /// True when a `Vec` alias cannot carry the codec (bool arrays, framing).
    pub newtype: bool,
}

pub struct RecordType {
    pub key: Ty,
    pub value: Ty,
}

pub struct AliasType {
    pub target: Ty,
}

pub struct VersionedType {
    pub framed: bool,
    /// Ascending, distinct versions; the first covers version 0.
    pub defs: Vec<VersionDef>,
}

pub struct VersionDef {
    pub version: u64,
    pub ty: Ty,
    pub map: Option<String>,
}

pub struct ExternalType {
    pub external: String,
}

// ---------------------------------------------------------------------------
// schema.json

#[derive(Deserialize)]
struct RawSchema {
    #[serde(default)]
    version: u64,
    schema: Vec<RawType>,
    #[serde(default)]
    namespaces: Vec<RawNamespace>,
}

#[derive(Deserialize)]
struct RawNamespace {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    #[serde(default)]
    external: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawType {
    name: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    alias: Option<String>,
    #[serde(rename = "enum", default)]
    variants: Option<Vec<RawVariant>>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    array: Option<bool>,
    #[serde(default)]
    record: Option<bool>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    external: Option<String>,
    #[serde(default)]
    versions: Option<Vec<RawVersion>>,
    #[serde(default)]
    framed: Option<bool>,
    #[serde(default)]
    compact: Option<bool>,
    #[serde(rename = "flagsPosition", default)]
    flags_position: Option<i64>,
    #[serde(default)]
    fields: Option<Vec<RawField>>,
    #[serde(rename = "type", default)]
    ty: Option<String>,
    #[serde(default)]
    derived: bool,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum RawVariant {
    Key(String),
    Full {
        key: String,
        #[serde(default)]
        #[allow(dead_code)]
        version: u64,
    },
}

impl RawVariant {
    fn key(&self) -> &str {
        match self {
            RawVariant::Key(k) => k,
            RawVariant::Full { key, .. } => key,
        }
    }
}

#[derive(Deserialize, Clone)]
struct RawVersion {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    map: Option<String>,
    version: u64,
}

#[derive(Deserialize, Clone, Default)]
struct RawField {
    name: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    array: bool,
    #[serde(default)]
    record: bool,
    #[serde(default)]
    inline: bool,
    #[serde(default)]
    external: bool,
    #[serde(default)]
    constant: Option<serde_json::Value>,
    #[serde(rename = "type", default)]
    ty: String,
    #[serde(default = "default_version")]
    version: u64,
}

fn default_version() -> u64 {
    1
}

// ---------------------------------------------------------------------------
// build

pub fn parse(json: &str) -> Result<Model> {
    let raw: RawSchema = serde_json::from_str(json)?;
    let mut namespaces: Vec<Namespace> = Vec::new();

    for ns in &raw.namespaces {
        push_namespace(&mut namespaces, &ns.name)?;
    }
    for t in &raw.schema {
        push_namespace(&mut namespaces, t.namespace.as_deref().unwrap_or(""))?;
    }

    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut fqns: Vec<String> = Vec::with_capacity(raw.schema.len());
    let mut tags: Vec<Tag> = Vec::with_capacity(raw.schema.len());

    for t in &raw.schema {
        if t.derived {
            return Err(Error::Unsupported(format!(
                "derived type `{}` present in the schema file",
                t.name
            )));
        }
        let ns_name = t.namespace.as_deref().unwrap_or("");
        let fqn = fqn_of(ns_name, &t.name);
        if index.insert(fqn.clone(), fqns.len()).is_some() {
            return Err(Error::Schema(format!("duplicate type `{fqn}`")));
        }
        fqns.push(fqn);
        tags.push(tag_of(t));
    }

    let resolve = |name: &str| -> Result<Ty> { resolve_name(name, &raw, &index, &tags, 0) };

    let mut types: Vec<Type> = Vec::with_capacity(raw.schema.len());
    for (i, t) in raw.schema.iter().enumerate() {
        let ns = namespaces
            .iter()
            .position(|n| n.name == t.namespace.as_deref().unwrap_or(""))
            .ok_or_else(|| Error::Schema(format!("unknown namespace for `{}`", t.name)))?;
        let kind = build_kind(t, &fqns[i], &raw, &resolve)?;
        types.push(Type {
            fqn: fqns[i].clone(),
            rust_name: type_name(&t.name),
            ns,
            kind,
        });
    }

    let mut seen: BTreeMap<(usize, String), String> = BTreeMap::new();
    for t in &types {
        let key = (t.ns, t.rust_name.clone());
        if let Some(prev) = seen.insert(key, t.fqn.clone()) {
            return Err(Error::Schema(format!(
                "`{prev}` and `{}` both map to the Rust name `{}`",
                t.fqn, t.rust_name
            )));
        }
    }

    let mut model = Model {
        version: raw.version,
        namespaces,
        types,
    };

    validate_bitwise(&model)?;
    layout_all(&mut model)?;
    mark_boxed(&mut model);

    let mut inlined = Vec::new();
    for t in &model.types {
        if let Kind::Struct(s) = &t.kind {
            for f in &s.fields {
                if let Some(child) = f.child {
                    inlined.push(child);
                }
            }
        }
    }
    for i in inlined {
        if let Some(Kind::Struct(s)) = model.types.get_mut(i).map(|t| &mut t.kind) {
            s.inlined = true;
        }
    }

    Ok(model)
}

fn tag_of(t: &RawType) -> Tag {
    if t.alias.is_some() {
        Tag::Alias
    } else if t.variants.is_some() {
        Tag::Enum
    } else if t.array.unwrap_or(false) {
        Tag::Array
    } else if t.record.unwrap_or(false) {
        Tag::Record
    } else if t.external.is_some() {
        Tag::External
    } else if t.versions.is_some() {
        Tag::Versioned
    } else {
        Tag::Struct
    }
}

fn resolve_name(
    name: &str,
    raw: &RawSchema,
    index: &BTreeMap<String, usize>,
    tags: &[Tag],
    depth: u32,
) -> Result<Ty> {
    if let Some(prim) = Prim::from_name(name) {
        return match prim {
            Ok(p) => Ok(Ty::Prim(p)),
            Err(reason) => Err(Error::Unsupported(format!("`{name}`: {reason}"))),
        };
    }

    let i = index
        .get(name)
        .copied()
        .ok_or_else(|| Error::Schema(format!("cannot resolve type `{name}`")))?;

    if tags[i] == Tag::Alias {
        if depth > 32 {
            return Err(Error::Schema(format!("alias cycle at `{name}`")));
        }
        let target = raw.schema[i]
            .alias
            .as_deref()
            .ok_or_else(|| Error::Schema(format!("`{name}` is not a type")))?;
        return resolve_name(target, raw, index, tags, depth + 1);
    }

    Ok(tags[i].ty(i))
}

fn build_kind(
    t: &RawType,
    fqn: &str,
    raw: &RawSchema,
    resolve: &impl Fn(&str) -> Result<Ty>,
) -> Result<Kind> {
    if let Some(alias) = &t.alias {
        return Ok(Kind::Alias(AliasType {
            target: resolve(alias)?,
        }));
    }

    if let Some(variants) = &t.variants {
        let offset = t.offset.unwrap_or(1);
        if offset > u64::from(u32::MAX) {
            return Err(Error::Unsupported(format!(
                "`{fqn}`: enum offset {offset} is out of range"
            )));
        }
        if variants.is_empty() {
            return Err(Error::Unsupported(format!("`{fqn}`: empty enum")));
        }
        let mut out: Vec<(String, String)> = Vec::with_capacity(variants.len());
        for v in variants {
            out.push((camel_case(v.key()), v.key().to_string()));
        }
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (rust, key) in &out {
            if let Some(prev) = seen.insert(rust.clone(), key.clone()) {
                return Err(Error::Schema(format!(
                    "`{fqn}`: enum keys `{prev}` and `{key}` both map to `{rust}`"
                )));
            }
        }
        return Ok(Kind::Enum(EnumType {
            offset,
            variants: out,
        }));
    }

    if t.array.unwrap_or(false) {
        let elem_name =
            t.ty.as_deref()
                .ok_or_else(|| Error::Schema(format!("`{fqn}`: array without a type")))?;
        let elem = resolve(elem_name)?;
        if let Ty::Prim(Prim::Bitwise(_)) = elem {
            return Err(Error::Unsupported(format!(
                "`{fqn}`: a bitwise uint cannot be used as an array"
            )));
        }
        let newtype = frameable(&elem, raw) || matches!(elem, Ty::Prim(Prim::Bool));
        return Ok(Kind::Array(ArrayType { elem, newtype }));
    }

    if t.record.unwrap_or(false) {
        let key = t
            .key
            .as_deref()
            .ok_or_else(|| Error::Schema(format!("`{fqn}`: record without a key")))?;
        let value = t
            .value
            .as_deref()
            .ok_or_else(|| Error::Schema(format!("`{fqn}`: record without a value")))?;
        return Ok(Kind::Record(RecordType {
            key: resolve(key)?,
            value: resolve(value)?,
        }));
    }

    if let Some(external) = &t.external {
        return Ok(Kind::External(ExternalType {
            external: external.clone(),
        }));
    }

    if let Some(versions) = &t.versions {
        if versions.is_empty() {
            return Err(Error::Unsupported(format!("`{fqn}`: no versions")));
        }
        let mut defs: Vec<VersionDef> = Vec::with_capacity(versions.len());
        let mut prev: Option<u64> = None;
        for v in versions {
            if prev.is_some_and(|p| v.version <= p) {
                return Err(Error::Schema(format!(
                    "`{fqn}`: versions must be declared in ascending order"
                )));
            }
            prev = Some(v.version);
            defs.push(VersionDef {
                version: v.version,
                ty: resolve(&v.ty)?,
                map: v.map.clone(),
            });
        }
        return Ok(Kind::Versioned(VersionedType {
            framed: t.framed.unwrap_or(false),
            defs,
        }));
    }

    let mut fields: Vec<Field> = Vec::new();
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    for f in t.fields.as_deref().unwrap_or(&[]) {
        let tag = format!("{fqn}/{}", f.name);
        if f.record {
            return Err(Error::Unsupported(format!(
                "`{tag}`: record fields are rejected by hyperschema itself"
            )));
        }
        if f.external {
            return Err(Error::Unsupported(format!(
                "`{tag}`: external placeholder fields"
            )));
        }
        let ty = resolve(&f.ty)?;
        let mut required = f.required;
        let flag_kind = if f.array {
            if let Ty::Prim(Prim::Bitwise(_)) = ty {
                return Err(Error::Unsupported(format!(
                    "`{tag}`: a bitwise uint cannot be used as an array"
                )));
            }
            FlagKind::None
        } else {
            match Prim::from_name(&f.ty) {
                Some(Ok(Prim::Bool)) => {
                    // codegen.js downgrades bools to optional so they fit the flags
                    required = false;
                    FlagKind::Bool
                }
                Some(Ok(Prim::Bitwise(n))) => {
                    required = false;
                    FlagKind::Bitwise(n)
                }
                _ => FlagKind::None,
            }
        };

        let rust_name = snake_case(&f.name);
        if let Some(prev) = taken.insert(rust_name.clone(), f.name.clone()) {
            return Err(Error::Schema(format!(
                "`{fqn}`: fields `{prev}` and `{}` both map to `{rust_name}`",
                f.name
            )));
        }

        let constant = match &f.constant {
            None => None,
            Some(c) if c.is_null() || c.is_boolean() || c.is_string() => Some(c.clone()),
            Some(c) if c.as_i64().is_some() => Some(c.clone()),
            Some(_) => {
                return Err(Error::Schema(format!(
                    "`{tag}`: a constant must be a bool, integer, string or null"
                )));
            }
        };

        let inline = f.inline;
        let child = if inline {
            match ty {
                Ty::Struct(i) => {
                    if !raw.schema[i].compact.unwrap_or(false) {
                        return Err(Error::Schema(format!(
                            "`{tag}`: inline requires a compact struct"
                        )));
                    }
                    if f.array {
                        return Err(Error::Schema(format!(
                            "`{tag}`: an array field cannot be inline"
                        )));
                    }
                    Some(i)
                }
                _ => {
                    return Err(Error::Schema(format!(
                        "`{tag}`: inline requires a struct type"
                    )));
                }
            }
        } else {
            None
        };

        fields.push(Field {
            name: f.name.clone(),
            rust_name,
            ty,
            flag_kind,
            array: f.array,
            inline,
            required,
            constant,
            version: f.version,
            flag_bit: 0,
            child_offset: 0,
            child,
            boxed: false,
        });
    }

    Ok(Kind::Struct(Struct {
        compact: t.compact.unwrap_or(false),
        flags_position: t.flags_position.unwrap_or(-1),
        fields,
        flags_bits: 0,
        inlined: false,
        always_optional: false,
    }))
}

/// Mirrors `codegen.js` `frameable()`: only non-compact structs can be framed.
fn frameable(ty: &Ty, raw: &RawSchema) -> bool {
    match ty {
        Ty::Struct(i) => !raw.schema[*i].compact.unwrap_or(false),
        _ => false,
    }
}

fn validate_bitwise(model: &Model) -> Result<()> {
    for t in &model.types {
        match &t.kind {
            Kind::Struct(s) => {
                for f in &s.fields {
                    if matches!(f.ty, Ty::Prim(Prim::Bitwise(_))) && !f.is_flag_field() {
                        return Err(Error::Unsupported(format!(
                            "`{}/{}`: a bitwise uint only exists inside a struct's flags",
                            t.fqn, f.name
                        )));
                    }
                }
            }
            Kind::Array(a) => reject_bitwise(&t.fqn, &a.elem)?,
            Kind::Record(r) => {
                reject_bitwise(&t.fqn, &r.key)?;
                reject_bitwise(&t.fqn, &r.value)?;
            }
            Kind::Alias(a) => reject_bitwise(&t.fqn, &a.target)?,
            Kind::Versioned(v) => {
                for d in &v.defs {
                    reject_bitwise(&t.fqn, &d.ty)?;
                }
            }
            Kind::Enum(_) | Kind::External(_) => {}
        }
    }
    Ok(())
}

fn reject_bitwise(fqn: &str, ty: &Ty) -> Result<()> {
    if matches!(ty, Ty::Prim(Prim::Bitwise(_))) {
        return Err(Error::Unsupported(format!(
            "`{fqn}`: a bitwise uint only exists inside a struct's flags"
        )));
    }
    Ok(())
}

fn layout_all(model: &mut Model) -> Result<()> {
    let mut done: BTreeMap<usize, u32> = BTreeMap::new();
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..model.types.len() {
        layout_struct(model, i, &mut stack, &mut done)?;
    }
    Ok(())
}

fn layout_struct(
    model: &mut Model,
    idx: usize,
    stack: &mut Vec<usize>,
    done: &mut BTreeMap<usize, u32>,
) -> Result<u32> {
    if let Some(bits) = done.get(&idx) {
        return Ok(*bits);
    }
    let (fqn, mut fields, mut flags_position) = match &model.types[idx].kind {
        Kind::Struct(s) => (
            model.types[idx].fqn.clone(),
            s.fields.clone(),
            s.flags_position,
        ),
        _ => {
            done.insert(idx, 0);
            return Ok(0);
        }
    };

    if stack.contains(&idx) {
        return Err(Error::Schema(format!("`{fqn}`: inline cycle")));
    }
    stack.push(idx);

    let first_optional = fields
        .iter()
        .position(|f| f.inline || (!f.required && f.constant.is_none()));

    match (first_optional, flags_position) {
        (None, p) if p >= 0 => {
            return Err(Error::Schema(format!(
                "`{fqn}`: flagsPosition {p} but no field is optional"
            )));
        }
        (Some(first), p) if p < 0 => flags_position = first as i64,
        (Some(first), p) if p as usize > first => {
            return Err(Error::Schema(format!(
                "`{fqn}`: flagsPosition {p} is after the first optional field ({first})"
            )));
        }
        _ => {}
    }

    if flags_position >= 0 {
        for (i, f) in fields.iter().take(flags_position as usize).enumerate() {
            let readable = f.constant.is_some() || (f.required && !f.is_flag_field() && !f.inline);
            if !readable {
                return Err(Error::Schema(format!(
                    "`{fqn}`: field {i} sits before flagsPosition {flags_position} but needs the flags"
                )));
            }
        }
    }

    let mut bits = 0u32;
    for f in &mut fields {
        if f.constant.is_some() {
            continue;
        }
        if f.is_flag_field() {
            f.flag_bit = bits;
            bits += match f.flag_kind {
                FlagKind::Bitwise(n) => n,
                _ => 1,
            };
        } else if !f.required {
            f.flag_bit = bits;
            bits += 1;
        }
        if f.inline {
            let child = f
                .child
                .ok_or_else(|| Error::Schema(format!("`{fqn}`: inline field without a target")))?;
            f.child_offset = bits;
            let child_bits = layout_struct(model, child, stack, done)?;
            bits += child_bits;
        }
    }

    if bits == 0 && flags_position >= 0 {
        return Err(Error::Schema(format!(
            "`{fqn}`: flagsPosition {flags_position} but the flags word has no bits"
        )));
    }
    if bits > MAX_FLAG_BITS {
        return Err(Error::Unsupported(format!(
            "`{fqn}`: {bits} flag bits; this generator supports {MAX_FLAG_BITS}"
        )));
    }

    let always_optional = fields
        .iter()
        .position(|f| !f.required && f.constant.is_none())
        .is_some_and(|i| {
            let version = fields[i].version;
            fields[..i].iter().all(|p| p.version == version)
        });

    if let Kind::Struct(s) = &mut model.types[idx].kind {
        s.fields = fields;
        s.flags_position = flags_position;
        s.flags_bits = bits;
        s.always_optional = always_optional;
    }
    stack.pop();
    done.insert(idx, bits);
    Ok(bits)
}

/// Fields that take part in a type cycle are boxed; arrays and records already
/// break cycles because their payload lives behind a `Vec`.
fn mark_boxed(model: &mut Model) {
    let n = model.types.len();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, t) in model.types.iter().enumerate() {
        match &t.kind {
            Kind::Struct(s) => {
                for f in &s.fields {
                    if f.array {
                        continue;
                    }
                    if let Ty::Struct(j) | Ty::Versioned(j) = f.ty {
                        edges[i].push(j);
                    }
                }
            }
            Kind::Versioned(v) => {
                for d in &v.defs {
                    if let Ty::Struct(j) = d.ty {
                        edges[i].push(j);
                    }
                }
            }
            _ => {}
        }
    }

    let mut reach = vec![vec![false; n]; n];
    for (start, reachable) in reach.iter_mut().enumerate() {
        let mut stack = edges[start].clone();
        while let Some(t) = stack.pop() {
            if reachable[t] {
                continue;
            }
            reachable[t] = true;
            stack.extend(edges[t].iter().copied());
        }
    }

    for (i, ty) in model.types.iter_mut().enumerate() {
        let Kind::Struct(s) = &mut ty.kind else {
            continue;
        };
        for f in &mut s.fields {
            if f.array {
                continue;
            }
            if let Ty::Struct(j) | Ty::Versioned(j) = f.ty {
                f.boxed = reach[j][i];
            }
        }
    }
}

fn push_namespace(namespaces: &mut Vec<Namespace>, name: &str) -> Result<()> {
    if namespaces.iter().any(|n| n.name == name) {
        return Ok(());
    }
    let module = module_name(name);
    if let Some(other) = namespaces.iter().find(|n| n.module == module) {
        return Err(Error::Schema(format!(
            "namespaces `{}` and `{name}` both map to module `{module}`",
            other.name
        )));
    }
    namespaces.push(Namespace {
        name: name.to_string(),
        module,
    });
    Ok(())
}

fn fqn_of(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("@{namespace}/{name}")
    }
}

// ---------------------------------------------------------------------------
// lookups used by the emitter

impl Model {
    pub fn as_struct(&self, index: usize) -> &Struct {
        match &self.types[index].kind {
            Kind::Struct(s) => s,
            _ => unreachable!("{} is not a struct", self.types[index].fqn),
        }
    }

    pub fn as_enum(&self, index: usize) -> &EnumType {
        match &self.types[index].kind {
            Kind::Enum(e) => e,
            _ => unreachable!("{} is not an enum", self.types[index].fqn),
        }
    }

    pub fn as_array(&self, index: usize) -> &ArrayType {
        match &self.types[index].kind {
            Kind::Array(a) => a,
            _ => unreachable!("{} is not an array", self.types[index].fqn),
        }
    }

    pub fn as_record(&self, index: usize) -> &RecordType {
        match &self.types[index].kind {
            Kind::Record(r) => r,
            _ => unreachable!("{} is not a record", self.types[index].fqn),
        }
    }

    pub fn as_versioned(&self, index: usize) -> &VersionedType {
        match &self.types[index].kind {
            Kind::Versioned(v) => v,
            _ => unreachable!("{} is not versioned", self.types[index].fqn),
        }
    }

    /// A field of this type is wrapped in `c.frame(..)` by codegen.
    pub fn frameable(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Struct(i) => !self.as_struct(*i).compact,
            _ => false,
        }
    }

    /// True when the type's Rust value already is an `Option` on the wire
    /// (`c.optionalBuffer`).
    pub fn option_flavored(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Prim(Prim::OptionalBuffer))
    }
}
