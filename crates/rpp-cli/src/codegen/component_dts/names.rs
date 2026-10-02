//! Identifier, key and type-name helpers for the generated declarations.

fn words(value: &str) -> impl Iterator<Item = &str> {
    value.split(['-', '_']).filter(|word| !word.is_empty())
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// Lower camelCase of a kebab-case name, as the Rust `heck` crate does.
fn camel_case(value: &str) -> String {
    words(value)
        .enumerate()
        .map(|(index, word)| {
            if index == 0 {
                word.to_lowercase()
            } else {
                capitalize(word)
            }
        })
        .collect()
}

/// Names the generated declarations must not shadow: TS globals and common lib types.
const GLOBAL_TYPES: &[&str] = &[
    "Array",
    "ReadonlyArray",
    "Uint8Array",
    "BigInt",
    "Record",
    "Partial",
    "Promise",
    "Error",
    "Object",
    "String",
    "Number",
    "Boolean",
    "Map",
    "Set",
    "Date",
    "Symbol",
    "Function",
];

/// A PascalCase type name that does not shadow a TS global.
pub(super) fn type_name(value: &str) -> String {
    let name = pascal_case(value);
    if GLOBAL_TYPES.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
}

pub(super) fn pascal_case(value: &str) -> String {
    let name: String = words(value).map(capitalize).collect();
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{name}")
    } else {
        name
    }
}

/// Words that cannot be parameter names.
const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "await",
];

fn is_identifier(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `name` exactly, quoted when it is not a valid identifier.
pub(super) fn quote_key(name: &str) -> String {
    if is_identifier(name) {
        name.to_string()
    } else {
        format!("{name:?}")
    }
}

/// A camelCase property name, quoted when it is not a valid identifier.
pub(super) fn property_key(name: &str) -> String {
    quote_key(&camel_case(name))
}

/// A camelCase parameter name that is not a reserved word.
pub(super) fn identifier(name: &str) -> String {
    let camel = camel_case(name);
    if RESERVED.contains(&camel.as_str()) || camel.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{camel}")
    } else {
        camel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_matches_shared_vectors() {
        for (input, expected) in [
            ("foo", "foo"),
            ("foo-bar", "fooBar"),
            ("HTTP-get", "httpGet"),
            ("a-b-c", "aBC"),
            ("x2-y", "x2Y"),
        ] {
            assert_eq!(camel_case(input), expected);
        }
        assert_eq!(pascal_case("parse-error"), "ParseError");
    }
}
