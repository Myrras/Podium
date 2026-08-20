//! Minimal justfile recipe scanner: reads the project root's justfile and
//! returns the public recipe names, feeding the frontend's terminal
//! quick-commands list. Parsing is line-based on purpose — no dependency on
//! a `just` binary being installed, no subprocess per sidebar render.
//!
//! ponytail: naive single-line parser — it skips indented bodies, comments,
//! `:=` assignments, keywords and private recipes, but does not understand
//! quoted default values containing `:` or line continuations. Good enough
//! for real-world justfiles; swap for `just --summary` if it ever isn't.

use std::path::Path;

/// Filenames `just` itself looks for, in its own precedence order.
const CANDIDATES: &[&str] = &["justfile", "Justfile", "JUSTFILE", ".justfile"];

/// Public recipe names from the justfile at `root`, in file order. Empty when
/// there is no justfile (or it has no public recipes).
pub fn recipes(root: &Path) -> Vec<String> {
    CANDIDATES
        .iter()
        .find_map(|name| std::fs::read_to_string(root.join(name)).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Keywords that introduce non-recipe top-level items.
const KEYWORDS: &[&str] = &[
    "set", "import", "import?", "mod", "mod?", "export", "unexport", "alias",
];

fn parse(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Set by an attribute line (e.g. `[private]`) for the recipe that follows.
    let mut private_attr = false;
    for line in text.lines() {
        // Indented lines are recipe bodies; blanks and comments are noise.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            private_attr = private_attr || line.contains("private");
            continue;
        }
        let was_private = std::mem::take(&mut private_attr);
        let first = line.split_whitespace().next().unwrap_or("");
        if KEYWORDS.contains(&first) {
            continue;
        }
        // `@name` runs quiet — same recipe, strip the sigil.
        let rest = line.strip_prefix('@').unwrap_or(line);
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            continue;
        }
        let tail = &rest[name.len()..];
        // `name := value` is an assignment, not a recipe.
        if tail.trim_start().starts_with(":=") {
            continue;
        }
        // A recipe header has a `:` (after optional parameters) not part of `:=`.
        let is_recipe = tail
            .char_indices()
            .any(|(i, c)| c == ':' && !tail[i + 1..].starts_with('='));
        if !is_recipe || was_private || name.starts_with('_') {
            continue;
        }
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_recipes_in_order() {
        let text = "\
# comment
set shell := [\"bash\", \"-c\"]
version := \"1.2.0\"
alias b := build

default:
    @just --list

build: check
    cargo build

@quiet arg='x':
    echo {{arg}}

test filter='':
    cargo test {{filter}}
";
        assert_eq!(parse(text), ["default", "build", "quiet", "test"]);
    }

    #[test]
    fn skips_private_recipes_and_non_recipe_items() {
        let text = "\
_helper:
    true

[private]
secret:
    true

[group('ci')]
lint:
    cargo clippy

import 'other.just'
mod bar
export FOO := \"1\"
";
        assert_eq!(parse(text), ["lint"]);
    }

    #[test]
    fn reads_the_justfile_at_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("justfile"), "dev:\n    pnpm dev\n").unwrap();
        assert_eq!(recipes(dir.path()), ["dev"]);
        assert!(recipes(&dir.path().join("missing")).is_empty());
    }
}
