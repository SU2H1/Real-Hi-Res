"""Writes rust/crates/core/src/pyunicode.rs: the tables of Python's str.isalnum, str.isspace and
str.casefold.  Run from the repository root:

    python3 rust/tools/gen_pyunicode.py rust/crates/core/src/pyunicode.rs
"""
import sys, unicodedata

def ranges(pred):
    out, start = [], None
    for cp in range(0x110000):
        ok = pred(chr(cp))
        if ok and start is None:
            start = cp
        elif not ok and start is not None:
            out.append((start, cp - 1)); start = None
    if start is not None:
        out.append((start, 0x10FFFF))
    return out

def esc(s):
    return "".join(c if 0x20 <= ord(c) < 0x7f and c not in '"\\' else f"\\u{{{ord(c):x}}}" for c in s)

alnum = ranges(str.isalnum)
space = ranges(str.isspace)
fold = [(cp, chr(cp).casefold()) for cp in range(0x110000)
        if not (0xD800 <= cp <= 0xDFFF) and chr(cp).casefold() != chr(cp)]
lines = [
    "//! Python's `str.isalnum()`, `str.isspace()` and `str.casefold()`, so names are",
    "//! compared exactly as the Python version compares them (`lyrics.normalize_name`).",
    f"//! Generated from the Unicode {unicodedata.unidata_version} database of Python {sys.version.split()[0]}",
    "//! by `rust/tools/gen_pyunicode.py`; do not edit.",
    "",
    "fn in_ranges(table: &[(u32, u32)], c: char) -> bool {",
    "    let c = c as u32;",
    "    table.binary_search_by(|&(lo, hi)| if hi < c { std::cmp::Ordering::Less } else if lo > c { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal }).is_ok()",
    "}",
    "",
    "/// `c.isalnum()`: letters and numbers of every script (what `\\w` matches, without `_`).",
    "pub fn is_alnum(c: char) -> bool {",
    "    if c.is_ascii() {",
    "        return c.is_ascii_alphanumeric();",
    "    }",
    "    in_ranges(ALNUM, c)",
    "}",
    "",
    "/// `c.isspace()` (what `\\s` matches).",
    "pub fn is_space(c: char) -> bool {",
    "    in_ranges(SPACE, c)",
    "}",
    "",
    "/// Python's `\\w`: `isalnum()` or `_`.",
    "pub fn is_word(c: char) -> bool {",
    "    c == '_' || is_alnum(c)",
    "}",
    "",
    "/// `s.casefold()`.",
    "pub fn casefold(s: &str) -> String {",
    "    let mut out = String::with_capacity(s.len());",
    "    for c in s.chars() {",
    "        match FOLD.binary_search_by(|(k, _)| k.cmp(&c)) {",
    "            Ok(i) => out.push_str(FOLD[i].1),",
    "            Err(_) => out.push(c),",
    "        }",
    "    }",
    "    out",
    "}",
    "",
    "/// `s.strip()`.",
    "pub fn strip(s: &str) -> &str {",
    "    s.trim_matches(is_space)",
    "}",
    "",
    "/// `s.rstrip()`.",
    "pub fn rstrip(s: &str) -> &str {",
    "    s.trim_end_matches(is_space)",
    "}",
    "",
    "#[rustfmt::skip]",
    "const ALNUM: &[(u32, u32)] = &[",
]
row = []
for lo, hi in alnum:
    row.append(f"(0x{lo:x}, 0x{hi:x}),")
for i in range(0, len(row), 6):
    lines.append("    " + " ".join(row[i:i + 6]))
lines += ["];", "", "#[rustfmt::skip]", "const SPACE: &[(u32, u32)] = &["]
lines.append("    " + " ".join(f"(0x{lo:x}, 0x{hi:x})," for lo, hi in space))
lines += ["];", "", "#[rustfmt::skip]", "const FOLD: &[(char, &str)] = &["]
row = [f"('\\u{{{cp:x}}}', \"{esc(v)}\")," for cp, v in fold]
for i in range(0, len(row), 4):
    lines.append("    " + " ".join(row[i:i + 4]))
lines += ["];", "", "#[cfg(test)]", "mod tests {", "    use super::*;", "",
          "    #[test]", "    fn like_python() {",
          "        assert_eq!(casefold(\"Straße ΣΑΣ\"), \"strasse σασ\");",
          "        assert!(is_alnum('夜') && is_alnum('٣') && !is_alnum('_') && !is_alnum('\\u{0301}'));",
          "        assert!(is_space('\\u{1c}') && is_space('\\u{3000}') && !is_space('\\u{200b}'));",
          "        assert_eq!(strip(\"\\u{a0} x \\n\"), \"x\");", "    }", "}", ""]
open(sys.argv[1], "w", encoding="utf-8").write("\n".join(lines))
print(len(alnum), len(space), len(fold))
