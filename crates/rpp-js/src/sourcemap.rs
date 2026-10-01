//! Mapping bundled stack locations back to original sources.

use rolldown_sourcemap::{JSONSourceMap, SourceMap};

/// Rewrite every `<module>:<line>:<column>` location in `stack` (1-based, as V8
/// prints them) through `source_map` to `<source>:<line>:<column>`, where `<source>`
/// is the source map's source path. Locations in other modules, or without a mapping,
/// are left unchanged.
pub(crate) fn map_stack(stack: &str, module: &str, source_map: &str) -> String {
    let Ok(json) = serde_json::from_str::<JSONSourceMap>(source_map) else {
        return stack.to_string();
    };
    let Ok(map) = SourceMap::from_json(json) else {
        return stack.to_string();
    };
    let table = map.generate_lookup_table();
    let needle = format!("{module}:");

    let mut out = String::with_capacity(stack.len());
    let mut rest = stack;
    while let Some(at) = rest.find(&needle) {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);
        let after = &tail[needle.len()..];
        let boundary = before
            .chars()
            .next_back()
            .is_none_or(|c| !is_module_char(c));
        let mapped = if boundary {
            parse_position(after).and_then(|(line, column, len)| {
                let token = map.lookup_token(&table, line - 1, column - 1)?;
                let source = map.get_source(token.get_source_id()?)?;
                let text = format!(
                    "{source}:{}:{}",
                    token.get_src_line() + 1,
                    token.get_src_col() + 1
                );
                Some((text, len))
            })
        } else {
            None
        };
        match mapped {
            Some((text, len)) => {
                out.push_str(&text);
                rest = &after[len..];
            }
            None => {
                out.push_str(&needle);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_module_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#' | '@')
}

/// Parses a leading `<line>:<column>` (both at least 1) and returns both numbers plus
/// the number of bytes consumed.
fn parse_position(text: &str) -> Option<(u32, u32, usize)> {
    let (line, after_line) = take_number(text)?;
    let after_colon = after_line.strip_prefix(':')?;
    let (column, after_column) = take_number(after_colon)?;
    if line == 0 || column == 0 {
        return None;
    }
    Some((line, column, text.len() - after_column.len()))
}

fn take_number(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let value = text[..end].parse().ok()?;
    Some((value, &text[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Generated line 3 column 4 (0-based 2:3) maps to src/x.ts line 10 column 6 (0-based 9:5).
    const MAP: &str = r#"{"version":3,"sources":["src/x.ts"],"names":[],"mappings":";;GASK"}"#;

    #[test]
    fn maps_frames_in_the_bundle_module() {
        let stack = "Error: boom\n    at transform (rpp:plugin/x:3:4)\n    at rpp:plugin/x:3:5";
        let mapped = map_stack(stack, "rpp:plugin/x", MAP);
        assert_eq!(
            mapped,
            "Error: boom\n    at transform (src/x.ts:10:6)\n    at src/x.ts:10:6"
        );
    }

    #[test]
    fn leaves_other_modules_unchanged() {
        let stack = "Error: boom\n    at run (rpp:other:3:4)\n    at rpp:plugin/xy:3:4";
        assert_eq!(map_stack(stack, "rpp:plugin/x", MAP), stack);
    }
}
