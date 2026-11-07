# hyperschema-rs

Rust code generator for [hyperschema](https://github.com/holepunchto/hyperschema)
`schema.json` files, byte-compatible with the JS generated `index.js` (hyperschema
1.26.2), built on [`compact-encoding-rs`](../compact-encoding-rs).

```rust
// build.rs
fn main() {
    let opts = hyperschema::Options::new();
    hyperschema::build("spec/hyperschema/schema.json", "hyperschema.rs", &opts).unwrap();
}
```

```rust
mod encoding {
    include!(concat!(env!("OUT_DIR"), "/hyperschema.rs"));
}

let value = encoding::bee::TreeDelta { type_: 3, index: Some(7), pointer: None };
let bytes = compact_encoding::encode(&value);      // the schema's latest version
let mut w = compact_encoding::Writer::new();
value.encode_version(1, &mut w);                    // pinned to schema version 1
```

The file has to be included inside a module: types in other schema namespaces are
referenced as `super::other_namespace::Type`.

`generate(schema_json, &opts)` returns the same source as a `String`, for build
scripts that already read the schema.

## Generated shape

- one `pub mod <namespace> { .. }` per schema namespace (`autobase-compat` →
  `autobase_compat`), plus `pub const VERSION: u64` for the schema version;
- struct names are CamelCase (`tree-delta` → `TreeDelta`), fields snake_case
  (`defaultDiscoveryKey` → `default_discovery_key`), Rust keywords get a trailing
  `_` (`type` → `type_`); a type whose name collides with a prelude item
  (`Writer`, `Bytes`, `Error`, …) also gets a trailing `_` (`writer` → `Writer_`);
- optional fields are `Option<T>`, arrays `Vec<T>`, buffers `bytes::Bytes`,
  fixed-size fields `[u8; N]`, `ipv4`/`ipv6`/`ip`/`ipAddress` the matching
  `std::net` types, enums Rust enums;
- every struct and versioned type implements `compact_encoding::Codec` at the
  schema's latest version and carries the version-aware API
  `encoded_len_version(&self, version)`, `encode_version(&self, version, w)` and
  `decode_version(r, version)`. `version` is the JS `setVersion` global: fields
  added in a later schema version are skipped when writing and reading an older
  one;
- versioned types (`"versions": [...]`) are an enum with one variant per declared
  version, `V1(..)`, `V3(..)`; the wire version byte comes from the variant, and
  a payload version below the first definition or inside a gap belongs to the
  first definition at or above it, exactly like the JS `switch`;
- compact structs that a parent embeds inline (`"inline": true`) also get
  `encoded_len_inline`, `encode_inline` and `decode_inline(r, version, flags)`;
  the parent owns the flags word;
- a field that closes a type cycle (a struct reaching itself through non-array
  fields) is boxed: `Option<Box<Node>>`.

Generated code depends on `compact_encoding` and `bytes` only and contains no
`unsafe`.

## Version checks

Lookup by name (`getEncoding`, `encode`, `decode`) is JS-only; Rust is statically
typed, so the type is the entry point:

```rust
let mut r = compact_encoding::Reader::new(bytes);
let value = encoding::bee::TreeDelta::decode_version(&mut r, 1)?;  // read as v1
let mut w = compact_encoding::Writer::new();
value.encode_version(1, &mut w);                                   // write as v1
```

## External types

A schema type declared as `{"external": "Name"}` has no generated codec: the
consumer supplies one, like `encoding/legacy.js` in autobee. Pass the Rust path
through `Options`; it is emitted verbatim, so use an absolute path:

```rust
let opts = hyperschema::Options::new().external("OplogMessageV0", "crate::legacy::OplogMessageV0");
```

The path is only required for externals something actually refers to. The type
must implement `compact_encoding::Codec`.

## Not supported

Rejected with an error naming the feature, because the primitives do not exist in
`compact-encoding-rs`:

| feature | reason |
|---|---|
| `json`, `date` | no JSON/date primitive |
| `lexint` | not ported |
| `bigint` | unbounded BigInt cannot map to a fixed Rust integer |
| `ascii`, `hex` | non-UTF-8 string codecs |

Also rejected:

- `record` fields (hyperschema rejects them itself);
- fields marked `"external": true` (placeholders, no type on the wire);
- a bitwise `uint1`-`uint7` outside a struct's flags, or as an array element;
- `inline` fields whose type is not a compact struct;
- a `flagsPosition` after the first optional field, or on a struct without
  optional fields;
- more than 64 flag bits in one struct (the JS codec keeps flags in a safe
  integer and starts multiplying above 2^31);
- two schema names that map to the same Rust name.

## Deviations from the JS codec

- **`map` functions are not applied.** A versioned type with `"map": "..."` decodes
  to the raw per-version value; the generated variant carries a doc comment naming
  the JS map. Port the mapping in the consumer, or read the raw body.
- **Unknown enum values are an error.** JS returns the number (numeric enums) or
  `null` (string enums); the Rust enum cannot hold a value outside its variants,
  so `from_value` returns `Error::Invalid`.
- **Optional fields are `Option<T>`.** JS decides presence by truthiness, so
  `Some(0)`, `Some("")` and `Some(Bytes::new())` are written with the flag set
  where JS would omit zero/empty values; decoding JS data is unaffected.
- **An optional `optionalBuffer` field is a single `Option<Bytes>`.** JS can
  distinguish "absent", "flag set, null" and "flag set, bytes"; the collapsed Rust
  type cannot, and a flag-set zero length decodes as `Some(Bytes::new())` where JS
  gives `null`.
- **Records are ordered `Vec<(K, V)>`.** JS records are plain objects, so duplicate
  keys collapse and keys have to be strings there; the Rust side keeps wire order
  and duplicates.
- **Legacy `system-writer` versions 0–3 need a `Reader` rewind.** autobee's
  `SystemWriterV0.decode` rewinds the version byte (`state.start--`) so that it
  doubles as the flags word; `compact_encoding::Reader` has no rewind, so a Rust
  external codec cannot reproduce it. The version prefix handling in the generated
  wrapper is correct; only that rewind-dependent legacy layout is out of reach.

## Fixtures

`fixtures/<schema>.json` holds the cases the golden test asserts: for every
generated struct and every schema version, for every enum, array and record, a
few deterministic values encoded by the npm reference codecs, plus the value the
reference decodes from those bytes. `cargo test` decodes each case with the
generated Rust type, compares it field by field and re-encodes it to the same
bytes.

| schema | source | cases | skipped |
|---|---|---|---|
| hyperbee2 | hyperbee2 2.21.1 | 79 | 0 |
| autobee | autobee 2.12.1 | 70 | 15 |
| protomux-wakeup | protomux-wakeup 2.9.0, schema reconstructed from the shipped `index.js` | 15 | 0 |
| hypercore-storage | hypercore-storage 3.4.0 | 80 | 3 |
| hyperdht-address | hyperdht-address 1.1.1 | 5 | 0 |
| synthetic | exercises features the shipped schemas do not use | 156 | 4 |

Skipped cases are versioned payloads the reference cannot encode (decode-only
`legacy.js` types), payloads whose JS decode applies a `map`, and types that are
only reachable through a versioned wrapper (their version byte belongs to the
wrapper).

protomux-wakeup ships no `schema.json`; `fixtures/schemas/protomux-wakeup` is
reconstructed from its generated `index.js` and reproduces that file exactly.
`@wakeup/writer` is marked compact there so that the `announce` array stays
unframed, as in the shipped codec.

Regenerate (needs network for the npm install):

```sh
cd fixtures/gen && npm install && npm run gen
```

The synthetic schema covers what the real ones miss: every primitive, all
`uint1`-`uint7` bitwise widths, framed and unframed versioned types, versioned
fields, inline structs nested two deep, recursion, records, bool bitfields,
framed arrays of non-compact structs, framed structs whose last field is
optional, nested framed versioned bodies, struct fields of array and record type
(required and optional, nested), aliases, constants and `flagsPosition` before a
required field.

## Layout

- `src/model.rs` — schema parsing, type resolution, flags layout, cycle detection;
- `src/codegen.rs` — the emitter;
- `src/naming.rs` — schema name to Rust name;
- `build.rs` — generates the codecs and the fixture expectations the golden test
  compiles;
- `tests/golden.rs` — decode/encode equality against the fixtures;
- `tests/generator.rs` — generator rules and rejections;
- `tests/legacy` — the autobee external types, mirroring `encoding/legacy.js`.

## License

MIT OR Apache-2.0
