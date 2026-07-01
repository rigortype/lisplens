# Library / dependency loading across dialects — survey

Investigation behind ADR-0004's open question and resolved in ADR-0014: what a static, non-evaluating editor can infer about dependency loading, and what it cannot. Surveyed Common Lisp, Scheme, Clojure, and Emacs Lisp.

## Universal pattern

Every dialect splits cleanly into two layers:

- **Declared, name-based dependency graph (in-repo, static).** `import` / `require` / `use-modules` / `:require` / `:depends-on` / `Package-Requires` name *other units by name*, never by filesystem location. These edges, plus each file's own declared name (`define-library`, `defpackage`, `ns`, `provide`), are parseable without evaluation.
- **Name → filesystem/version resolution (out-of-repo, runtime).** Turning a name into a concrete file requires a set of **search roots**, an **implementation choice**, and (for external deps) the **installed package layout** — all of which live in env vars, CLI flags, init files, and per-machine install dirs, never in the source being edited.

The intra-repo name→file mapping *is* statically inferable, given which directories are source roots (usually readable from the project's own manifest or from matching declared names against the directory layout).

## Per-dialect

**Common Lisp.** System layer (`asdf:defsystem :depends-on`, `ql:quickload`, `require`) vs package layer (`defpackage :use`/`:import-from`). In-repo: `.asd` `:depends-on`/`:components`, `defpackage`, `in-package`, `#+/#-` conditionals. Not inferable: ASDF source-registry (`CL_SOURCE_REGISTRY`, `source-registry.conf`, `*central-registry*`), Quicklisp local-projects/dists, chosen implementation, dependency versions, package↔system mapping when names differ.

**Scheme.** R6RS `library`/`import`, R7RS `define-library`/`import`/`include`, Guile `use-modules`, Racket `require`+`#lang`, Gauche dotted `use`. In-repo: `define-library` names + import edges, relative `include`, extensions as implementation hints, Racket `info.rkt`. Not inferable: load-path roots (`GUILE_LOAD_PATH`, `CHEZSCHEMELIBDIRS`, `CHIBI_MODULE_PATH`, `GAUCHE_LOAD_PATH`, Racket collection paths + `links.rktd`), implementation, R6RS-vs-R7RS mode, which installed library an external import resolves to.

**Clojure.** `ns :require` / `require` name namespaces; the classpath resolves them via deterministic munging (dots→slashes, hyphens→underscores). In-repo: `deps.edn`/`project.clj` (`:paths`, `:deps`), `ns` forms, namespace↔path layout. Not inferable: actual classpath roots, active alias/profile, external dep resolution (Maven/git/local → `~/.m2`/checkout SHA), CLJ-vs-CLJS target for `.cljc`, runtime/dynamic `require`.

**Emacs Lisp.** `require`/`provide` by feature symbol, `load` by file string, `autoload` symbol→file. In-repo: `;;; Package-Requires:` header, `provide`/`require`/`autoload` forms, `foo.el`⇒feature `foo` convention. Not inferable: the entire `load-path` (set in init/env/CLI, never in the library), where external packages are installed (elpa/straight build dirs), installed versions, Emacs version + built-in feature set, generated `*-autoloads.el`.

## The irreducible tuple

What is genuinely not inferable and *affects editing* — the only candidates for the Project profile:

1. **Implementation** (when extension + content is ambiguous): scheme → {guile, chez, racket, chibi, gauche}; cl → {sbcl, ccl, …}; clojure target → {clj, cljs}; elisp Emacs-version baseline (optional).
2. **Dialect / standard mode** when ambiguous: R6RS vs R7RS.
3. **Source roots override**: only when not inferable from the project's own config or directory layout.
4. **(Advanced, optional)** extra in-tree search roots; active build variant (Clojure alias/profile, Scheme `cond-expand` features).

External dependency resolution (name → installed version/location) is deliberately **out of scope** — see ADR-0014.

## Development tooling manifests (repo-local — widen zero-config)

Real projects carry dev-tooling manifests at the repo root that move deps and source roots (and, for Emacs, the version baseline) into the statically inferable set. Several are themselves S-expressions, so lisplens parses them with its own backend.

**Emacs Lisp.** Primary, always-parseable: the main file's `Package-Requires` header (and/or `foo-pkg.el` `define-package`) → dependency names + min-versions **and** the Emacs baseline (`(emacs "X.Y")`). Build-tool files layer on top:

- `Cask` — declarative S-expr DSL: `(source …)` archives, `(package …)` / `(package-file …)`, `(depends-on "name" "ver")`, `(development …)`, `(files …)` globs. Follow `package-file` / `package-descriptor` to the real header.
- `Eask` — mirrors Cask (`(package …)`, `(source …)`, `(depends-on …)`, `(files …)`, `(script …)`); modern, CI-oriented.
- `Eldev` — arbitrary Elisp; recognize only well-known top-level `eldev-*` calls (`eldev-use-package-archive`, `eldev-add-loading-roots`, `eldev-project-source-dirs`, `eldev-add-extra-dependencies`). Core deps still come from the `.el` header.
- `makem.sh` — no manifest; discovers from git + `Package-Requires` / `-pkg.el` / `Cask`.
- Installed-dep dirs `.cask/` `.eask/` `.eldev/` at root → add to load-path when resolving deps.

**Common Lisp.**

- `qlfile` / `qlfile.lock` (qlot) — the strongest static in-repo dependency signal: declared external deps + source kind (`ql` / `github` / `git` / `local` / `http`) and exact pins in the lockfile. `local` lines add source-registry roots. `.qlot/` presence marks a qlot project (its contents are git-ignored/external, but fully declared by these inputs).
- `clpmfile` / `clpmfile.lock` (CLPM) — same category as qlot, declarative S-expr directives read-parseable without evaluation: `(:source …)`, `(:system …)` / `(:project … :version …)`, VCS `(:github … :branch/:tag/:commit)`, and `(:asd "…")` local system roots; the lockfile holds the resolved recursive tree + exact pins. Unlike qlot, installed deps live in a global cache (`~/.local/share/clpm/`), not in-repo — only the two manifests are local.
- `.asd` files — system names + `:depends-on` + `:components` / `:pathname` = system/source roots.
- Roswell `.ros` scripts — polyglot shell+Lisp; the `exec ros … -L <impl>` line in the `#|…|#` block is the only repo-local *implementation* hint, and only per-script. Implementation choice is otherwise global (`ros use`, `~/.roswell/`), not in-repo.

**Scheme.**

- `Akku.manifest` / `Akku.lock` (Akku.scm) — the de-facto general Scheme package manager. The manifest is a plain, `read`-parseable R6RS S-expression: `(akku-package (NAME VERSION) … (depends (NAME RANGE)…) (depends* …dev…))`, dep names being strings or library-name lists, ranges npm-style SemVer. The lockfile holds the transitive closure + git/tarball pins + checksums. Vendored deps install to repo-local `.akku/lib` (load path revealed by `.akku/env`). Target implementation is **not** declared — Akku is intentionally multi-implementation (ships an R7RS→R6RS translator).
- `define-library` / `library` name declarations + `info.rkt` (Racket) + file extensions (`.sld` ⇒ R7RS, `.sls`/`.sps` ⇒ R6RS) and `#!r6rs`/`#!r7rs` directives — the per-file dialect signal (as in the base survey).

**Racket.**

- `info.rkt` (`#lang info`) — declares `deps` / `build-deps` (package-level dependency names) and `collection` (`'multi` ⇒ each top-level subdir is a collection root; a string / pkg-name ⇒ the package dir is one collection), plus `compile-omit-paths` / `test-omit-paths`. **Caveat:** `#lang info` is a *restricted* language, not plain data — field values may be `(if …)` / `string-append` / `getenv` expressions, so a static tool must interpret the constrained grammar (safely sandboxable), not merely `read` it. Cross-package and stdlib resolution needs external `links.rktd` + the `collects` tree (`raco pkg install` writes links by reference, in user/installation scope — outside the repo).

**Takeaway.** Scanning these manifests brings deps + pins + source roots (Emacs: + version baseline) firmly into zero-config across every ecosystem's package managers. **No package manager declares the target implementation** — Akku is deliberately multi-implementation, qlot/CLPM are implementation-agnostic, Roswell keeps it global (`ros use`), Racket's `#lang` may be external, Clojure's clj/cljs comes from build config. So **implementation / target choice is the universal irreducible signal**, and remains the primary thing the Project profile fills (a secondary hint may come from CI files or a per-script `.ros -L`).

**Parsing caveats.** Most manifests are plain S-expressions, `read`-safe by matching known directive heads and ignoring unknown/dynamic forms: `Akku.manifest`/`Akku.lock`, `clpmfile`/`clpmfile.lock`, `.asd`, `Cask`/`Eask`, `qlfile` (line-based). Two need more than `read`: **`info.rkt`** (`#lang info` restricted grammar with `if`/`string-append`/`getenv` — interpret in a sandbox) and **`Eldev`** (arbitrary Elisp — recognize only well-known `eldev-*` calls). Since most of these are themselves S-expressions, lisplens can parse them with its own backend.

## Sources

Common Lisp: ASDF manual & source-registry docs, Quicklisp local-projects, CL Cookbook (packages). Scheme: R6RS/R7RS specs, Guile/Chez/Racket/Chibi/Gauche manuals. Clojure: clojure.org libs & deps_edn, Leiningen PROFILES, shadow-cljs, clojurescript.org. Emacs Lisp: GNU Emacs Lisp Reference (Library Search, Named Features), use-package, straight.el.
