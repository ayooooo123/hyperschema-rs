//! Generator rules that fixtures cannot express: rejections, flag layout,
//! versioned framing and name conversion. The wire behaviour itself is covered
//! by `golden.rs`.

use hyperschema::{Error, Options, generate};

fn generate_schema(body: &str) -> String {
    let json = format!(r#"{{"version":1,"schema":[{body}]}}"#);
    generate(&json, &Options::new()).expect("generate")
}

fn err(body: &str) -> String {
    let json = format!(r#"{{"version":1,"schema":[{body}]}}"#);
    match generate(&json, &Options::new()) {
        Ok(_) => panic!("expected an error for {body}"),
        Err(Error::Unsupported(m) | Error::Schema(m)) => m,
        Err(other) => panic!("unexpected error {other}"),
    }
}

fn field(name: &str, ty: &str, extra: &str) -> String {
    format!(r#"{{"name":"{name}","type":"{ty}","version":1{extra}}}"#)
}

fn optional_struct(name: &str, count: usize) -> String {
    let fields: Vec<String> = (0..count)
        .map(|i| field(&format!("f{i}"), "uint", ""))
        .collect();
    format!(
        r#"{{"name":"{name}","namespace":"n","compact":true,"fields":[{}]}}"#,
        fields.join(",")
    )
}

#[test]
fn flags_width_follows_the_bit_count() {
    let one = generate_schema(&optional_struct("one", 1));
    assert!(one.contains("w.uint8(flags as u8);"), "{one}");
    assert!(one.contains("n += 1;"), "one flags byte");

    let eight = generate_schema(&optional_struct("eight", 8));
    assert!(eight.contains("w.uint8(flags as u8);"));
    assert!(eight.contains("let flags = r.uint8()? as u64;"));

    let nine = generate_schema(&optional_struct("nine", 9));
    assert!(nine.contains("w.uint16(flags as u16);"), "{nine}");
    assert!(nine.contains("n += 2;"));
    assert!(nine.contains("r.uint16()? as u64"));

    let sixteen = generate_schema(&optional_struct("sixteen", 16));
    assert!(sixteen.contains("w.uint16(flags as u16);"));
}

#[test]
fn non_compact_structs_use_a_uint_flags_word() {
    let code = generate_schema(
        r#"{"name":"s","namespace":"n","compact":false,"fields":[
        {"name":"a","type":"uint","version":1}
    ]}"#,
    );
    assert!(code.contains("w.uint(flags);"), "{code}");
    assert!(code.contains("n += uint_len(flags);"));
    assert!(code.contains("let flags = r.uint()?;"));
}

#[test]
fn flags_position_sits_before_the_field_it_is_attached_to() {
    let code = generate_schema(
        r#"{"name":"s","namespace":"n","compact":false,"flagsPosition":1,"fields":[
        {"name":"head","required":true,"type":"uint","version":1},
        {"name":"body","required":true,"type":"fixed32","version":1},
        {"name":"tail","type":"uint","version":1}
    ]}"#,
    );
    let encode = &code[code.find("pub fn encode_version").unwrap()..];
    let head = encode.find("w.uint(self.head);").unwrap();
    let flags = encode.find("w.uint(flags);").unwrap();
    let body = encode.find("w.raw(&self.body);").unwrap();
    assert!(head < flags && flags < body, "{encode}");

    let decode = &code[code.find("pub fn decode_version").unwrap()..];
    let head = decode.find("let v0 = r.uint()?;").unwrap();
    let flags = decode.find("let flags = r.uint()?;").unwrap();
    assert!(head < flags);
}

#[test]
fn flags_position_after_the_first_optional_field_is_rejected() {
    let message = err(r#"{"name":"s","namespace":"n","flagsPosition":2,"fields":[
        {"name":"a","required":true,"type":"uint","version":1},
        {"name":"b","type":"uint","version":1}
    ]}"#);
    assert!(
        message.contains("after the first optional field"),
        "{message}"
    );
}

#[test]
fn flags_position_without_optionals_is_rejected() {
    let message = err(r#"{"name":"s","namespace":"n","flagsPosition":1,"fields":[
        {"name":"a","required":true,"type":"uint","version":1},
        {"name":"b","required":true,"type":"uint","version":1}
    ]}"#);
    assert!(message.contains("no field is optional"), "{message}");
}

#[test]
fn flags_are_elided_only_when_the_field_did_not_arrive_later() {
    let always = generate_schema(
        r#"{"name":"s","namespace":"n","compact":true,"fields":[
        {"name":"a","required":true,"type":"uint","version":1},
        {"name":"b","type":"uint","version":1}
    ]}"#,
    );
    assert!(always.contains("let flags = r.uint8()? as u64;"));
    assert!(!always.contains("r.remaining() > 0"), "{always}");

    let gated = generate_schema(
        r#"{"name":"s","namespace":"n","compact":true,"fields":[
        {"name":"a","required":true,"type":"uint","version":1},
        {"name":"b","type":"uint","version":2}
    ]}"#,
    );
    assert!(
        gated.contains("let flags = if r.remaining() > 0 { r.uint8()? as u64 } else { 0 };"),
        "{gated}"
    );
}

#[test]
fn versioned_types_frame_and_cover_version_ranges() {
    let code = generate_schema(
        r#"{"name":"a","namespace":"n","compact":false,"fields":[
            {"name":"x","required":true,"type":"uint","version":1}
        ]},
        {"name":"b","namespace":"n","compact":false,"fields":[
            {"name":"y","required":true,"type":"uint","version":1}
        ]},
        {"name":"boxed","namespace":"n","framed":true,"versions":[
            {"type":"@n/a","map":null,"version":1},
            {"type":"@n/b","map":null,"version":4}
        ]}"#,
    );

    assert!(code.contains("pub enum Boxed {"), "{code}");
    assert!(code.contains("V1(A)"));
    assert!(code.contains("V4(B)"));
    assert!(code.contains("0..=1 => Ok(Self::V1({") || code.contains("0 | 1 => Ok(Self::V1({"));
    assert!(code.contains("2..=4 => Ok(Self::V4({"));
    assert!(code.contains("let n = v.encoded_len_version(version);"));
    assert!(code.contains("let len = r.uint()? as usize;"));
    assert!(code.contains("unsupported @n/boxed version"));
}

#[test]
fn unframed_versioned_types_skip_the_length_window() {
    let code = generate_schema(
        r#"{"name":"a","namespace":"n","compact":false,"fields":[
            {"name":"x","required":true,"type":"uint","version":1}
        ]},
        {"name":"open","namespace":"n","framed":false,"versions":[
            {"type":"@n/a","map":null,"version":1}
        ]}"#,
    );
    assert!(!code.contains("let len = r.uint()? as usize;"), "{code}");
    assert!(code.contains("uint_len(self.version()) + inner"));
}

#[test]
fn enum_values_are_the_index_plus_the_offset() {
    let code =
        generate_schema(r#"{"name":"color","namespace":"n","offset":4,"enum":["red","green"]}"#);
    assert!(code.contains("Self::Red => 4,"), "{code}");
    assert!(code.contains("Self::Green => 5,"));
    assert!(code.contains("4 => Ok(Self::Red),"));
    assert!(code.contains("unknown @n/color value"));

    let default = generate_schema(r#"{"name":"level","namespace":"n","enum":["low"]}"#);
    assert!(default.contains("Self::Low => 1,"));
}

#[test]
fn string_enums_share_the_numeric_mapping() {
    let code = generate_schema(
        r#"{"name":"kind","namespace":"n","offset":0,"strings":true,"enum":["a","b"]}"#,
    );
    assert!(code.contains("Self::A => 0,"), "{code}");
    assert!(code.contains("1 => Ok(Self::B),"));
}

#[test]
fn recursive_fields_are_boxed() {
    let code = generate_schema(
        r#"{"name":"node","namespace":"n","compact":false,"fields":[
        {"name":"next","type":"@n/node","version":1}
    ]}"#,
    );
    assert!(code.contains("pub next: Option<Box<Node>>,"), "{code}");
    assert!(code.contains("Box::new("));

    let plain = generate_schema(
        r#"{"name":"node","namespace":"n","compact":false,"fields":[
        {"name":"value","required":true,"type":"uint","version":1}
    ]}"#,
    );
    assert!(!plain.contains("Box<"));
}

#[test]
fn arrays_of_compact_structs_stay_vecs_and_framed_ones_become_newtypes() {
    let code = generate_schema(
        r#"{"name":"small","namespace":"n","compact":true,"fields":[
            {"name":"k","required":true,"type":"fixed32","version":1}
        ]},
        {"name":"wide","namespace":"n","compact":false,"fields":[
            {"name":"k","required":true,"type":"fixed32","version":1}
        ]},
        {"name":"smalls","namespace":"n","array":true,"type":"@n/small"},
        {"name":"wides","namespace":"n","array":true,"type":"@n/wide"},
        {"name":"bits","namespace":"n","array":true,"type":"bool"}"#,
    );

    assert!(code.contains("pub type Smalls = Vec<Small>;"), "{code}");
    assert!(code.contains("pub struct Wides(pub Vec<Wide>);"));
    assert!(code.contains("pub struct Bits(pub Vec<bool>);"));
    assert!(code.contains("w.bitarray("));
    assert!(code.contains("let len = r.uint()? as usize;"));
}

#[test]
fn aliases_are_type_aliases() {
    let code = generate_schema(r#"{"name":"h","namespace":"n","alias":"fixed32"}"#);
    assert!(code.contains("pub type H = [u8; 32];"), "{code}");
}

#[test]
fn option_flavoured_fields_are_a_single_option() {
    let code = generate_schema(
        r#"{"name":"s","namespace":"n","compact":false,"fields":[
        {"name":"a","required":true,"type":"optionalBuffer","version":1},
        {"name":"b","type":"optionalBuffer","version":1}
    ]}"#,
    );
    assert!(code.contains("pub a: Option<Bytes>,"), "{code}");
    assert!(code.contains("pub b: Option<Bytes>,"));
    assert!(!code.contains("Option<Option<Bytes>>"));
}

#[test]
fn unsupported_primitives_are_named() {
    for (prim, needle) in [
        ("json", "json"),
        ("date", "date"),
        ("lexint", "lexint"),
        ("ascii", "ascii"),
        ("hex", "hex"),
        ("bigint", "bigint"),
    ] {
        let message = err(&format!(
            r#"{{"name":"s","namespace":"n","fields":[{{"name":"a","required":true,"type":"{prim}","version":1}}]}}"#
        ));
        assert!(message.contains(needle), "{prim}: {message}");
    }
}

#[test]
fn unsupported_shapes_are_named() {
    let record_field = err(r#"{"name":"s","namespace":"n","fields":[
        {"name":"a","record":true,"type":"uint","version":1}]}"#);
    assert!(record_field.contains("record fields"), "{record_field}");

    let external_field = err(r#"{"name":"s","namespace":"n","fields":[
        {"name":"a","external":true,"type":"uint","version":1}]}"#);
    assert!(
        external_field.contains("external placeholder"),
        "{external_field}"
    );

    let bitwise_array = err(r#"{"name":"s","namespace":"n","array":true,"type":"uint3"}"#);
    assert!(bitwise_array.contains("bitwise uint"), "{bitwise_array}");

    let bitwise_record =
        err(r#"{"name":"r","namespace":"n","record":true,"key":"uint3","value":"uint"}"#);
    assert!(bitwise_record.contains("bitwise uint"), "{bitwise_record}");

    let inline_wide = err(r#"{"name":"wide","namespace":"n","compact":false,"fields":[
            {"name":"a","required":true,"type":"uint","version":1}]},
        {"name":"s","namespace":"n","compact":true,"fields":[
            {"name":"w","inline":true,"type":"@n/wide","version":1}]}"#);
    assert!(
        inline_wide.contains("inline requires a compact struct"),
        "{inline_wide}"
    );
}

#[test]
fn rust_names_that_collide_are_rejected() {
    let fields = err(r#"{"name":"s","namespace":"n","fields":[
        {"name":"fooBar","type":"uint","version":1},
        {"name":"foo_bar","type":"uint","version":1}]}"#);
    assert!(fields.contains("both map to"), "{fields}");

    let enums = err(r#"{"name":"e","namespace":"n","enum":["a-b","a_b"]}"#);
    assert!(enums.contains("both map to"), "{enums}");

    let types = err(r#"{"name":"a-b","namespace":"n","enum":["x"]},
        {"name":"a_b","namespace":"n","enum":["y"]}"#);
    assert!(types.contains("both map to the Rust name"), "{types}");
}

#[test]
fn externals_need_a_path_only_when_used() {
    let unused = generate_schema(r#"{"name":"ext","namespace":"n","external":"Ext"}"#);
    assert!(unused.contains("is external"), "{unused}");

    let message = err(r#"{"name":"ext","namespace":"n","external":"Ext"},
        {"name":"s","namespace":"n","fields":[{"name":"a","required":true,"type":"@n/ext","version":1}]}"#);
    assert!(message.contains("Options::external"), "{message}");

    let opts = Options::new().external("Ext", "crate::ext::Ext");
    let json = r#"{"version":1,"schema":[
        {"name":"ext","namespace":"n","external":"Ext"},
        {"name":"s","namespace":"n","fields":[{"name":"a","required":true,"type":"@n/ext","version":1}]}]}"#;
    let code = generate(json, &opts).expect("generate");
    assert!(code.contains("crate::ext::Ext"));
}

#[test]
fn output_is_deterministic() {
    let body = r#"{"name":"s","namespace":"n","compact":true,"fields":[
        {"name":"a","required":true,"type":"uint","version":1},
        {"name":"b","type":"fixed32","version":1}]}"#;
    assert_eq!(generate_schema(body), generate_schema(body));
}

#[test]
fn a_type_named_like_the_prelude_is_renamed() {
    let code = generate_schema(
        r#"{"name":"writer","namespace":"n","compact":true,"fields":[
        {"name":"length","required":true,"type":"uint","version":1}]}"#,
    );
    assert!(code.contains("pub struct Writer_ {"), "{code}");
    assert!(code.contains("impl Codec for Writer_"));
}
