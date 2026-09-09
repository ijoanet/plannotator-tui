# Architecture

What the fork is, how a document becomes pixels, and where every module sits. Read `AGENTS.md`
first for the rules this obeys, and `docs/decisions.md` for why each choice was made rather than
its alternative. This file is the map; that one is the argument.

## What it is

A terminal Markdown reviewer. An **agent writes a document and presents it**; a human reads it,
marks it up, and the marks travel back to that agent as its next message. Everything below follows
from that one sentence.

Two consequences worth stating early, because they explain choices that look odd otherwise:

- **It is not a browser.** There is no file tree. Documents arrive as a *set* and show one tab each.
  Nothing exists to go looking for a file with (decision 22).
- **Rendering is for judgement, not display.** A diagram or table that is clipped, or a command
  truncated to look complete, can change what a reviewer approves. So the renderer would rather
  reshape a block than lose part of it (decisions 20, 23).

## The pipeline

```
source ─→ doc.rs ─────→ layout.rs ──────────→ app/draw.rs ─→ terminal
          split into    render each block      compose rows
          blocks        once, wrap to width    into a frame
                             │
                             ├─ art/      mermaid, images, Obsidian embeds
                             ├─ table.rs  tables laid out here
                             ├─ code.rs   code blocks laid out here
                             └─ srcmap.rs every rendered char → its source byte
```

`doc.rs` walks `pulldown-cmark` at depth zero and records byte ranges. Everything downstream is
keyed by **block index and source byte**, which is what makes a selection quotable.

`layout.rs` renders each block once (expensive, cached) and wraps to the current width (cheap, redone
on resize). Per frame only visible rows are touched.

### The source map, and the invariant that bit twice

Two maps exist and they are indexed differently. Getting them confused shipped a bug that a green
test hid, twice:

| Map | Indexed by | Used for |
|---|---|---|
| `LineOffsets` (`srcmap.rs`) | rendered **char** | which source byte a character came from |
| `Row.cells` (`wrap.rs`) | display **column** | hit-testing a click, painting a selection |

Producing one where the other is expected mis-anchors every character after a wide one,
cumulatively. Assert the behaviour: *the byte under a column belongs to the character drawn at that
column* (decision 20).

## Blocks that are not text

Three kinds of block are laid out here instead of by `tui-markdown`, each for the same reason: the
library sizes from content with no notion of the pane, so a wide block could only be clipped.

- **`art/`** — a ```` ```mermaid ```` fence becomes box-drawing art (via `grok-mermaid` under Node,
  one process per document), an image becomes truecolor half-blocks, and `![[x.png]]` is parsed by
  hand because it is not Markdown. Art has **no per-character source map**: the picture stands for
  the block, so a comment on it quotes the block's source (decisions 15, 16, 18).
- **`table.rs`** — columns shrink until the row fits, then cells wrap. Below `MIN_COLUMN` per column
  it becomes one labelled record per row, the way `psql \x` expands a wide result (decision 20).
- **`code.rs`** — fences hidden, language labelled, long lines wrapped at the column edge with a
  continuation marker so a copied command is whole. `code/highlight.rs` maps syntect scopes onto
  nine theme tokens; HCL and TOML are vendored because syntect ships without them (decision 23).

## State

`app/mod.rs` owns `App` and the operations that mutate it. `Open` is the open document: its source,
`Document`, `DocLayout` and `Store`, swapped wholesale when a tab changes.

| Module | Responsibility |
|---|---|
| `app/docset.rs` | opening and switching documents in the set; keeps open document, focus, send state and viewport in step |
| `app/input.rs` | every keypress and mouse event |
| `app/draw.rs` | the frame: tab row, gutter, document, rail |
| `app/footer.rs` | status and the `?` hint, sized so neither can eat the other |
| `app/help.rs` | `KEYS`, the **one** description of every binding, plus the `?` overlay |
| `app/compose.rs` / `compose_view.rs` | the comment box: its state, and its drawing |
| `app/review.rs` | composing feedback from annotations |
| `app/send.rs` | delivery, the Send button's state, and clearing a sent review |
| `app/pick.rs` | choosing among an agent's recent replies |
| `app/headless.rs` | `--bench`, `--snapshot`, scripting; no keypress reaches it |
| `docs.rs` | the presented set: tab naming, collision expansion, tab row overflow |
| `store.rs` | annotations on disk, anchor resolution, delivery records |
| `archive.rs` | the shared feedback archive, wire-compatible with the Plannotator web app |
| `git.rs` | the change bar: what differs from `HEAD` |
| `theme.rs` | every colour, one token per meaning |
| `render.rs` | what rendering needs from outside the layout, so `layout` and `art` share it without importing each other |
| `last/` | finding a coding agent's transcript and its recent replies |
| `herdr/` | the plugin launcher and Herdr's invocation context |

## Seams to the outside

Four, and each must be survivable when absent:

- **Node + `grok-mermaid`** for diagrams. Missing Node, missing package, a rejected diagram or a
  hang all fall back to a plain code block. The first renderer-level failure disables the feature
  for the process, so a machine without Node spawns once per run, not once per fence.
- **git**, for the change bar and the repo name. No repository, no `HEAD`, or no git gives no bars
  and never an error. Paths are canonicalised first: git resolves a symlinked `-C` directory but
  rejects a pathspec spelled through the same symlink (decision 24).
- **Herdr**, for the pane and the delivery target. `E` sends to `PLANNOTATOR_TUI_DELIVER_TO`, else
  the focused agent pane from `HERDR_PLUGIN_CONTEXT_JSON`, else the clipboard. Never
  `HERDR_PANE_ID`: that is the viewer itself.
- **The clipboard**, via OSC 52, which is the fallback for all of the above.

## Review, and what leaves

Four keys leave or narrow the review, and the distinctions between them are the feature:

| Key | Sends | Pane |
|---|---|---|
| `q` | no | closes the **tab**; leaves and closes the pane only when it was the last |
| `Q` | no | closes it |
| `ctrl+c` | no | **left alone**, so `present.sh` finds it by label and reuses it |
| `A` | the whole set | closes it |

`q` closing a tab is how a document is **excluded** from `A`: clean up, then approve the rest.
Nothing is lost, because annotations are written when made, but the excluded document's notes go
unsent and the status line says so. `Esc` cancels what is pending and never leaves: it is the cancel
key everywhere else here, so quitting on it was a trap once `q` stopped quitting.

`E` sends the annotations. `A` hands over the whole set and closes the pane: annotated documents
contribute their notes, unmarked ones arrive as approvals **in prose, never as invented
`LooksGood` records** (decision 22, and the test that pins it is the most important one in the
suite).

A successful send is archived, then the file's annotations are cleared, so the next send carries only
what is new. The order is the safety argument: nothing is cleared unless the archive confirms it
wrote, because the store is the recovery copy (decision 21).

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
~/.plannotator/feedback/<project>/index.jsonl        + records/*.md
```

`<project>` is the git repo name, `<slug>` the basename plus 8 hex of the path's sha256. Wire-exact
with Plannotator Workspaces, so any agent can read it.

## Configuration

`~/.config/plannotator-tui/config.toml`; `plannotator-tui config` prints the effective values.
Sections: `[herdr]` placement, `[mermaid]`, `[image]` (including `obsidian_embeds`), `[code]`
`highlight`, `[git]` `signs`, `[review]` `clear_on_send`, and `[theme]`, which is one token per
meaning rather than per colour so it can be pointed at an outside palette (decision 17).

## Testing

One test per invariant, named for the invariant. The rule this codebase learned the hard way, three
times: **assert the observable behaviour through the public path, never an intermediate quantity.**
A source map asserted by its length, a footer asserted by hint width, and a help overlay counting
only the columns it dropped horizontally were all green while the behaviour was broken.

Verify a new test fails against the defect before fixing it. Mutation is the cheap way: restore the
old line, watch the test go red, revert.

## Measurements

Kept honest in `crates/plannotator-tui/README.md`. The one number that misleads without its reason:
`big.md` renders in ~487 ms, but it holds ~2,800 fenced code blocks, so highlighting dominates a
corpus no real document resembles. A twenty-block document pays about 3 ms.
