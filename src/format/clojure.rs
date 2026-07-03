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

use lispexp::{Datum, DatumKind, Delim};

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
/// target column; `body` is `lisp-body-indent` (2 by default).
pub(super) fn indent(cols: &Cols, data: &[Datum], c: &Datum, offset: usize, body: usize) -> usize {
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
    // `items[pos]` at arg index `pos - 1` (head is `items[0]`).
    let pos = items
        .iter()
        .filter(|it| (it.span.start as usize) < offset)
        .count();
    let arg_index = pos.wrapping_sub(1);

    // `:inner D` — a rule on an ancestor `D+1` levels above the node forces `+2`.
    // The container stack is outermost→innermost, with the innermost equal to `c`.
    let (stack, reader_cond) = container_stack(data, offset);

    // A reader conditional `#?(…)` / `#?@(…)` is data, not a call: its clauses align
    // under the first element like a collection (cljfmt).
    if reader_cond {
        return coll_indent(cols, items, c.span.start as usize, open_col, Delim::Round);
    }

    if inner_applies(&stack, offset) {
        return open_col + body;
    }

    // `:block N` on the head symbol, else default alignment.
    if let Some(head) = items.first().and_then(head_symbol) {
        if let Some(n) = rules_for(head).iter().find_map(|r| match r {
            Rule::Block(n) => Some(*n),
            Rule::Inner { .. } => None,
        }) {
            return block_indent(cols, items, open_col, offset, arg_index, n, body);
        }
    }
    default_indent(cols, items, open_col)
}

/// Whether an `:inner D` rule on some ancestor applies to the node beginning this
/// line — i.e. the node is `D+1` levels below a form whose head carries `[:inner D]`
/// (and, when the rule has an `idx`, the node descends through that form's `idx`-th
/// argument). `stack` is outermost→innermost; the node's immediate container is
/// `stack.last()`.
fn inner_applies(stack: &[&Datum], offset: usize) -> bool {
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
        for rule in rules_for(head) {
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

/// `[:block N]`: args `< N` use default; body args `≥ N` get `open + body` when the
/// first body form (arg `N`, i.e. `items[N + 1]`) begins its own line, else default.
fn block_indent(
    cols: &Cols,
    items: &[Datum],
    open_col: usize,
    offset: usize,
    arg_index: usize,
    n: usize,
    body: usize,
) -> usize {
    if arg_index < n {
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

/// Default call alignment: align under arg 0 when it sits on the head's line, else
/// `open + 1` (head alone on its line).
fn default_indent(cols: &Cols, items: &[Datum], open_col: usize) -> usize {
    let head = &items[0];
    if let Some(arg0) = items.get(1) {
        if cols.line_of(arg0.span.start as usize) == cols.line_of(head.span.start as usize) {
            return cols.col(arg0.span.start as usize);
        }
    }
    open_col + 1
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
/// (`clojure.core/when` → `when`) and seeing through reader prefixes. `None` for a
/// non-symbol head (which uses the default alignment).
fn head_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(strip_ns(s)),
        DatumKind::Prefixed { inner, .. } => head_symbol(inner),
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
                DatumKind::Prefixed { prefix, inner, .. } => {
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

/// cljfmt's default indent rules (`indents/clojure.clj` + the merged `compojure.clj`
/// and `fuzzy.clj`), verbatim. Returns the rule(s) for `name`, or `&[]` for a plain
/// function call — with the `def…`/`with-…` regex fallbacks applied. Names are
/// matched bare (namespace already stripped by [`head_symbol`]).
fn rules_for(name: &str) -> &'static [Rule] {
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
    fn already_formatted_is_a_fixed_point() {
        let input = "\
(defn process [input]
  (let [x (parse input)]
    (->> x
         (map inc)
         (filter pos?))))";
        assert_eq!(fmt(input), input);
    }
}
