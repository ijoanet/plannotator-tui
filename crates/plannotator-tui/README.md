# plannotator-tui

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.

```bash
cargo build --release
./target/release/plannotator-tui samples/plugins.md     # one file
./target/release/plannotator-tui a.md b.md              # a set: flat tree, Shift-Tab cycles
./target/release/plannotator-tui samples                # a folder: tree on the left
```

## Keys

`?` shows this list in the app. Both it and the footer hint render from `KEYS` in `src/app/help.rs`,
and a test walks `input.rs`, `compose.rs`, `pick.rs` and `help.rs` to fail the build if a key is
bound without being described there. Do not hand-write a key list anywhere else.

| Where | Keys |
|---|---|
| anywhere | `?` this list · `Tab` next document · `n` notes · `E` send annotations · `A` send all, approve, close · `q` close this tab (leaves when it is the last) · `Q` leave, sending nothing · `ctrl+c` leave, keeping the pane · `r` reload from disk · `p` pick another reply |
| document | `j`/`k` block by block · `g`/`G` first / last block · `h`/`l` move the cursor · `ctrl+d`/`ctrl+u` half a page · `v` select text · `c` comment on block · `x` clear block notes · drag to select |
| selection | `a` looks good · `c` comment · `d` delete this · `esc` clear the selection |
| notes | `j`/`k` note by note · `e` edit the note · `x` remove the note · `esc` back to the document |

Selections and exports are copied to the terminal clipboard (OSC 52).

## Where things live

Every annotation is saved the moment it is made, as JSON, in the Plannotator data directory:

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

`<project>` and `<slug>` follow Plannotator's own rules (git repo name; basename + 8 hex of
sha256 of the path), so one file maps to one directory in both tools. The records are in the
Plannotator Workspaces wire shape (`plannotator-tui-schema`); any agent can read them. Nothing is
written next to your files. `PLANNOTATOR_DATA_DIR` relocates the directory. Transient
documents (an agent's last message, stdin) are never persisted.

## Headless tools

```bash
plannotator-tui --export <file.md>                     # feedback markdown to stdout
plannotator-tui --bench <file.md>                      # parse / render / reflow timings
plannotator-tui --blocks <file.md>                     # block index, kind, first row
plannotator-tui --annotate <file.md> <quote> <text> [comment|looks_good|delete]
plannotator-tui --annotate-block <file.md> <block> <text>
plannotator-tui --snapshot <file|folder> [cols rows scroll] [select-quote]   # one frame as text + mark map
```

## Measured (Apple Silicon, release build)

| corpus | blocks | rows | render + align | reflow on resize | row lookup |
|---|---|---|---|---|---|
| plugins.md (16 KB) | 60 | 305 | 14 ms | 0.7 ms | 20 ns |
| big.md (2.5 MB, 50k lines) | 14,100 | 54,779 | 487 ms | 68-88 ms | 15 ns |
| 20 Mermaid diagrams | 41 | 181 | 79 ms | 0.1 ms | - |

Per frame, only visible rows are touched. The diagram figure is one Node process for the whole
document; it was 1375 ms when each fence spawned its own, and a machine with no Node spawns once per
run rather than once per fence.

`big.md` is a stress corpus, not a typical document: it holds ~2,800 fenced code blocks, so syntax
highlighting dominates its render (487 ms with highlighting, 320 ms with `[code] highlight = false`).
Highlighting stops after `HIGHLIGHT_CEILING` blocks per document so a pathological file stays
bounded; a document with twenty blocks pays about 3 ms. Reflow does not re-highlight.

The binary is ~7.5 MB, of which ~2.5 MB is syntect and its syntax dumps. That is the cost of
highlighting, and `[code] highlight = false` does not reclaim it: only building without the
dependency would.

## Known limits

- Reference-style links and footnotes lose their target when a block is rendered alone.
- A whole list is one block for block-level commands; text selection is not affected.
- Art (a Mermaid diagram, an image) has no per-character source map, so a text selection
  cannot start inside one; comment on the block instead. Art clips rather than wraps when the
  terminal is narrower than the picture.
- Images render as half-blocks and need truecolor; remote images are never fetched.
- Obsidian `![[image.png]]` embeds render only with `[image] obsidian_embeds`; note
  transclusions are not followed.
- Every color is a `[theme]` token, and badges derive a readable foreground from their fill;
  diagram spans follow pi's mapping of the `grok-mermaid`
  classes, so a diagram can be made to look the same as in the agent that wrote it.
- A table wider than the pane shrinks its columns and wraps its cells; too narrow for `MIN_COLUMN`
  per column and it becomes one labelled record per row. Cell alignment (`:---:`) is not honoured
  once cells wrap.
- A sent review is cleared once archived (`[review] clear_on_send`), so the rail empties after a
  send and the document widens again.
- The gutter's first column is the change bar and the second the block marker, so a very narrow pane
  shows both in two columns and neither can be turned into more room.
- `[git] signs` reads `git diff HEAD` once per document open and once per `r`. A huge repository
  makes that first read slower; it is never done while drawing.
- Below about 23 columns the footer drops the annotation count to keep the document named.
