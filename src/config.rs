//! Resolving format config — `indent-tabs-mode` and `tab-width` — from
//! file-local variables, directory-local variables, and EditorConfig, in the
//! Emacs-faithful precedence of ADR-0029 (file-local > dir-local > EditorConfig
//! > defaults).

use std::path::Path;

use lispexp_emacs::dir_locals::DirLocals;
use lispexp_emacs::local_vars::file_locals;

/// Formatting parameters resolved for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatConfig {
    /// Emacs `indent-tabs-mode`: leading indent uses tabs when true.
    pub indent_tabs: bool,
    /// Emacs `tab-width`: columns per tab.
    pub tab_width: usize,
    /// Emacs `lisp-body-indent`: columns for one structural indent step
    /// (a `def…` body, a specform's distinguished/body args). Default 2.
    pub body_indent: usize,
    /// Emacs `comment-column`: the column a lone `;` own-line comment aligns to
    /// (`indent-for-comment`). Default 40.
    pub comment_column: usize,
    /// Whether the file is indented under [Nameless](https://github.com/Malabarba/Nameless)
    /// (ADR-0030), so column measurement must account for its prefix composition.
    /// Enabled by a `nameless-mode` file-/dir-local, or the `--nameless` CLI flag.
    /// The per-file `Nameless` (current name, aliases) is built by the caller.
    pub nameless: bool,
    /// Clojure only (ADR-0040): use the **fixed / Tonsky** indent style (every
    /// symbol-headed list body at a flat `+2`) instead of the default semantic
    /// `:inner`/`:block` style. Enabled by a `clojure-ts-indent-style: fixed`
    /// file-/dir-local, or the `--tonsky` CLI flag. Ignored by the other engines.
    pub clojure_fixed_indent: bool,
}

impl Default for FormatConfig {
    fn default() -> Self {
        FormatConfig {
            indent_tabs: false,
            tab_width: 8,
            body_indent: 2,
            comment_column: 40,
            nameless: false,
            clojure_fixed_indent: false,
        }
    }
}

/// Resolve the config for `path` (whose content is `source`), applying sources
/// low-to-high so the higher-precedence one wins: defaults → EditorConfig →
/// dir-locals → file-locals.
pub fn resolve(path: &Path, source: &str) -> FormatConfig {
    let mut cfg = FormatConfig::default();
    apply_editorconfig(path, &mut cfg);
    apply_dir_locals(path, &mut cfg);
    apply_file_locals(source, &mut cfg);
    cfg
}

/// Interpret one `variable: value` pair against the config.
fn set_var(cfg: &mut FormatConfig, var: &str, val: &str) {
    match var.trim() {
        "indent-tabs-mode" => match val.trim() {
            "nil" => cfg.indent_tabs = false,
            "t" => cfg.indent_tabs = true,
            _ => {}
        },
        "tab-width" => {
            if let Ok(n) = val.trim().parse::<usize>() {
                cfg.tab_width = n;
            }
        }
        "lisp-body-indent" => {
            if let Ok(n) = val.trim().parse::<usize>() {
                cfg.body_indent = n;
            }
        }
        "comment-column" => {
            if let Ok(n) = val.trim().parse::<usize>() {
                cfg.comment_column = n;
            }
        }
        "nameless-mode" => match val.trim() {
            "t" => cfg.nameless = true,
            "nil" => cfg.nameless = false,
            _ => {}
        },
        // clojure-ts-mode's style selector (ADR-0040): `fixed` = Tonsky style.
        "clojure-ts-indent-style" => match val.trim().trim_matches('\'') {
            "fixed" => cfg.clojure_fixed_indent = true,
            "semantic" => cfg.clojure_fixed_indent = false,
            _ => {}
        },
        _ => {}
    }
}

// --- File-local variables (ADR-0029 #1) ------------------------------------

/// Read the file-local variables — the `-*- … -*-` header cookie and the
/// trailing `Local Variables:` block — via `lispexp_emacs::local_vars`, and
/// interpret each binding into the config (last-wins, so the crate's application
/// order is preserved by feeding bindings in `.iter()` order).
fn apply_file_locals(source: &str, cfg: &mut FormatConfig) {
    for (var, val) in file_locals(source).iter() {
        set_var(cfg, var, val);
    }
}

// --- Directory-local variables (ADR-0029 #2) -------------------------------

fn apply_dir_locals(path: &Path, cfg: &mut FormatConfig) {
    // Root-most first so nearer directories, applied later, win.
    let mut dirs: Vec<&Path> = path.ancestors().skip(1).collect();
    dirs.reverse();
    for dir in dirs {
        for name in [".dir-locals.el", ".dir-locals-2.el"] {
            if let Ok(content) = std::fs::read_to_string(dir.join(name)) {
                apply_dir_locals_content(&content, cfg);
            }
        }
    }
}

fn apply_dir_locals_content(content: &str, cfg: &mut FormatConfig) {
    // `lispexp_emacs::dir_locals` parses the alist (both `(MODE . ((VAR . VAL) …))`
    // and `(MODE (VAR . VAL) …)` entry forms, plus subdir nesting); lisplens keeps
    // its own multi-mode applicability filter over the parsed entries. Subdir-scoped
    // groups are ignored here — dir-locals resolution stays a per-directory,
    // whole-file pass (subdir support is a future feature, not the mechanical port).
    for entry in DirLocals::parse(content).entries() {
        if entry.subdir.is_some() || !mode_applies(entry.mode.as_deref()) {
            continue;
        }
        for (var, val) in &entry.vars {
            set_var(cfg, var, val);
        }
    }
}

/// Whether a dir-locals mode entry applies to Emacs Lisp. `None` is the `nil` =
/// all-modes entry; a named mode applies only if it is one of the Lisp modes.
fn mode_applies(mode: Option<&str>) -> bool {
    matches!(
        mode,
        None | Some("emacs-lisp-mode" | "lisp-mode" | "lisp-data-mode" | "prog-mode")
    )
}

// --- EditorConfig (ADR-0029 #3) --------------------------------------------

fn apply_editorconfig(path: &Path, cfg: &mut FormatConfig) {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut set_style = false;
    let mut set_tab = false;
    let mut set_size = false;
    for dir in abs.ancestors().skip(1) {
        let Ok(content) = std::fs::read_to_string(dir.join(".editorconfig")) else {
            continue;
        };
        let rel = abs
            .strip_prefix(dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or_default();
        let (props, is_root) = editorconfig_props(&content, rel);
        if !set_style {
            if let Some(v) = props.indent_tabs {
                cfg.indent_tabs = v;
                set_style = true;
            }
        }
        if !set_tab {
            if let Some(v) = props.tab_width {
                cfg.tab_width = v;
                set_tab = true;
            }
        }
        if !set_size {
            if let Some(v) = props.indent_size {
                cfg.body_indent = v;
                set_size = true;
            }
        }
        if is_root {
            break;
        }
    }
}

#[derive(Default)]
struct EcProps {
    indent_tabs: Option<bool>,
    tab_width: Option<usize>,
    /// EditorConfig `indent_size` → `lisp-body-indent`.
    indent_size: Option<usize>,
}

/// Parse one `.editorconfig`, returning the properties for `rel` (later matching
/// sections win) and whether `root = true`.
fn editorconfig_props(content: &str, rel: &str) -> (EcProps, bool) {
    let mut props = EcProps::default();
    let mut is_root = false;
    let mut matching = false; // are we in a section that matches `rel`?
    let mut indent_size: Option<usize> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            matching = glob_match(glob, rel);
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (
            key.trim().to_ascii_lowercase(),
            val.trim().to_ascii_lowercase(),
        );
        if key == "root" && val == "true" {
            is_root = true;
        }
        if !matching {
            continue;
        }
        match key.as_str() {
            "indent_style" => match val.as_str() {
                "tab" => props.indent_tabs = Some(true),
                "space" => props.indent_tabs = Some(false),
                _ => {}
            },
            "tab_width" => props.tab_width = val.parse().ok(),
            "indent_size" => indent_size = val.parse().ok(),
            _ => {}
        }
    }
    // tab_width defaults to indent_size when unset (EditorConfig).
    if props.tab_width.is_none() {
        props.tab_width = indent_size;
    }
    props.indent_size = indent_size;
    (props, is_root)
}

/// Match an EditorConfig `glob` against a `rel`ative forward-slash path. A glob
/// without a `/` matches the basename in any directory.
fn glob_match(glob: &str, rel: &str) -> bool {
    let basename = rel.rsplit('/').next().unwrap_or(rel);
    for pat in expand_braces(glob) {
        let (p, s) = if pat.contains('/') {
            (pat.as_str(), rel)
        } else {
            (pat.as_str(), basename)
        };
        if match_glob(p.as_bytes(), s.as_bytes()) {
            return true;
        }
    }
    false
}

/// Expand a single level of `{a,b,c}` brace alternation.
fn expand_braces(glob: &str) -> Vec<String> {
    let Some(open) = glob.find('{') else {
        return vec![glob.to_string()];
    };
    let Some(close) = glob[open..].find('}').map(|i| i + open) else {
        return vec![glob.to_string()];
    };
    let (before, after) = (&glob[..open], &glob[close + 1..]);
    let mut out = Vec::new();
    for alt in glob[open + 1..close].split(',') {
        for tail in expand_braces(after) {
            out.push(format!("{before}{alt}{tail}"));
        }
    }
    out
}

fn match_glob(p: &[u8], s: &[u8]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    match p[0] {
        b'*' if p.get(1) == Some(&b'*') => {
            // `**` matches anything, including `/`.
            (0..=s.len()).any(|k| match_glob(&p[2..], &s[k..]))
        }
        b'*' => {
            // `*` matches anything except `/`.
            for k in 0..=s.len() {
                if match_glob(&p[1..], &s[k..]) {
                    return true;
                }
                if s.get(k) == Some(&b'/') {
                    break;
                }
            }
            false
        }
        b'?' => !s.is_empty() && s[0] != b'/' && match_glob(&p[1..], &s[1..]),
        b'[' => match_set(p, s),
        c => !s.is_empty() && s[0] == c && match_glob(&p[1..], &s[1..]),
    }
}

/// Match a `[...]` character class at the head of `p`.
fn match_set(p: &[u8], s: &[u8]) -> bool {
    let Some(close) = p.iter().position(|&b| b == b']') else {
        return false;
    };
    if s.is_empty() {
        return false;
    }
    let (negate, set) = match p.get(1) {
        Some(&b'!') | Some(&b'^') => (true, &p[2..close]),
        _ => (false, &p[1..close]),
    };
    let mut hit = false;
    let mut i = 0;
    while i < set.len() {
        if i + 2 < set.len() && set[i + 1] == b'-' {
            if set[i] <= s[0] && s[0] <= set[i + 2] {
                hit = true;
            }
            i += 3;
        } else {
            if set[i] == s[0] {
                hit = true;
            }
            i += 1;
        }
    }
    hit != negate && match_glob(&p[close + 1..], &s[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_common_editorconfig_patterns() {
        assert!(glob_match("*", "src/foo.el"));
        assert!(glob_match("*.el", "foo.el"));
        assert!(glob_match("*.el", "a/b/foo.el")); // no slash → basename
        assert!(glob_match("*.{el,lisp}", "foo.lisp"));
        assert!(glob_match("src/**/*.el", "src/a/b/foo.el"));
        assert!(!glob_match("*.el", "foo.scm"));
        assert!(glob_match("[abc].el", "b.el"));
        assert!(!glob_match("[!abc].el", "b.el"));
    }

    #[test]
    fn header_and_footer_set_indent_tabs() {
        let mut c = FormatConfig::default();
        apply_file_locals(
            ";;; x -*- indent-tabs-mode: t; tab-width: 4 -*-\n(foo)\n",
            &mut c,
        );
        assert!(c.indent_tabs);
        assert_eq!(c.tab_width, 4);

        let mut c2 = FormatConfig::default();
        apply_file_locals(
            "(foo)\n;; Local Variables:\n;; indent-tabs-mode: t\n;; End:\n",
            &mut c2,
        );
        assert!(c2.indent_tabs);
    }

    #[test]
    fn file_local_overrides_and_shebang_is_skipped() {
        let mut c = FormatConfig::default();
        apply_file_locals(
            "#!/usr/bin/emacs --script\n;; -*- tab-width: 2 -*-\n",
            &mut c,
        );
        assert_eq!(c.tab_width, 2);
    }

    #[test]
    fn dir_locals_apply_to_elisp_modes() {
        let mut c = FormatConfig::default();
        apply_dir_locals_content(
            "((emacs-lisp-mode . ((indent-tabs-mode . t) (tab-width . 3))))",
            &mut c,
        );
        assert!(c.indent_tabs);
        assert_eq!(c.tab_width, 3);
    }

    #[test]
    fn dir_locals_nil_key_applies_to_all_modes() {
        let mut c = FormatConfig::default();
        apply_dir_locals_content("((nil . ((indent-tabs-mode . t))))", &mut c);
        assert!(c.indent_tabs);
    }

    #[test]
    fn dir_locals_accept_both_mode_entry_forms() {
        // Non-dotted `(MODE (VAR . VAL) …)` — the form php-mode's own file uses.
        let mut c = FormatConfig::default();
        apply_dir_locals_content(
            "((emacs-lisp-mode (tab-width . 5) (nameless-mode . t)))",
            &mut c,
        );
        assert_eq!(c.tab_width, 5);
        assert!(c.nameless);

        // Dotted `(MODE . ((VAR . VAL) …))` still works.
        let mut c2 = FormatConfig::default();
        apply_dir_locals_content("((emacs-lisp-mode . ((nameless-mode . t))))", &mut c2);
        assert!(c2.nameless);
    }

    #[test]
    fn nameless_mode_resolves_from_file_local() {
        let mut c = FormatConfig::default();
        apply_file_locals(";;; x -*- nameless-mode: t -*-\n(foo)\n", &mut c);
        assert!(c.nameless);
    }

    #[test]
    fn clojure_ts_indent_style_selects_fixed() {
        let mut c = FormatConfig::default();
        set_var(&mut c, "clojure-ts-indent-style", "fixed");
        assert!(c.clojure_fixed_indent);
        set_var(&mut c, "clojure-ts-indent-style", "semantic");
        assert!(!c.clojure_fixed_indent);
        // A quoted symbol value (`'fixed`) is accepted too.
        set_var(&mut c, "clojure-ts-indent-style", "'fixed");
        assert!(c.clojure_fixed_indent);
    }

    #[test]
    fn editorconfig_space_and_tab_width() {
        let (props, root) = editorconfig_props(
            "root = true\n[*.el]\nindent_style = tab\ntab_width = 4\n",
            "foo.el",
        );
        assert_eq!(props.indent_tabs, Some(true));
        assert_eq!(props.tab_width, Some(4));
        assert!(root);
    }

    #[test]
    fn lisp_body_indent_resolves_from_file_local_and_editorconfig() {
        // File-local `lisp-body-indent` (a `:safe` Emacs var) sets body_indent.
        let mut c = FormatConfig::default();
        assert_eq!(c.body_indent, 2);
        apply_file_locals(";;; x -*- lisp-body-indent: 4 -*-\n(foo)\n", &mut c);
        assert_eq!(c.body_indent, 4);

        // EditorConfig `indent_size` maps to body_indent (and, EditorConfig's
        // own rule, to tab_width when tab_width is unset).
        let (props, _) = editorconfig_props("[*.el]\nindent_size = 3\n", "foo.el");
        assert_eq!(props.indent_size, Some(3));
        assert_eq!(props.tab_width, Some(3));
    }
}
