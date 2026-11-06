//! Schema name to Rust name conversion. The rules are the same in every
//! generated file, so consumers can predict the identifiers.

/// Rust keywords and reserved words; a name that collides gets a trailing `_`.
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

pub fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
}

/// `tree-delta` and `headV2` both become `TreeDelta`/`HeadV2`.
pub fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;

    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        } else {
            upper = true;
        }
    }

    if out.is_empty() {
        out.push('_');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'N');
    }

    out
}

/// Names the generated prelude imports, so a schema type that reads like one of
/// them gets a trailing `_` instead of shadowing it.
const RESERVED_TYPES: &[&str] = &[
    "Bytes",
    "CReader",
    "CWriter",
    "Codec",
    "Error",
    "IpAddr",
    "Ipv4Addr",
    "Ipv6Addr",
    "MAX_ARRAY_DECODE",
    "Reader",
    "Result",
    "SocketAddr",
    "SocketAddrV4",
    "SocketAddrV6",
    "VERSION",
    "Writer",
];

/// A schema type name as a Rust type name: CamelCase, `_` when it would shadow
/// a prelude name.
pub fn type_name(name: &str) -> String {
    let mut out = camel_case(name);
    if RESERVED_TYPES.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// `defaultDiscoveryKey` becomes `default_discovery_key`; `versionID` becomes
/// `version_id`.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);

    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !out.ends_with('_') {
                out.push('_');
            }
            continue;
        }
        if c.is_ascii_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary && !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }

    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if is_keyword(&out) {
        out.push('_');
    }

    out
}

/// Converts a schema namespace to a module name (`autobase-compat` →
/// `autobase_compat`).
pub fn module_name(namespace: &str) -> String {
    let name = snake_case(namespace);
    if name.is_empty() {
        "schema".to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel() {
        assert_eq!(camel_case("tree-delta"), "TreeDelta");
        assert_eq!(camel_case("head-v1"), "HeadV1");
        assert_eq!(camel_case("keyPair"), "KeyPair");
        assert_eq!(camel_case("blake2b"), "Blake2b");
        assert_eq!(camel_case("ed25519"), "Ed25519");
        assert_eq!(camel_case("3d"), "N3d");
        assert_eq!(type_name("writer"), "Writer_");
        assert_eq!(type_name("info"), "Info");
    }

    #[test]
    fn snake() {
        assert_eq!(snake_case("defaultDiscoveryKey"), "default_discovery_key");
        assert_eq!(snake_case("treeHash"), "tree_hash");
        assert_eq!(snake_case("versionID"), "version_id");
        assert_eq!(snake_case("type"), "type_");
        assert_eq!(snake_case("isRemoved"), "is_removed");
        assert_eq!(snake_case("key"), "key");
        assert_eq!(snake_case("t"), "t");
        assert_eq!(snake_case("type3"), "type3");
    }

    #[test]
    fn modules() {
        assert_eq!(module_name("autobase-compat"), "autobase_compat");
        assert_eq!(module_name("core"), "core");
    }
}
