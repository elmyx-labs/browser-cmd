//! No module reaches a layer above its own (ARCHITECTURE.md), checked from the source.

use std::path::Path;

/// Low to high; a module may use its own layer and those below.
const LAYERS: &[&[&str]] = &[
    &["error", "text", "env", "http", "web"],
    &["settings", "workspace"],
    &["chrome"],
    &["input"],
    &["page"],
    &["browser"],
    &["tabs"],
    &["session"],
    &["tools"],
    &["api"],
];

/// What lib.rs re-exports at the crate root, by the module it comes from.
const REEXPORTS: &[(&str, &str)] = &[
    ("Error", "error"),
    ("Result", "error"),
    ("Settings", "settings"),
    ("Host", "browser"),
    ("Outcome", "session"),
    ("Image", "session"),
];

fn layer(module: &str) -> Option<usize> {
    let module = REEXPORTS.iter().find(|(name, _)| *name == module).map_or(module, |(_, from)| from);
    LAYERS.iter().position(|layer| layer.contains(&module))
}

/// The first segment of every `crate::` path, including those inside `use crate::{…}` groups.
fn used(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("crate::") {
        rest = &rest[at + "crate::".len()..];
        if let Some(group) = rest.strip_prefix('{') {
            let mut depth = 1;
            let mut item = true;
            for (index, c) in group.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 1 => item = true,
                    c if item && depth == 1 && (c.is_alphanumeric() || c == '_') => {
                        let name: String =
                            group[index..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        found.push(name);
                        item = false;
                    }
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
            }
        } else {
            found.push(rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect());
        }
    }
    found
}

fn visit(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn no_module_uses_a_layer_above_its_own() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&src, &mut files);
    let mut wrong = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&src).unwrap();
        let top = relative
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .trim_end_matches(".rs")
            .to_owned();
        if matches!(top.as_str(), "tests" | "lib" | "main") {
            continue;
        }
        let own = layer(&top).unwrap_or_else(|| panic!("{top} is in no layer: add it to LAYERS"));
        let text = std::fs::read_to_string(&file).unwrap();
        let code: String = text
            .split("#[cfg(test)]\nmod ")
            .next()
            .unwrap()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for name in used(&code) {
            let theirs = layer(&name)
                .unwrap_or_else(|| panic!("{}: crate::{name} is in no layer", relative.display()));
            if theirs > own {
                wrong.push(format!("{} uses crate::{name}", relative.display()));
            }
        }
    }
    assert!(wrong.is_empty(), "layering broken:\n{}", wrong.join("\n"));
}
