use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=messages.tsv");
    let catalog = fs::read_to_string("messages.tsv").expect("read translation catalog");
    let mut messages = String::from("pub const MESSAGES: &[Message] = &[\n");
    let mut text =
        String::from("pub fn text_in(language: Language, key: &str) -> &str {\nmatch key {\n");
    let mut formats = String::from("#[macro_export]\nmacro_rules! tr_format {\n");
    for (line, row) in catalog.lines().enumerate() {
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        // Columns are Rust string literals: key, English, French.
        let cols: Vec<_> = row.split('\t').collect();
        assert_eq!(
            cols.len(),
            3,
            "messages.tsv:{}: expected three columns",
            line + 1
        );
        let [key, en, fr] = [cols[0], cols[1], cols[2]];
        messages.push_str(&format!("Message {{ key: {key}, en: {en}, fr: {fr} }},\n"));
        text.push_str(&format!(
            "{key} => match language {{ Language::English => {en}, Language::French => {fr} }},\n"
        ));
        if key.contains('{') {
            for args in [false, true] {
                let pattern = if args { ", $($args:tt)+" } else { "" };
                let pass = if args { ", $($args)+" } else { "" };
                formats.push_str(&format!(
                    "({key}{pattern}) => {{{{ match $crate::language() {{ $crate::Language::English => format!({en}{pass}), $crate::Language::French => format!({fr}{pass}) }} }}}};\n"
                ));
            }
        }
    }
    messages.push_str("] ;\n");
    text.push_str("_ => key,\n}\n}\n");
    formats.push_str("}\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("catalog.rs"),
        format!("{messages}\n{text}\n{formats}"),
    )
    .unwrap();
}
