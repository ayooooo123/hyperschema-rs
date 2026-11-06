//! Byte-for-byte fixtures.
//!
//! `fixtures/gen/gen.js` encodes deterministic values with the npm reference
//! (`hyperschema` codecs of the five modules, plus the synthetic schema), and
//! `build.rs` turns each case into a Rust literal of the generated type. Every
//! case here must decode to that value and re-encode to the recorded bytes.

use bytes::Bytes;
use compact_encoding::{Codec, Reader, Writer};

mod legacy;

mod hyperbee2_spec {
    include!(concat!(env!("OUT_DIR"), "/hyperbee2.rs"));
}

mod autobee_spec {
    include!(concat!(env!("OUT_DIR"), "/autobee.rs"));
}

mod protomux_wakeup_spec {
    include!(concat!(env!("OUT_DIR"), "/protomux-wakeup.rs"));
}

mod hypercore_storage_spec {
    include!(concat!(env!("OUT_DIR"), "/hypercore-storage.rs"));
}

mod hyperdht_address_spec {
    include!(concat!(env!("OUT_DIR"), "/hyperdht-address.rs"));
}

mod synthetic_spec {
    include!(concat!(env!("OUT_DIR"), "/synthetic.rs"));
}

type CaseGroup = (&'static str, fn() -> Vec<Case>);

/// One fixture: the expected value is baked into `run`.
pub struct Case {
    pub name: &'static str,
    pub run: Box<dyn Fn()>,
}

pub fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length: {s}");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex digit"))
        .collect()
}

#[allow(clippy::all)]
mod cases_hyperbee2 {
    include!(concat!(env!("OUT_DIR"), "/cases_hyperbee2.rs"));
}

#[allow(clippy::all)]
mod cases_autobee {
    include!(concat!(env!("OUT_DIR"), "/cases_autobee.rs"));
}

#[allow(clippy::all)]
mod cases_protomux_wakeup {
    include!(concat!(env!("OUT_DIR"), "/cases_protomux-wakeup.rs"));
}

#[allow(clippy::all)]
mod cases_hypercore_storage {
    include!(concat!(env!("OUT_DIR"), "/cases_hypercore-storage.rs"));
}

#[allow(clippy::all)]
mod cases_hyperdht_address {
    include!(concat!(env!("OUT_DIR"), "/cases_hyperdht-address.rs"));
}

#[allow(clippy::all)]
mod cases_synthetic {
    include!(concat!(env!("OUT_DIR"), "/cases_synthetic.rs"));
}

#[test]
fn every_fixture_case_round_trips() {
    let groups: [CaseGroup; 6] = [
        ("hyperbee2", cases_hyperbee2::cases),
        ("autobee", cases_autobee::cases),
        ("protomux-wakeup", cases_protomux_wakeup::cases),
        ("hypercore-storage", cases_hypercore_storage::cases),
        ("hyperdht-address", cases_hyperdht_address::cases),
        ("synthetic", cases_synthetic::cases),
    ];

    let mut failures = Vec::new();
    let mut total = 0usize;

    for (schema, cases) in groups {
        let cases = cases();
        assert!(!cases.is_empty(), "{schema} has no fixtures");
        total += cases.len();
        for case in cases {
            let name = case.name;
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (case.run)())).is_err() {
                failures.push(format!("{schema}: {name}"));
            }
        }
    }

    assert!(total > 0);
    assert!(
        failures.is_empty(),
        "{} of {total} fixture cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Shared by every generated case: decode the bytes, compare the value, then
/// encode the value again.
pub fn check<T>(expected: &T, version: u64, hex: Vec<u8>)
where
    T: VersionedValue + Codec + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        expected.encoded_len_at(version),
        hex.len(),
        "encoded length at version {version}"
    );

    let mut w = Writer::new();
    expected.encode_at(version, &mut w);
    assert_eq!(w.finish().as_ref(), &hex[..], "encode at version {version}");

    let mut r = Reader::new(Bytes::from(hex));
    let decoded = T::decode_at(&mut r, version).expect("decode");
    assert_eq!(&decoded, expected, "decode at version {version}");
}

/// The version aware half of the generated API.
pub trait VersionedValue: Sized {
    fn encoded_len_at(&self, version: u64) -> usize;
    fn encode_at(&self, version: u64, w: &mut Writer);
    fn decode_at(r: &mut Reader, version: u64) -> compact_encoding::Result<Self>;
}

/// Non-versioned values are checked through their `Codec` impl.
pub fn check_codec<T>(expected: &T, hex: Vec<u8>)
where
    T: Codec + PartialEq + std::fmt::Debug,
{
    assert_eq!(expected.encoded_len(), hex.len(), "encoded length");
    let mut w = Writer::new();
    expected.encode(&mut w);
    assert_eq!(w.finish().as_ref(), &hex[..], "encode");
    let mut r = Reader::new(Bytes::from(hex));
    assert_eq!(&T::decode(&mut r).expect("decode"), expected, "decode");
}

#[test]
fn a_wire_version_belongs_to_the_first_definition_at_or_above_it() {
    // `@syn/plain-box` declares version 1 and version 3, so a version 2 payload
    // is the version 3 body, exactly like the JS switch.
    let mut r = Reader::new(Bytes::from(hex("020700")));
    let decoded = synthetic_spec::syn::PlainBox::decode_version(&mut r, 1).expect("decode");
    assert_eq!(
        decoded,
        synthetic_spec::syn::PlainBox::V3(synthetic_spec::syn::V3Body { a: 7, c: None })
    );
}

#[test]
fn unknown_versions_and_enum_values_are_rejected() {
    let mut r = Reader::new(Bytes::from(hex("09")));
    assert!(synthetic_spec::syn::PlainBox::decode_version(&mut r, 3).is_err());

    assert!(synthetic_spec::syn::Color::from_value(99).is_err());
    assert_eq!(
        synthetic_spec::syn::Color::from_value(6).ok(),
        Some(synthetic_spec::syn::Color::Blue)
    );
}
