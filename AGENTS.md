# plannotator-tui — engineering notes

A small open-source terminal application. Optimize for a reader who has never seen the
code: clear module boundaries, boring Rust, no cleverness that needs a comment to defend it.

## Shape

```
crates/plannotator-tui-schema   annotation + anchor types and resolvers. Wire-exact with
                          Plannotator Workspaces. No I/O, no UI, no async. Pure functions.
crates/plannotator-tui          the app: parse → layout → draw → input. Talks to the schema crate,
                          never the other way round. Network clients live in their own
                          modules behind a trait so the app has one seam per external system.
                          There is no file tree: documents arrive as a set (`src/docs.rs`) and
                          show one tab each. It is a tool agents present to, not one to browse.
herdr/                    the Herdr plugin manifest. The launcher it runs is
                          `plannotator-tui herdr open` (src/herdr/); no shell logic here.
```

Markdown parsing is `pulldown-cmark`; rendering to styled text is `tui-markdown`. We never
interpret markdown ourselves. Anything that needs to know "what is a heading" is a bug.

`src/table.rs` is the one place that lays something out instead of `tui-markdown`: a table's
columns are sized from content there with no notion of the pane, and `Options` has no width knob,
so a wide table could only be clipped. Cells still come from `pulldown-cmark`'s event stream with
source ranges - this is a layout, not a second parser. Widths are display widths, and every
rendered character keeps its source byte. Which map is indexed by what is spelled out below, and
getting it wrong is how a shipped bug survived a green test. Decision 20.

`src/theme.rs` owns every color, one token per meaning; nothing else may name a color. `src/render.rs`
carries what rendering needs from outside the layout (theme, art settings, the document's directory),
so `layout` and `art` share it without importing each other. See `docs/decisions.md` 17.

`src/art/` is the exception to "one seam per external system" being in-process: a Mermaid fence
is laid out by `grok-mermaid` under Node, spawned once per document with an embedded bridge
script. It must never be able to fail a document — no Node, no package, a rejected diagram and
a hang all fall back to the plain code block. Detection still goes through `pulldown-cmark`
(the fence's info string, the image destination), never string matching on markup. See
`docs/decisions.md` 15 and 16.

`src/art/obsidian.rs` is the **only** sanctioned exception to "we never interpret markdown
ourselves", because `![[x.png]]` is not markdown and `pulldown-cmark` will never report it. It is
off by default and must stay that way. Any other hand-parsing of markup is still a bug. Decision 18.

`src/git.rs` is the only place that runs git for content (`workspace_paths.rs` runs it for the repo
name). Its parser is pure and its runner is infallible: no repository, no `HEAD`, or a git that will
not run means no bars, never an error. A document must render with git absent. Decision 24.

`src/app/help.rs` holds `KEYS`, the one description of every binding. The overlay and the footer
hint both render from it, and a test walks `input.rs`, `compose.rs`, `pick.rs` and `help.rs` to fail
the build when a key is bound without being described. Never hand-write a key list elsewhere: the
footer used to be prose, and it advertised `t hide` for a tree that had been deleted. Decision 25.

Two source maps exist and they are indexed differently. `LineOffsets` is per rendered **char**,
because `cells_of` pulls one entry per char; `Row.cells` is per display **column**, because
hit-testing takes a column. Producing one where the other is expected mis-anchors every character
after a wide one, cumulatively, and the test that hid it asserted a length. Assert the behaviour: the
byte under a column belongs to the character drawn at that column. Decision 20.

`docs/architecture.md` is the map: what the app is, how a document becomes a frame, and which module
owns what. This file is the rules; that one is the shape.

## Rules that are enforced (see `Cargo.toml` workspace lints)

- `unsafe` is forbidden. `unwrap`/`expect`/`panic!`/`todo!` warn — production code returns
  errors or handles the `None`. Indexing warns: use `.get()`, iterators, or slice patterns.
- `println!`/`eprintln!` warn outside the CLI entry points; those opt out locally with a
  one-line `#[allow]` and a reason.
- `cargo fmt --all` and `cargo clippy --workspace --all-targets` clean before commit.

## Rules that are judgement

- **Files stay small.** A module over ~300 lines is a signal to split by responsibility, not
  to add a second `impl` block. Tests live next to the code they test; when a test module
  outgrows the code, move it to `tests/` for that crate.
- **Tests are for behaviour that can regress.** One test per invariant, named for the
  invariant. No tests that restate the implementation, no mocks of our own types, no test
  helpers that need their own tests. Prefer a real temp dir over a fake filesystem, a real
  `TestBackend` over a fake terminal, real fixtures over builders.
- **Performance is structural, not micro.** Render once per block and cache; wrap on
  resize; touch only visible rows per frame; never reparse a whole document on an edit.
  Measure with `plannotator-tui --bench` before and after anything that changes those paths.
- **Errors carry context, not stack traces.** `anyhow` at the app boundary, typed errors in
  the schema crate. A user-facing failure names the file and what was being done.
- **No abstraction without a second caller.** No traits for one impl, no generics for one
  type, no builders for structs with four fields. A trait is justified only at a seam to an
  external system (Workspaces, Herdr, the clipboard) where a test needs a stand-in.
- **Ownership over cleverness.** Owned `String`s in long-lived structs; borrow in function
  signatures. Lifetimes in public types need a reason.
- **Commits are small and describe intent**, lowercase conventional style:
  `feat(schema): anchor resolution by rendered quote`.

## Data contract

Annotations are the Workspaces wire shape. The anchor object is opaque to the server; the
web client reads `originalText` (rendered text). Our fields ride alongside. See
`crates/plannotator-tui-schema/src/lib.rs` for the source of truth; do not redefine these types
elsewhere.
