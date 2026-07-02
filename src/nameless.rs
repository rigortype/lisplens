//! Modelling [Nameless](https://github.com/Malabarba/Nameless) for indentation
//! (ADR-0030).
//!
//! Nameless hides a package's namespace prefix, composing e.g. `php-foo` to
//! display as `:foo`. With `nameless-affect-indentation-and-filling` at its
//! default, Emacs measures alignment against that *displayed* width, so code
//! edited under Nameless is checked in with narrower alignment. This module
//! reports, for a symbol, how many columns its prefix collapses by; the
//! formatter subtracts that from every column measured to its right.

/// `nameless-global-aliases` default: `font-lock-` displays as `fl:`.
const DEFAULT_ALIASES: &[(&str, &str)] = &[("fl", "font-lock")];

/// `nameless-prefix` — the glyph shown in place of the namespace.
const NAMELESS_PREFIX: &str = ":";

/// Displayed columns of a composed prefix whose glyph string is `dis_len`
/// characters. Nameless composes via `(Br . Bl)` reference-point rules
/// (`nameless--make-composition`), which pack the glyphs to roughly half width;
/// Emacs's `current-column` then measures `⌊dis_len/2⌋ + 1` (verified against
/// Emacs for lengths 1–5). So `:` (1) → 1 column and `fl:` (3) → 2.
fn display_cols(dis_len: usize) -> usize {
    dis_len / 2 + 1
}

/// The composed prefixes that shrink column measurement for one file.
pub struct Nameless {
    /// `(region, display_cols)` — the matched text (namespace + separator) and
    /// the width it collapses to, e.g. `("php-", 1)`, `("font-lock-", 3)`.
    prefixes: Vec<(String, usize)>,
}

impl Nameless {
    /// Emulation for a file named `file_name`, with the default aliases.
    /// `nameless-prefix` is `:` (1 column) and `nameless-separator` is `-`.
    pub fn for_file(file_name: &str) -> Self {
        const SEP: &str = "-";
        let mut prefixes = Vec::new();
        if let Some(name) = current_name(file_name) {
            // `NAME-rest` displays as `:rest` — glyph string is `nameless-prefix`.
            prefixes.push((format!("{name}{SEP}"), display_cols(NAMELESS_PREFIX.len())));
        }
        for (alias, ns) in DEFAULT_ALIASES {
            // `NS-rest` displays as `ALIAS:rest` — glyph string is `alias` + `:`.
            prefixes.push((
                format!("{ns}{SEP}"),
                display_cols(alias.len() + NAMELESS_PREFIX.len()),
            ));
        }
        Nameless { prefixes }
    }

    /// Columns saved by composing `symbol` — nonzero only when it begins with a
    /// composed prefix and has at least one character after it (Nameless's
    /// regexp requires a symbol constituent after the separator).
    pub fn saving(&self, symbol: &str) -> usize {
        for (region, display) in &self.prefixes {
            if symbol.len() > region.len() && symbol.starts_with(region.as_str()) {
                return region.len() - display;
            }
        }
        0
    }
}

/// Nameless's `nameless-current-name` discovery: strip a trailing
/// `(-mode)?(-tests?)?\.EXT` from the file's basename. `php-mode.el` → `php`,
/// `php-project.el` → `php-project`, `foo-mode-tests.el` → `foo`.
fn current_name(file_name: &str) -> Option<String> {
    let base = file_name.rsplit('/').next().unwrap_or(file_name);
    // `\.[^.]*\'` — drop the extension at the last dot.
    let mut s = match base.rfind('.') {
        Some(i) => &base[..i],
        None => base,
    };
    // `(-tests?)?` then `(-mode)?`, right-to-left.
    for suf in ["-tests", "-test"] {
        if let Some(t) = s.strip_suffix(suf) {
            s = t;
            break;
        }
    }
    if let Some(t) = s.strip_suffix("-mode") {
        s = t;
    }
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_name_matches_nameless_discovery() {
        assert_eq!(current_name("php-mode.el").as_deref(), Some("php"));
        assert_eq!(
            current_name("php-project.el").as_deref(),
            Some("php-project")
        );
        assert_eq!(current_name("php.el").as_deref(), Some("php"));
        assert_eq!(current_name("foo-mode-tests.el").as_deref(), Some("foo"));
        assert_eq!(current_name("bar-test.el").as_deref(), Some("bar"));
        assert_eq!(
            current_name("/a/b/php-face.el").as_deref(),
            Some("php-face")
        );
    }

    #[test]
    fn savings_for_current_name_and_alias() {
        let nl = Nameless::for_file("php-mode.el");
        assert_eq!(nl.saving("php-mode-some-function"), 3); // php- (4) -> : (1)
        assert_eq!(nl.saving("font-lock-add-keywords"), 8); // font-lock- (10) -> fl: (2)
        assert_eq!(nl.saving("php"), 0); // no separator, no trailing char
        assert_eq!(nl.saving("php-"), 0); // no trailing char
        assert_eq!(nl.saving("other-symbol"), 0);
    }
}
