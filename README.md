<img src="docs/assets/banner.webp" alt="Plannotator TUI" width="720">

Annotate Markdown in the terminal. Select text, leave a 💬 comment, mark it 👍 looks good or
✗ delete, and hand the review to a coding agent as numbered feedback. One static binary,
no runtime. Rust + ratatui.

[![crates.io](https://img.shields.io/crates/v/plannotator-tui?style=flat-square)](https://crates.io/crates/plannotator-tui)
[![release](https://img.shields.io/github/v/release/plannotator/plannotator-tui?style=flat-square)](https://github.com/plannotator/plannotator-tui/releases)
[![ci](https://img.shields.io/github/actions/workflow/status/plannotator/plannotator-tui/post-merge.yml?branch=main&style=flat-square&label=main)](https://github.com/plannotator/plannotator-tui/actions/workflows/post-merge.yml)

```text
                                                                    Send 2 to claude in w1:p1 ▸
  Herdr plugins are shareable, executable workflow packages. A
  plugin can be a Bash script, JavaScript app, Lua script, Rust
▍ binary, or any other argv command your machine can run. Herdr     ╭ 💬  8d3e8 ────────────────╮
▍ owns the host surface: installation, manifest validation,         │ Say which parts the      │
  keybindings, terminal panes, events, invocation context, and      │ plugin can override.     │
  socket access. The plugin owns its implementation language,       ╰──────────────────────────╯
  dependencies, files, and durable state.
    👍  looks good (a)  💬  comment (c)  ✗ delete (d)
▍ Plugins exist so Herdr can stay lean. The core stays focused on   ╭ 👍  25300 ────────────────╮
▍ terminal workspaces, panes, agents, and a stable CLI/socket API.  │ looks good               │
▍ Plugins turn that existing extension surface into reusable        ╰──────────────────────────╯
▍ workflows that people can build, install, and share without
▍ adding every workflow to Herdr itself.

 plugins.md · 2 annotations · selected 36 chars a looks good · c comment · d delete · esc clear
```

Watch it inside Herdr: [demo](https://x.com/plannotator/status/2093419561077154287).

## Install

```sh
brew trust plannotator/tap && brew install plannotator/tap/plannotator-tui   # macOS, Linux
cargo install plannotator-tui                                                # anywhere with Rust
```

Homebrew 6 asks you to trust a third-party tap once before installing from it.

Prebuilt binaries for macOS, Linux and Windows are on the
[releases page](https://github.com/plannotator/plannotator-tui/releases).

## Use

```sh
plannotator-tui docs/plan.md            # one file
plannotator-tui plan.md notes.md        # several: a set, one tab each
plannotator-tui docs                    # a folder: its markdown files, as a set
plannotator-tui last                    # your coding agent's recent replies, pick one, annotate it
```

Drag with the mouse (or `v` and move) to select, then `a` 👍 · `c` 💬 · `d` ✗. `E` sends the review
as numbered annotations (`# Annotations on plan.md`, `## Annotation 1 (line 12)`, …). `A` hands over
the whole set, approving the documents you left unmarked, and closes. Every annotation is saved as
JSON the moment you make it.

`q` closes the current tab, which is how you **exclude** a document from `A`: clean up the tabs you
do not want in the review, then press `A` to approve the rest. Closing a tab never loses anything,
since annotations are already on disk, but it does leave that document's notes unsent, and the status
line says so. `q` on the last tab leaves, as does `Q` at any time, sending nothing.

**`?` lists every key.** The table below is generated from the same source the overlay and the
footer hint read, so it cannot drift from the bindings.

| Where | Keys |
|---|---|
| anywhere | `?` this list · `Tab` next document · `n` notes · `E` send annotations · `A` send all, approve, close · `q` close this tab · `Q` leave, sending nothing · `r` reload · `p` pick another reply |
| document | `j`/`k` block by block · `g`/`G` first / last · `h`/`l` cursor · `ctrl+d`/`u` half a page · `v` select · `c` comment on block · `x` clear its notes · drag to select |
| selection | `a` looks good · `c` comment · `d` delete this · `esc` clear |
| notes | `j`/`k` note by note · `e` edit · `x` remove · `esc` back to the document |

There is no file tree. Documents arrive as a set, one tab per document above the header, and `Tab`
walks them, as does clicking a tab. Tabs are named by file, growing to a parent directory only when
two would collide; the footer spells out the whole path. The gutter bars what changed since `HEAD`
(see below).

The footer carries only `?`. Every other key lives behind that overlay, because the row is worth
more to the document's path than to a list of keys you read once.

## Diagrams and images

A ```` ```mermaid ```` fence renders as Unicode box-drawing art, and a paragraph that is just
`![alt](path.png)` renders as the picture, in half-blocks. Both are annotated like any other
block: `j`/`k` to the block and `c` to comment on it. You cannot select *inside* a picture —
box-drawing is not your text — and a comment on one quotes the Mermaid source or the
`![alt](…)` line, so the feedback your agent gets names what you meant.

Diagrams need [`grok-mermaid`](https://github.com/xl0/grok-mermaid) and Node on the machine;
point `base_dir` at a directory that has it installed:

```sh
mkdir -p ~/.local/share/grok-mermaid && cd ~/.local/share/grok-mermaid && npm install grok-mermaid
```

```toml
# ~/.config/plannotator-tui/config.toml
[mermaid]
enabled = true
base_dir = "~/.local/share/grok-mermaid"   # holds node_modules/grok-mermaid; empty = cwd
node = ""                                  # empty = `node` on PATH
timeout_ms = 5000                          # budget for the whole document

[image]
enabled = true
max_rows = 20      # tallest a picture may render
```

In an Obsidian vault, attachments are written `![[image.png]]`, which is not Markdown —
`pulldown-cmark` hands it back as text, so it is parsed separately and stays behind a flag:

```toml
[image]
obsidian_embeds = true
```

Targets resolve the way Obsidian resolves them: relative to the note, then from the vault root
(the nearest ancestor holding `.obsidian`), then by bare filename anywhere in the vault. A
`|300` size or `|caption` alias is accepted and ignored, since width here is measured in
columns. `![[note.md]]` transclusions are left as text.

`PLANNOTATOR_MERMAID_BASE` and `PLANNOTATOR_MERMAID_NODE` override the first two. Without
Node or `grok-mermaid` a fence stays an ordinary code block, and an image that cannot be read
stays `[img] alt` — neither is an error. Images are read from disk only: a remote `https://`
image is never fetched. Local `png`, `jpeg`, `gif` and `webp` are supported; pictures need a
truecolor terminal.

## After a send

A successful Send or Copy clears that file's annotations, so the next one carries only what is
new - an agent acting on a review should not receive the same comment twice. Nothing is lost: the
feedback archive below already holds what was sent. Clearing only happens once the archive has
written, so with history off a send keeps everything and the status line says so. To keep them
regardless:

```toml
[review]
clear_on_send = false
```

## Width

The annotation rail is only reserved once a file has an annotation, so an unmarked document uses
the whole pane. A table wider than the pane keeps its shape: columns shrink and cells wrap, rather
than the tail being clipped away. Diagrams and images do not wrap - a narrow pane clips a diagram
and re-samples an image.

## The change bar

The gutter's first column bars what changed since `HEAD`, the way `gitsigns.nvim` does, so a
review starts from what the agent actually touched. Measured with `git diff -U0 HEAD`, which
covers staged and unstaged work together, once when a document opens and again on `r` - never
while it is being drawn. Four colours: added, changed, and deleted on the row *after* the gap,
since a removed line has no row of its own. A file git has never seen is untracked on every
line, in its own colour rather than being reported as added. A file git has been told about but
never committed is *added*, not untracked, so staging a new file changes its colour. In a
repository before its first commit every line is added, because there is nothing to compare
against. A document outside a repository gets no bars at all.

```toml
[git]
signs = false           # leave the sign column empty
```

## Colors

Every color is a named token in `[theme]`. A value is anything `ratatui` parses — `#41464e`,
`cyan`, `light-yellow`, `238`, `reset` — and an empty value keeps the default, so a config only
names what it changes. `plannotator-tui config` prints every token and its effective value.

```toml
[theme]
text = ""          heading = ""      heading_deep = ""   code = ""
link = ""          quote = ""        accent = ""         muted = ""
border = ""        comment = ""      approve = ""        delete = ""
comment_bg = ""    approve_bg = ""   block_bg = ""       cursor_bg = ""
toolbar_bg = ""    send_bg = ""
change_added = ""  change_changed = ""   change_deleted = ""   change_untracked = ""
```

The four `change_*` tokens are the change bar's, kept apart from `comment`/`approve`/`delete`
because those mean what the reviewer said, not what git says.

`accent` is focused borders, the selected-block marker, diagram edges and diagram titles;
`muted` is unfocused borders and diagram edge labels; `border` is diagram box-drawing.

Diagram spans follow the mapping pi uses for the same renderer — `border → border`,
`text → text`, `edge → accent`, `edgeLabel → muted`, `title → accent` bold — so pointing these
tokens at your agent's palette makes a plan look the same being reviewed as it did being
written. Matching pi's `synth`, for example:

```toml
[theme]
text = "#c1c3c4"          # pi text
heading = "#ea770d"       # pi mdHeading
heading_deep = "#ea770d"
code = "#06ea61"          # pi mdCode
link = "#42fff9"          # pi mdLink
quote = "#abacad"         # pi mdQuote
accent = "#03aeff"        # pi accent
muted = "#abacad"         # pi muted
border = "#41464e"        # pi borderMuted
comment = "#c9d364"       # pi warning
approve = "#06ea61"       # pi success
delete = "#ff6865"        # pi error
block_bg = "#202527"      # pi bgMuted
cursor_bg = "#202d3a"     # pi selBg
toolbar_bg = "#202527"
send_bg = "#03aeff"
```

## Inside Herdr

Install [Herdr Annotate](https://github.com/plannotator/herdr-annotate); it bundles this binary,
opens it in a pane with `prefix+o` (folder) or `prefix+shift+o` (agent's last reply) or by
Ctrl-clicking a `file://…md` link, and the header button sends the review straight back to
the agent as its next message: `Send 3 to claude in w1:p2 ▸`.

```toml
# ~/.config/plannotator-tui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotator-tui config` prints the file's path and the values in effect. The `herdr/`
directory in this repo is the development manifest; users should install Herdr Annotate.

Actions forwarded by Herdr Mirror default to a split beside the invoking remote
pane. Mirror does not preserve overlay presentation, and Herdr 0.8.2 opens an
overlay in its server's active tab, which can differ from the tab you are viewing.
An explicit `--placement` or `PLANNOTATOR_TUI_PLACEMENT` still takes precedence.

## Agent replies

`plannotator-tui last` finds the transcript of the agent that launched your shell and shows a
picker of its recent replies. Hosts: Claude Code, Codex, pi, Oh My Pi, GitHub Copilot CLI,
Droid, Hermes CLI, OpenCode (1 and 2). `--host`, `--pid`, `--session <transcript>` (format sniffed when
no host is named) and `--session-id <id>` (Hermes, OpenCode) override detection; `--stdin`
reads a document;
`--print` writes the newest reply to stdout and always exits 0 (for hooks and scripts).
A reply review keeps its annotations in memory only; nothing about it survives the run, but
the feedback you send or copy is archived like any other (see Feedback archive below).

On Linux, an explicit Codex `--pid` selects the rollout opened by that process. If it
cannot be identified uniquely, `last` reports the failure instead of choosing an unrelated
session. `--session` and `--session-id` keep precedence over PID discovery.

Inside Herdr, the exact-session path needs the session id Herdr reports for the pane. Herdr's
Claude Code integration registers on `SessionStart`, so a session reports its id only when it
started after `herdr integration install claude`; a session that was already running when the
integration was installed reports none. Without an id, `last` shows the newest transcript for
the folder, which is a guess when several sessions share one directory, and says so in the
status line.

## Where annotations live

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

`<project>` is the git repo name, `<slug>` the file's basename plus 8 hex of the sha256 of
its path: Plannotator's own layout, so both tools see one record per file. The JSON is the
Plannotator Workspaces wire shape; any agent can read it. Nothing is written next to your
files. `PLANNOTATOR_DATA_DIR` relocates the directory.

### Feedback archive

A successful Send or Copy also appends what was submitted (the feedback text, the quoted
selections and their annotations, and the file, folder or agent session it was about) to
`{data_dir}/feedback/<project>/index.jsonl`, with a Markdown copy under `records/`. The data
dir is `PLANNOTATOR_DATA_DIR`, else an existing `~/.plannotator`, else
`$XDG_DATA_HOME/plannotator`, else `~/.plannotator`. File, folder and reply reviews are all
archived; a send that fails or is refused is not. The format is the one the Plannotator
browser app writes, so both tools share one history. To turn it off, set
`PLANNOTATOR_FEEDBACK_HISTORY=0` (once the variable is set, only `1` or `true` enable) or put
`"feedbackHistory": false` in `{data_dir}/config.json`; the variable wins over the file.

## Headless

```sh
plannotator-tui --export <file|folder>                          # the review, to stdout
plannotator-tui --annotate <file> <quote> <text> [comment|looks_good|delete]
plannotator-tui --snapshot <file|folder> [cols rows scroll] [quote]   # one frame as text
plannotator-tui --bench <file>                                  # parse / layout timings
```

## Repository

- `crates/plannotator-tui`: the app. `crates/plannotator-tui-schema`: annotation and anchor
  types, wire-compatible with Plannotator Workspaces. `crates/plannotator-tui-hosts`: agent
  transcript readers.
- `docs/architecture.md` is the map of the app; `docs/decisions.md` the design record; `AGENTS.md` the engineering rules;
  `crates/plannotator-tui/README.md` the full key reference and measurements.

MIT.
