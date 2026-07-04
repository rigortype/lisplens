//! Clojure indentation — a native Rust port of **cljfmt's** semantic
//! `:inner`/`:block` indent model (ADR-0039), the style the whole Clojure
//! ecosystem converged on (cljfmt, clojure-ts-mode, cljstyle, and clojure-mode's
//! modern spec format all share it). Emacs bundles no Clojure indenter, so — unlike
//! the Common Lisp and Scheme engines, which port an Emacs function validated
//! against Emacs — this engine targets **cljfmt** (`cljfmt fix`) as its oracle.
//!
//! Model (empirically pinned against cljfmt 0.16.4; see the ADR). To indent a code
//! line, let `c` be its innermost enclosing collection and `open` the column of
//! `c`'s opening delimiter. Value children are counted (comments/`#_`/metadata do
//! not consume a slot): in `(head a b …)`, `head` is position 0, `a` is **arg 0**.
//!
//! - **Collection literals** `[…]`, `{…}`, `#{…}` → align under the first element.
//! - **Round lists / `#(…)`** are always the *call* model:
//!   - **default** (no rule / non-symbol head): an arg on the head line → align
//!     every continuation under **arg 0**; head alone on its line → `open + 1`.
//!     Threading `->`/`->>` have no rule and use this.
//!   - **`[:inner 0]`** (`defn`, `fn`, …) → every direct child → `open + 2`.
//!   - **`[:inner D]`** / **`[:inner D idx]`** (`D ≥ 1`; `reify`, `letfn`, …) → a
//!     rule on the form `D+1` levels above the line's node gives `open + 2`
//!     (relative to the innermost `c`); `idx` restricts it to that ancestor's arg.
//!   - **`[:block N]`** (`let`/`when` = 1, `do`/`cond` = 0, `condp`/`catch` = 2):
//!     args `< N` are *special* and use **default**; args `≥ N` are *body* and get
//!     `open + 2` **only if the first body form (arg `N`) begins its own line**,
//!     else the whole form falls back to **default**. (This is where `:block`
//!     differs from Emacs's integer spec, which double-indents the special args.)
//!
//! A form may carry several rules (`defrecord` = `[:block 2] [:inner 1]`); each acts
//! at its own tree level. Unknown `def…`/`with-…` heads match cljfmt's regex
//! fallbacks → `[:inner 0]`. Like every engine here it only rewrites leading
//! whitespace, so it is always parse-safe.

use lispexp::{Datum, DatumKind, Delim, Dialect};

use super::Cols;

/// A cljfmt indent rule (a symbol maps to one or more).
#[derive(Clone, Copy)]
enum Rule {
    /// `[:inner D]` / `[:inner D idx]` — constant `+2` body indent for nodes `D+1`
    /// levels below the form, optionally only within the form's arg index `idx`.
    Inner { depth: usize, idx: Option<usize> },
    /// `[:block N]` — `N` special args, then body indent when the body breaks.
    Block(usize),
}

/// Max levels to search upward for an `:inner D` rule (cljfmt/clojure-mode cap).
const MAX_DEPTH: usize = 3;

/// Indent the code line starting at `offset`, given the whole `data` tree and the
/// innermost containing collection `c` (the driver's `container_at`). Returns the
/// target column; `body` is `lisp-body-indent` (2 by default). `fixed` selects the
/// Tonsky style (ADR-0040): every symbol-headed list body at a flat `+2`.
pub(super) fn indent(
    cols: &Cols,
    data: &[Datum],
    c: &Datum,
    offset: usize,
    body: usize,
    fixed: bool,
    dialect: Dialect,
) -> usize {
    let DatumKind::List { items, delim, .. } = &c.kind else {
        return 0;
    };
    let open_col = cols.col(c.span.start as usize);

    // Collection literals ([], {}, #{}) align under the first element.
    if !matches!(delim, Delim::Round) {
        return coll_indent(cols, items, c.span.start as usize, open_col, *delim);
    }

    // Round list (or the inner list of `#(…)`): the call model. `pos` is the number
    // of value children that begin before this line; the line's element is
    // `items[pos]` at arg index `pos - 1` (head is `items[0]`). A child is anchored
    // at its *form* start, past any leading `^metadata` prefix (which may sit on the
    // head line while the form itself wraps to this one). A `#_`-discarded form is
    // kept in the tree (`Options.keep_discarded`) and **counts as a value child** —
    // matching cljfmt, which walks every node: a discard in a body slot makes the
    // block degrade to default alignment exactly as a real form would, and a line
    // *inside* a multi-line discard indents against it via `container_at`.
    let pos = items
        .iter()
        .filter(|it| (form_start(it) as usize) < offset)
        .count();
    let arg_index = pos.wrapping_sub(1);

    // `:inner D` — a rule on an ancestor `D+1` levels above the node forces `+2`.
    // The container stack is outermost→innermost, with the innermost equal to `c`.
    let (stack, reader_cond) = container_stack(data, offset);

    // A reader conditional `#?(…)` / `#?@(…)` is data, not a call: its clauses align
    // under the first element like a collection (cljfmt). Same in both styles.
    if reader_cond {
        return coll_indent(cols, items, c.span.start as usize, open_col, Delim::Round);
    }

    // Fixed / Tonsky style: a symbol-headed list bodies at a flat `+2`, everything
    // else (a non-symbol head) uses the default alignment — no rule table, no
    // `:block`/`:inner` (cljfmt with `{:indents {#re ".*" [[:inner 0]]}}`).
    if fixed {
        return match items.first().and_then(head_symbol) {
            Some(_) => open_col + body,
            None => default_indent(cols, items, open_col),
        };
    }

    if inner_applies(&stack, offset, dialect) {
        return open_col + body;
    }

    // `:block N` on the head symbol, else default alignment.
    if let Some(head) = items.first().and_then(head_symbol) {
        if let Some(n) = rules_for(head, dialect).iter().find_map(|r| match r {
            Rule::Block(n) => Some(*n),
            Rule::Inner { .. } => None,
        }) {
            return block_indent(cols, items, open_col, offset, arg_index, n, body, dialect);
        }
    }
    default_indent(cols, items, open_col)
}

/// Whether an `:inner D` rule on some ancestor applies to the node beginning this
/// line — i.e. the node is `D+1` levels below a form whose head carries `[:inner D]`
/// (and, when the rule has an `idx`, the node descends through that form's `idx`-th
/// argument). `stack` is outermost→innermost; the node's immediate container is
/// `stack.last()`.
fn inner_applies(stack: &[&Datum], offset: usize, dialect: Dialect) -> bool {
    let n = stack.len();
    for depth in 0..MAX_DEPTH.min(n) {
        // The ancestor `depth` levels above the immediate container `stack[n-1]`.
        let ancestor = stack[n - 1 - depth];
        let DatumKind::List { items, .. } = &ancestor.kind else {
            continue;
        };
        let Some(head) = items.first().and_then(head_symbol) else {
            continue;
        };
        for rule in rules_for(head, dialect) {
            let Rule::Inner { depth: d, idx } = rule else {
                continue;
            };
            if *d != depth {
                continue;
            }
            match idx {
                None => return true,
                Some(want) => {
                    // The ancestor's child on the path down to the node is one level
                    // below `ancestor`, i.e. `stack[n - depth]`.
                    if n - depth < n {
                        let child = stack[n - depth];
                        if arg_index_in(items, child, offset) == Some(*want) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// The arg index (0-based, head excluded) of `child` within `parent`'s items, by
/// span. `offset` disambiguates the empty-container case; unused otherwise.
fn arg_index_in(items: &[Datum], child: &Datum, _offset: usize) -> Option<usize> {
    items
        .iter()
        .position(|it| it.span.start == child.span.start)
        .map(|p| p.saturating_sub(1))
}

/// `[:block N]`: whether the current line gets body indent (`open + body`) or the
/// default alignment turns on the **first body form** (arg `N`, i.e. `items[N + 1]`)
/// — if it begins its own line, the form is in body layout. The two dialects then
/// differ on the *special* args (`arg_index < N`): cljfmt keeps them on the default
/// alignment (open+1 when they wrap); **Phel** indents them at `open + body` too
/// (its `BlockIndenter` applies the inner indent to every child once the body
/// breaks). Reference: cljfmt `core.cljc`; Phel `BlockIndenter.php`.
#[allow(clippy::too_many_arguments)]
fn block_indent(
    cols: &Cols,
    items: &[Datum],
    open_col: usize,
    offset: usize,
    arg_index: usize,
    n: usize,
    body: usize,
    dialect: Dialect,
) -> usize {
    // cljfmt: a special arg always uses the default alignment. Phel does not
    // special-case them (once the body breaks, every child is body-indented).
    if !matches!(dialect, Dialect::Phel) && arg_index < n {
        return default_indent(cols, items, open_col);
    }
    match items.get(n + 1) {
        // The first body form isn't present before this line (we are it, or it is
        // this line) → body indent.
        None => open_col + body,
        Some(first_body) => {
            if (first_body.span.start as usize) >= offset || begins_line(cols, items, n + 1) {
                open_col + body
            } else {
                default_indent(cols, items, open_col)
            }
        }
    }
}

/// Default call alignment: align under arg 0 when its *form* sits on the head's
/// line, else `open + 1` (head alone on its line). The head-line test uses each
/// element's form start (past any `^metadata` prefix, which may sit on the head
/// line while the form wraps to the next); the alignment column, though, is the
/// element's true start — cljfmt aligns a continuation under the `^`, not the form.
fn default_indent(cols: &Cols, items: &[Datum], open_col: usize) -> usize {
    let head_line = cols.line_of(items[0].span.start as usize);
    if let Some(arg0) = items.get(1) {
        if cols.line_of(form_start(arg0) as usize) == head_line {
            return cols.col(arg0.span.start as usize);
        }
    }
    open_col + 1
}

/// The offset where a datum's *form* begins, skipping any leading `^metadata`
/// prefix(es) — `^Tag form` / `^{…} form` (which may stack, `^:private ^String x`).
/// Other reader prefixes (`'`, `` ` ``, `~`, …) are the form's own anchor and are
/// kept. Used to decide which line an argument's *form* is on, so a metadata
/// annotation on the head line does not make a wrapped argument look completed.
fn form_start(d: &Datum) -> u32 {
    match &d.kind {
        DatumKind::Prefixed {
            prefix: lispexp::Prefix::Meta,
            inner,
            ..
        } => form_start(inner),
        _ => d.span.start,
    }
}

/// Collection literal: align under the first element, or `open + delimiter-width`
/// when the opener is alone on its line.
fn coll_indent(
    cols: &Cols,
    items: &[Datum],
    open_offset: usize,
    open_col: usize,
    delim: Delim,
) -> usize {
    let width = match delim {
        Delim::Set => 2, // `#{`
        _ => 1,          // `[` `{`
    };
    match items.first() {
        Some(first) if cols.line_of(first.span.start as usize) == cols.line_of(open_offset) => {
            cols.col(first.span.start as usize)
        }
        _ => open_col + width,
    }
}

/// Whether `items[j]` begins its own physical line — its predecessor (`items[j-1]`,
/// or the container's opener for `j == 0`) ends on an earlier line.
fn begins_line(cols: &Cols, items: &[Datum], j: usize) -> bool {
    if j == 0 || j >= items.len() {
        return true;
    }
    let elem_line = cols.line_of(items[j].span.start as usize);
    let prev_end = (items[j - 1].span.end as usize).saturating_sub(1);
    elem_line > cols.line_of(prev_end)
}

/// The head symbol of a round list for rule lookup: namespace-stripped
/// (`clojure.core/when` → `when`), seeing through a `^metadata` prefix only — cljfmt
/// treats `(^:m foo …)` as a `foo`-headed (symbol) form. `None` for any other
/// non-symbol head, **including** a quote / var-quote / unquote one (`'foo`,
/// `#'foo`, `~foo`): cljfmt keys its rules (and the fixed-style `#re ".*"`) on the
/// bare symbol token, so those take the default alignment, not the symbol-head path.
/// (This is the one place the Clojure engine diverges from the Emacs engines, whose
/// `backward-prefix-chars` makes every prefix transparent.)
fn head_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(strip_ns(s)),
        DatumKind::Prefixed {
            prefix: lispexp::Prefix::Meta,
            inner,
            ..
        } => head_symbol(inner),
        _ => None,
    }
}

/// Strip a namespace/alias qualifier: the part after the last `/`, unless the
/// symbol *is* `/` (the division function).
fn strip_ns(s: &str) -> &str {
    match s.rfind('/') {
        Some(i) if s.len() > 1 => &s[i + 1..],
        _ => s,
    }
}

/// The container stack enclosing `offset`, outermost→innermost. Mirrors
/// `container_at`'s descent (through `#(…)` reader-macro lists and reader
/// prefixes), pushing each enclosing list; the last entry is the innermost `c`.
/// The container stack enclosing `offset` (outermost→innermost), plus whether the
/// innermost container is the inner list of a reader conditional `#?(…)` / `#?@(…)`.
fn container_stack<'a, 't>(data: &'a [Datum<'t>], offset: usize) -> (Vec<&'a Datum<'t>>, bool) {
    let mut stack = Vec::new();
    let mut reader_cond = false;
    push_containers(data, offset, false, &mut stack, &mut reader_cond);
    (stack, reader_cond)
}

fn push_containers<'a, 't>(
    data: &'a [Datum<'t>],
    offset: usize,
    wrapped_by_reader_cond: bool,
    stack: &mut Vec<&'a Datum<'t>>,
    reader_cond: &mut bool,
) {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            match &d.kind {
                DatumKind::List { items, .. } => {
                    stack.push(d);
                    // The innermost pushed list wins; its reader-cond-ness is whether
                    // its immediate wrapper was a `#?`/`#?@` HashLiteral.
                    *reader_cond = wrapped_by_reader_cond;
                    push_containers(items, offset, false, stack, reader_cond);
                }
                DatumKind::Prefixed {
                    prefix, inner, arg, ..
                } => {
                    // Metadata `^{…} form` holds the map in `arg`; descend it when it
                    // contains `offset` (else the applied form in `inner`).
                    if let Some(a) = arg {
                        if (a.span.start as usize) < offset && offset < (a.span.end as usize) {
                            push_containers(
                                std::slice::from_ref(a),
                                offset,
                                false,
                                stack,
                                reader_cond,
                            );
                            return;
                        }
                    }
                    // A reader conditional `#?(…)` / `#?@(…)` is a `Prefixed`; its
                    // inner list is data (align under the first element), not a call.
                    let is_rc = matches!(prefix, lispexp::Prefix::ReaderConditional { .. });
                    push_containers(
                        std::slice::from_ref(inner),
                        offset,
                        is_rc,
                        stack,
                        reader_cond,
                    );
                }
                DatumKind::HashLiteral {
                    inner: Some(inner), ..
                } => {
                    push_containers(
                        std::slice::from_ref(inner),
                        offset,
                        false,
                        stack,
                        reader_cond,
                    );
                }
                _ => {}
            }
            return;
        }
    }
}

/// The indent rules for `name`, per dialect: cljfmt's table for Clojure, Phel's own
/// table for Phel (ADR-0041). Both share the `:inner`/`:block` model and this
/// engine; only the symbol set differs.
fn rules_for(name: &str, dialect: Dialect) -> &'static [Rule] {
    match dialect {
        Dialect::Phel => phel_rules_for(name),
        _ => clojure_rules_for(name),
    }
}

/// cljfmt's default indent rules (`indents/clojure.clj` + the merged `compojure.clj`
/// and `fuzzy.clj`), verbatim. Returns the rule(s) for `name`, or `&[]` for a plain
/// function call — with the `def…`/`with-…` regex fallbacks applied. Names are
/// matched bare (namespace already stripped by [`head_symbol`]).
fn clojure_rules_for(name: &str) -> &'static [Rule] {
    use Rule::{Block, Inner};
    const B0: &[Rule] = &[Block(0)];
    const B1: &[Rule] = &[Block(1)];
    const B2: &[Rule] = &[Block(2)];
    const I0: &[Rule] = &[Inner {
        depth: 0,
        idx: None,
    }];
    match name {
        // --- [:inner 0] (defn-like) ---
        "def" | "defn" | "defn-" | "defmacro" | "defmethod" | "defmulti" | "defonce"
        | "deftest" | "fn" | "bound-fn" | "fdef" | "use-fixtures"
        // compojure route handlers (cljfmt merges compojure.clj by default)
        | "ANY" | "DELETE" | "GET" | "HEAD" | "OPTIONS" | "PATCH" | "POST" | "PUT"
        | "context" | "defroutes" | "rfn" => I0,

        // --- [:block 0] ---
        "alt!" | "alt!!" | "comment" | "cond" | "delay" | "do" | "finally" | "future"
        | "go" | "thread" | "try" | "with-out-str" => B0,

        // --- [:block 1] ---
        "binding" | "case" | "cond->" | "cond->>" | "defstruct" | "doseq" | "dotimes"
        | "doto" | "extend" | "for" | "go-loop" | "if" | "if-let" | "if-not" | "if-some"
        | "let" | "let*" | "locking" | "loop" | "match" | "ns" | "struct-map" | "testing"
        | "when" | "when-first" | "when-let" | "when-not" | "when-some" | "while"
        | "with-local-vars" | "with-open" | "with-precision" | "with-redefs"
        | "let-routes" => B1,

        // --- [:block 2] ---
        "are" | "as->" | "catch" | "condp" => B2,

        // --- multi-rule ([:block …] + [:inner 1]) ---
        "defprotocol" | "extend-protocol" | "extend-type" => {
            &[Block(1), Inner { depth: 1, idx: None }]
        }
        "defrecord" | "deftype" | "proxy" => &[Block(2), Inner { depth: 1, idx: None }],
        "reify" => &[Inner { depth: 0, idx: None }, Inner { depth: 1, idx: None }],
        "letfn" => &[Block(1), Inner { depth: 2, idx: Some(0) }],

        // --- regex fallbacks (fuzzy.clj) ---
        _ if is_def_like(name) || name.starts_with("with-") => I0,
        _ => &[],
    }
}

/// Phel's indent table (`phel format`, `FormatterFactory::{INNER,BLOCK}_INDENT_SYMBOLS`,
/// ADR-0041), verbatim. Phel only uses `[:inner 0]` and `[:block N]` — no nested
/// `:inner`, no regex fallback, no reader conditionals — so it is a strict subset of
/// the shared model.
fn phel_rules_for(name: &str) -> &'static [Rule] {
    use Rule::{Block, Inner};
    const B0: &[Rule] = &[Block(0)];
    const B1: &[Rule] = &[Block(1)];
    const B2: &[Rule] = &[Block(2)];
    const I0: &[Rule] = &[Inner {
        depth: 0,
        idx: None,
    }];
    match name {
        // INNER_INDENT_SYMBOLS
        "def" | "def-" | "defn" | "defn-" | "defmacro" | "defmacro-" | "deftest" | "fn"
        | "defstruct" | "defrecord" | "definterface" | "defexception" | "defenum"
        | "defprotocol" | "defmulti" | "defmethod" | "defonce" | "reify" => I0,

        // BLOCK_INDENT_SYMBOLS => 0
        "do" | "cond" | "try" | "finally" | "with-output-buffer" | "delay" | "lazy-seq" => B0,

        // BLOCK_INDENT_SYMBOLS => 1
        "if" | "if-not" | "foreach" | "for" | "dofor" | "let" | "ns" | "loop" | "case" | "when"
        | "when-not" | "when-let" | "when-some" | "if-let" | "if-some" | "binding"
        | "when-first" | "doseq" | "dotimes" | "letfn" | "with-redefs" | "with-bindings"
        | "extend-type" | "extend-protocol" => B1,

        // BLOCK_INDENT_SYMBOLS => 2
        "catch" | "condp" => B2,

        _ => &[],
    }
}

/// cljfmt's `^def(?!ault)(?!late)(?!er)` fallback: a `def…` head, but not `default`,
/// `deflate`, or `defer` (which are ordinary function calls).
fn is_def_like(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("def") else {
        return false;
    };
    !rest.starts_with("ault") && !rest.starts_with("late") && !rest.starts_with("er")
}

#[cfg(test)]
mod tests {
    use crate::config::FormatConfig;
    use lispexp::Dialect;

    /// Reindent flat Clojure and compare to cljfmt's canonical output (captured
    /// from `cljfmt fix` 0.16.4 with only `:indentation?` enabled — the oracle).
    fn fmt(input: &str) -> String {
        crate::format::format(input, &FormatConfig::default(), Dialect::Clojure)
    }

    /// Reindent in the fixed / Tonsky style (ADR-0040), the oracle being `cljfmt fix
    /// --config {:indents {#re ".*" [[:inner 0]]}}`.
    fn fmt_fixed(input: &str) -> String {
        let config = FormatConfig {
            clojure_fixed_indent: true,
            ..FormatConfig::default()
        };
        crate::format::format(input, &config, Dialect::Clojure)
    }

    /// Reindent Phel, whose engine shares this model with a Phel-specific table
    /// (ADR-0041); the oracle is `phel format`.
    fn fmt_phel(input: &str) -> String {
        crate::format::format(input, &FormatConfig::default(), Dialect::Phel)
    }

    #[test]
    fn block_and_inner_bodies() {
        // when/let (:block 1), do/cond (:block 0), defn/fn (:inner 0).
        let input = "\
(defn f [x]
(when x
(g x)
(h x)))
(let [a 1
b 2]
(+ a b))
(do
(a)
(b))
(cond
p 1
q 2)";
        let want = "\
(defn f [x]
  (when x
    (g x)
    (h x)))
(let [a 1
      b 2]
  (+ a b))
(do
  (a)
  (b))
(cond
  p 1
  q 2)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn block_special_args_and_fallback() {
        // :block 1 with the body form on the head line falls back to arg-0 align;
        // condp (:block 2) keeps two special args; head-alone default is +1.
        let input = "\
(when x y
z)
(condp = x
1 :a
2 :b)
(foo
bar
baz)
(foo bar
baz)";
        let want = "\
(when x y
      z)
(condp = x
  1 :a
  2 :b)
(foo
 bar
 baz)
(foo bar
     baz)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn threading_and_collections() {
        // Threading macros use the default (align under the first arg); vectors,
        // maps and sets align under the first element.
        let input = "\
(-> x
(a)
(b))
(->> x
(a))
[1
2]
{:a 1
:b 2}
#{a
b}";
        let want = "\
(-> x
    (a)
    (b))
(->> x
     (a))
[1
 2]
{:a 1
 :b 2}
#{a
  b}";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn nested_inner_rules() {
        // reify (:inner 0 + :inner 1), defrecord (:block 2 + :inner 1), and letfn
        // (:inner 2 0 — fn bodies inside the binding vector).
        let input = "\
(reify P
(f [_]
body))
(defrecord R [a]
P
(m [_]
x))
(letfn [(g [x]
y)]
body)";
        let want = "\
(reify P
  (f [_]
    body))
(defrecord R [a]
  P
  (m [_]
    x))
(letfn [(g [x]
          y)]
  body)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn namespaced_heads_def_fallback_and_reader_conditional() {
        // Namespace-stripped lookup, the def…/with-… regex fallback, `#(…)` as a
        // call, and `#?(…)` reader conditionals aligning under the first clause.
        let input = "\
(clojure.core/when x
y)
(defthing a
b)
(map #(+ % 1)
coll)
#?(:clj a
:cljs b)";
        let want = "\
(clojure.core/when x
  y)
(defthing a
  b)
(map #(+ % 1)
     coll)
#?(:clj a
   :cljs b)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn metadata_map_and_docstring_interior() {
        // A `^{…}` metadata map aligns its keys under the first key (the map is in
        // the `Prefixed` node's `arg`); a multi-line docstring *inside* the metadata
        // stays untouched string interior. Regression for the real-corpus finds.
        let input = "\
(def ^{:doc \"one
  See: x.\"
:deprecated \"1.1\"}
name 1)";
        let want = "\
(def ^{:doc \"one
  See: x.\"
       :deprecated \"1.1\"}
  name 1)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn metadata_prefix_on_args_and_heads() {
        // A metadata-annotated argument whose *form* wraps to the next line is
        // located by its form, not the `^` — so `(-write x)` is doto's special arg 0
        // (open+1), `(.close)` its body (+2). A `^meta` head is transparent for rule
        // lookup (`^:m when` uses when's `:block 1`), but a var-quote head `#'foo` is
        // not a symbol head (default alignment). Regressions from the real corpora.
        let input = "\
(doto ^Foo
(-write x)
(.close))
(^:m when x
y)
(#'foo x
y)";
        let want = "\
(doto ^Foo
 (-write x)
  (.close))
(^:m when x
  y)
(#'foo x
       y)";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn discarded_form_interior_indents_against_the_discard() {
        // With `keep_discarded` the reader keeps a `#_`-discarded form in the tree
        // (as `Prefixed { Discard, … }`), so lines *inside* a multi-line discard
        // indent against the discarded collection, not the enclosing container —
        // matching cljfmt (which keeps every node). Regression for feedback 0003.
        let input = "\
[a
#_[\"/spec\" {:c d}
[\"/x\" {:p 1}]]
b]";
        let want = "\
[a
 #_[\"/spec\" {:c d}
    [\"/x\" {:p 1}]]
 b]";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn discard_in_a_body_slot_counts_like_a_real_form() {
        // A kept `#_` discard counts as a value child for the call/block model —
        // cljfmt walks every node. Here `#_lazy` sits in `if`'s (`:block 1`) body
        // slot on the head line, so the block degrades to default alignment (under
        // the condition `true`), just as a real form on the head line would. This is
        // the common real-world shape (a `#_` commenting out a form) — validated
        // byte-exact vs cljfmt on the malli/reitit corpora. Regression for 0003.
        let input = "\
(if true #_lazy
(a)
(b))";
        let want = "\
(if true #_lazy
    (a)
    (b))";
        assert_eq!(fmt(input), want, "\n{}", fmt(input));
    }

    #[test]
    fn already_formatted_is_a_fixed_point() {
        let input = "\
(defn process [input]
  (let [x (parse input)]
    (->> x
         (map inc)
         (filter pos?))))";
        assert_eq!(fmt(input), input);
    }

    #[test]
    fn fixed_tonsky_style() {
        // Every symbol-headed list body is a flat +2 (no rule table, no
        // align-under-arg for calls, threading included); collections align under
        // the first element; a non-symbol head still uses the default alignment;
        // `#(…)` bodies at +2. Golden from `cljfmt fix` with the Tonsky config.
        let input = "\
(defn f [x]
(when x
(foo bar
baz)
(-> a (b)
(c))))
[1
2]
((g) x
y)
#(foo %
%2)";
        let want = "\
(defn f [x]
  (when x
    (foo bar
      baz)
    (-> a (b)
      (c))))
[1
 2]
((g) x
     y)
#(foo %
   %2)";
        assert_eq!(fmt_fixed(input), want, "\n{}", fmt_fixed(input));
    }

    #[test]
    fn phel_indentation() {
        // Phel uses the same engine with its own table (ADR-0041): `defn`/`defstruct`
        // are `:inner 0`, `when`/`foreach`/`condp` are `:block`. Unlike cljfmt, a
        // block form's *special* args also get the +2 body indent once the body
        // breaks (`when-not`, `condp` head-alone). Goldens from `phel format`.
        let input = "\
(defn f [x]
(when x
(foo)
(bar)))
(foreach [a xs]
(println a))
(when-not
(test)
(body))
(condp
=
x
y)
(defstruct Point [x y])";
        let want = "\
(defn f [x]
  (when x
    (foo)
    (bar)))
(foreach [a xs]
  (println a))
(when-not
  (test)
  (body))
(condp
  =
  x
  y)
(defstruct Point [x y])";
        assert_eq!(fmt_phel(input), want, "\n{}", fmt_phel(input));
    }

    #[test]
    fn phel_reader_constructs_indent_cleanly() {
        // Phel-specific reader forms fixed upstream in lispexp 0.7 (feedback
        // 0004/0005/0006): a `|(…)` short anonymous function, PHP fully-qualified
        // names (`\RuntimeException`, `\Phel\Lang\Symbol/create`), and a symbol with
        // an interior `;` (`'*_.%;!:+-?`). Each now reads as one structurally-correct
        // form, so a call's arg count — and thus its body indent — is right. Golden
        // captured byte-exact from `phel format`.
        let input = "\
(defn f [x]
(map |(inc $) x)
(php/new \\RuntimeException \"boom\")
(\\Phel\\Lang\\Symbol/create \"s\")
(def sym '*_.%;!:+-?))";
        let want = "\
(defn f [x]
  (map |(inc $) x)
  (php/new \\RuntimeException \"boom\")
  (\\Phel\\Lang\\Symbol/create \"s\")
  (def sym '*_.%;!:+-?))";
        assert_eq!(fmt_phel(input), want, "\n{}", fmt_phel(input));
    }

    #[test]
    fn fixed_prefixed_heads() {
        // In fixed style a `^meta` head still counts as a symbol head (+2), but a
        // var-quote head `#'foo` does not — it takes the default alignment (cljfmt
        // keys `#re ".*"` on the bare symbol token).
        let input = "\
(^:m foo x
y)
(#'foo x
y)";
        let want = "\
(^:m foo x
  y)
(#'foo x
       y)";
        assert_eq!(fmt_fixed(input), want, "\n{}", fmt_fixed(input));
    }
}
