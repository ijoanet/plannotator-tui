# Decisions

Short records of choices that shape the code. Newest last. Each has the reason and the
source, so a future change can revisit the reason rather than the conclusion.

## 1. Anchor wire shape (2026-08-28)

Validated with workspaces-ops against the Workspaces code.

```json
{
  "originalText": "<rendered selection text>",
  "quote": "<same>",
  "plannotator-tui": {
    "kind": "comment" | "looks_good" | "delete",
    "source": { "start": <byte>, "end": <byte>, "version": "<git blob sha of the raw markdown>" },
    "prefix": "<up to 32 chars of RAW source before>",
    "suffix": "<up to 32 chars of RAW source after>",
    "block": <top-level block index hint>
  }
}
```

- The server stores the anchor opaquely (16 KiB cap; only a top-level `point` is inspected).
- The web client renders from `originalText` (falls back to `quote`): exact substring over the
  rendered text, first occurrence, whitespace-collapsed fallback. It ignores everything else.
- Everything plannotator-tui-only lives under one `plannotator_tui` key. Top level carries only
  `originalText`, `quote`, and that namespace. Never claim `startMeta`, `endMeta`,
  `htmlAnchor`, `htmlAdditionalTargets`, `point`, `kind`, `state`, `source` at top level.
- `prefix`/`suffix` are RAW source, same coordinate system as the byte offsets. One system
  per object.
- The spec.yaml example `{block_id, start_offset, original_text}` is stale: it saves and
  renders as an unanchored card. Not used.

## 2. Kinds are not bodies (2026-08-28)

`looks_good` and `delete` are tags in the namespace, not body text. The body is the human
sentence a teammate should read (may be empty for looks_good). The web shows an ordinary
comment; plannotator-tui shows the glyph. No verdict convention exists on the annotation surface
(verdicts are approval rounds, a different object).

## 3. Cross-block selections join with no separator (2026-08-28)

Verified by workspaces-ops with an executed DOM test through the real viewer: the rendered
DOM contributes no character between blocks (`textContent` is `"...words.The second..."`),
so a cross-paragraph `originalText` resolves only when the block texts are joined with
**no separator**. `"\n\n"` and space joins both fail (the whitespace-collapse fallback
normalizes toward a space the haystack never has). This is what the browser's own
`Range.toString()` produces, which is why in-app cross-block annotations already work.

So: `originalText`/`quote` = rendered block texts concatenated directly. The raw-source
form with real newlines stays under `plannotator-tui.source`. No clamping. An upstream ask is
filed so `"\n\n"` eventually works too; we do not wait on it.

## 4. Document version = git blob sha (2026-08-28)

The document `ETag` is `sha1("blob " + len + "\0" + bytes)` over the exact bytes returned —
byte-identical to `git hash-object`. Computed locally; used as `plannotator-tui.source.version`.
When comparing, strip `W/` and quotes first.

## 5. Polling cannot use `?since=` for updates (2026-08-28)

`since` filters on created time. Resolves and edits do not appear. Phase 3 polls the full
per-document list (small) and diffs by id + state + updated_at. If that gets expensive it
becomes a contract ask for a changed-signal.

## 6. Documents are sources, not files (2026-08-28)

From plannotator-ops on the "last message" capability: the app opens a *document source* —
content, display name, `transient` flag, opaque provenance — not a path. A file is one
source; an agent's last message handed in as a string is another. Transient sources have
no sidecar, no history, no drafts. Delivery is a separate seam: on submit, feedback goes
back to wherever the source came from (a file's sidecar, a Workspaces document, an agent
pane via Herdr). Transcript extraction is its own crate behind the source seam (decision
8); the app core never sees transcript formats.

## 7. No abstraction ahead of a second caller (2026-08-28)

Traits exist only at seams to external systems: document source, delivery, Workspaces
client, clipboard. Everything else is concrete.

## 8. plannotator-tui owns "last message" extraction (2026-08-28)

A standalone app cannot depend on the Plannotator CLI (a Bun + browser install) to read an
agent's last reply. `plannotator-tui last` extracts it itself, in a `plannotator-tui-hosts` crate: one
small module per agent behind one trait — `detect`, `last_message`, `deliver`. Claude Code
first (JSONL session log: last assistant entry on the active branch, text blocks joined),
Codex second; others on demand. Each host module carries one real transcript fixture and
one test. The Plannotator sources (`apps/hook/server/session-log.ts`, `codex-session.ts`)
are the format reference — knowledge copied, not code. Decision 6 is unchanged: the
message is a transient document source, delivery is a seam, and inside Herdr `deliver`
may target the pane's agent.

## 9. `plannotator-tui last`: detection and extraction, from the Plannotator source (2026-08-28)

Read directly from `/Users/ramos/plannotator/plannotator` (`apps/hook/server/index.ts:505-520`,
`:1348-1520`; `session-log.ts`; `codex-session.ts`) and verified on this machine. This is the
reference for `plannotator-tui-hosts`; knowledge copied, not code.

**Host detection is an env-var chain, then a fallback.** `PLANNOTATOR_ORIGIN` override
(validated) > `CODEX_THREAD_ID` > `COPILOT_CLI` > `OPENCODE` > `GEMINI_CLI` > `OMPCODE`
(last: OMP exports it into every shell it spawns) > default Claude Code. We mirror it with
`PLANNOTATOR_TUI_HOST` as the override. `cwd` is never identity.

**Claude Code session resolution — the deterministic part.** Claude Code writes
`~/.claude/sessions/<pid>.json` per running session: `{pid, sessionId, cwd, startedAt, …}`
(verified: `1421.json` here belongs to the plannotator-ops session). Ladder, most precise
first, first hit wins:
1. **Ancestor-PID walk** (`resolveSessionLogByAncestorPids`): snapshot the process table
   once (`ps -eo pid=,ppid=`, parsed by a pure function), walk ≤8 parents from our ppid,
   read `sessions/<pid>.json` at each hop; match its `sessionId` to
   `~/.claude/projects/<slug(cwd)>/<sessionId>.jsonl`. Ghost check: if a NEWER jsonl exists
   in that project dir with no registered metadata, it is a `/clear` session — prefer it.
   This is why `plannotator last` works from a bare shell: the shell's ancestor IS claude.
2. **Cwd scan of session metadata**: all `sessions/*.json` whose `cwd` matches, newest
   `startedAt` first, matched to a jsonl.
3. **Slug + mtime**: `~/.claude/projects/<cwd with [^A-Za-z0-9-] → '-'>/*.jsonl`, newest
   first; case-insensitive dir fallback (Windows lowercases the slug).
4. **Ancestor-directory walk**: try each parent directory's slug (user `cd`'d deeper).
Each candidate is tried until one yields a message ("no messages" means "wrong file").

**Claude Code extraction** (`resolveActiveBranchIndices`, `extractRecentRenderedMessages`):
- Parse JSONL leniently (skip malformed lines). Entries carry `uuid`/`parentUuid`; bookkeeping
  types (`last-prompt`, `ai-title`, `mode`, `file-history-snapshot`) have none and are often
  written last, so "newest entry" = newest entry WITH a uuid.
- Active branch = walk `parentUuid` from that entry to the root (`parentUuid: null`).
  Untrusted chain (no ids, dangling parent, cycle) → fall back to file order.
- `/compact` writes a new root, so right after it the active branch may hold no assistant
  text: an empty result falls back to file order ("fail open, never fail empty").
- Skip: `progress`, `system`, `file-history-snapshot`, `queue-operation`; hidden visibility
  (`llm_only`, `assistant_only`, `hidden`); `isSidechain` subagent entries; non-text blocks
  (`thinking`, `tool_use`). Role = `type` or `message.role`.
- A rendered message = all `text` blocks of the same `message.id` (streamed chunks share
  it), concatenated in file order. Collect the newest N such messages for the picker
  (Plannotator uses 25); the newest is the default.
- A human prompt = `user` role, not hidden, has text, and does not start with
  `<local-command-`, `<command-name>`, `<local-command-stdout|stderr>`, `<system-reminder>`,
  `<system-notification>`.

**Codex.** `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`; thread id = uuid in
the filename; scan date dirs newest-first. Entries: `type == "response_item"`,
`payload.type == "message"`, `payload.role == "assistant"`, text from `output_text` blocks.
Active turn: newest `event_msg` turn-start after the newest turn-complete → walk backward
from just before it. Multi-file threads (issue #1367, PR #1387 unmerged): collect ALL files
for the thread from day one and walk backward across them.

**Delivery contract** (`index.ts:338-350`, ours to freeze): plain mode prints feedback on
stdout, empty on close, "The user approved." on approve, **exit 0 always** (a non-zero exit
from a Bash bang-prefix aborts the prompt before the model reads it). `--json` prints one
`{"decision":"approved|dismissed|annotated","feedback":"…"}`. `--hook` prints nothing on
approve/close and `{"decision":"block","reason":"…"}` on annotate. No size framing, no
bracketed paste. Hosts may kill our process group on a shell timeout (OpenCode: 120 s).

**Explicit input first.** `plannotator-tui last --stdin` and `PLANNOTATOR_TUI_HOST`/`PLANNOTATOR_TUI_SESSION`
overrides ship before any detection, so the tool is usable and testable without a host.

**Tests.** Every resolver and parser is a pure function over strings/paths with injected
`sessions_dir`, `projects_dir`, and a `parent_pid` closure — never the real `~/.claude`,
never a spawned `ps`. Plannotator's `session-log.test.ts` (1,621 lines) is the regression
inventory: slug rules, human-prompt filtering, last-message extraction edge cases, picker,
active branch, rewind, compact, ancestor walk, ancestor pids, cwd scan, `ps` parsing,
cross-platform cwd compare. Freeze the JSONL entry shape and our stdout contract; do not
freeze Codex file layout or anything derived from process tables.

## 10. The anchor stores the raw quote; context only ranks (2026-08-28)

Found by a test in phase 1: reconstructing the quote from prefix/suffix is unsound — an
empty suffix matches anywhere, so a deleted quote "resolved" to whatever now sits after the
prefix. `plannotator-tui.quote` holds the selected raw-source text and is the only thing the
resolver searches for. The byte range is a shortcut (trusted only when it still holds the
quote between its context); prefix/suffix and the block hint only rank occurrences. A quote
that is not in the document is an orphan, always.

## 11. Submit is delivery, and delivery is a seam (2026-08-28)

The standalone app has no submit step: every annotation is saved to JSON as it is made, and
`E` copies the feedback text to the clipboard. "Send this review back to the agent" is a
Herdr concern, because only Herdr knows which agent sits in which pane.

The app is built so that is a plug, not a rewrite:

```rust
/// Where rendered feedback goes when the user sends it.
pub(crate) trait Delivery {
    fn describe(&self) -> String;              // shown in the footer: "send → clipboard"
    fn deliver(&self, feedback: &str) -> Result<()>;
}
```

- `Clipboard` is the only implementation in 2b, and the default everywhere.
- The Herdr plugin (phase 5) adds `HerdrAgent { pane_id }`: `herdr agent prompt <pane>
  <feedback>`. Verified in Herdr's source (2026-08-28): an overlay/split pane's
  `HERDR_PLUGIN_CONTEXT_JSON` is snapshotted **before** the new pane spawns
  (`src/app/api/plugins/panes.rs:51`), so `focused_pane_id` / `focused_pane_agent` name the
  pane that was focused when plannotator-tui was triggered — the agent's pane. An explicit
  `--deliver-to <pane>` overrides. `agent prompt` takes the text as one argument, honors
  the pane's bracketed-paste mode, sends Enter after 300 ms, and returns `agent_blocked`
  without sending if the agent is waiting on a dialog; the footer shows that. The footer
  always names the target.
- `plannotator-tui last` (phase 4) adds the host deliveries (stdout with exit 0, hook JSON).
- The feedback text is produced by one function (`export::feedback`) regardless of target,
  so what an agent receives from Herdr is byte-identical to what the clipboard gets.

This is decision 6's delivery seam made concrete. Nothing else in the app knows about
Herdr; `HERDR_ENV=1` only selects the implementation.

## 12. Placement is the user's, and one launcher serves humans and agents (2026-08-28)

Where plannotator-tui opens inside Herdr — full-screen overlay, split beside the agent, or a
modal popup — is a preference, not a property of the entry point. It lives in plannotator-tui's
own config (`[herdr] placement`, `~/.config/plannotator-tui/config.toml`), because Herdr has no
per-user plugin settings and a manifest cannot express "the user prefers". The parser is
strict: an unknown key is an error that names it, so a typo never silently falls back.
`PLANNOTATOR_TUI_PLACEMENT` and `--placement` override for one launch; the agent skill uses
`split` because watching the agent react is the point of that path.

All entry points run the same command, `plannotator-tui herdr open [PATH]`. A manifest action runs
it with Herdr's invocation context (the focused pane's folder, or the Ctrl-clicked
`file://` link, and the focused pane's agent as the feedback target); an agent runs it from
its own pane, where `HERDR_PANE_ID` is the caller and therefore the target. The launcher
resolves file, target and placement, then execs `herdr plugin pane open` with explicit
`PLANNOTATOR_TUI_FILE` / `PLANNOTATOR_TUI_DELIVER_TO` / `PLANNOTATOR_TUI_DELIVER_AGENT`, so the pane never
has to guess where it came from. Inside the pane `HERDR_PANE_ID` is plannotator-tui's own pane
and is never a target. `argv` construction is a pure function with exact-string tests.

Verified live (Herdr 0.8.2, disposable named session, a `cat` renamed `claude` standing in
for the agent): `agent prompt` delivers a multi-line feedback body verbatim; the manifest
action opens the overlay in folder mode on the agent's cwd; `--placement` on `plugin pane
open` overrides the manifest, so one `doc` entrypoint serves all three placements.

## 13. Sending is a button, and the record remembers it (2026-08-28)

Herdr is mouse-first, so the send is a visible control in the header, not only `E`. Its
label is derived from the delivery target and the send state — `Send 3 to claude ▸`,
`Sent ▸ claude`, `claude at a dialog · copied · click to retry`, or `Copy 3 as feedback`
outside Herdr — so the user always knows where feedback goes before pressing anything.
The button and `E` call the same function; there is no second path.

Delivery outcomes are typed (`Blocked`, `Unavailable`, `Failed`) because the app reacts
differently: a blocked agent means "retry later", so the text is also put on the clipboard
and nothing is lost; a missing agent means the clipboard is the target now.

`annotations.json` gains `deliveries: [{at, target, annotation_ids}]`. It is what lets the
button say "Sent" after a restart and, later, lets Workspaces know which comments the agent
has already seen. It is append-only and absent until the first send, so records written by
earlier builds load unchanged. A folder-wide send is recorded on the open file only; the
per-file state is a UI convenience, not an audit log.

## 14. What the real transcripts changed in decision 9 (2026-08-28)

`plannotator-tui-hosts` was built against decision 9 and then checked against this
machine's live Claude Code and Codex files. Four things the rules did not say:

- `attachment` entries carry `uuid`/`parentUuid` and sit inside the parent chain. The branch
  walk passes through them and rendering skips them; skipping them from the walk would make
  every chain look dangling.
- `/compact` is a `system` entry with `subtype: "compact_boundary"`, `parentUuid: null` and a
  `logicalParentUuid`, followed by a `user` entry flagged `isCompactSummary` /
  `isVisibleInTranscriptOnly`. The summary is not a human prompt. The file-order fallback
  after a compact works as specified.
- `<task-notification>` is another machine-written prefix on `user` entries; it joins the
  human-prompt filter.
- Codex assistant messages carry `phase` (`commentary` | `final_answer`); both are returned
  newest first so the final answer is the default pick. Subagent rollouts
  (`payload.source.subagent`) have their own thread ids and are excluded when choosing a
  thread — grouping by `session_id` would merge a reviewer's output into the main thread.

`visibility` and `isSidechain` did not appear in the sampled transcripts; the rules stay,
covered by synthesized fixture entries. The stdout contract (`--print`: newest reply, exit 0
always, errors on stderr) is frozen as decision 9 required.


Peer-reviewed against Plannotator's source at main by the plannotator-ops session
(2026-08-28): the attachment-in-chain rule matches Plannotator exactly; the compact-summary
exclusion, the `<task-notification>` prefix, the explicit Codex subagent skip, and
multi-rollout grouping are stricter than Plannotator (which has an open bug, #1367, on the
last one). Keep them; do not regress to match.

Reference survey (plannotator-ops, 2026-08-28, Plannotator at main): only Claude Code,
Codex, Copilot CLI and Droid have on-disk transcript readers there. Copilot sessions
(`~/.copilot/session-state/<uuid>/`) are found through `inuse.<pid>.lock` files matched
against the ancestor pids — in Herdr the pane's pid is a direct hit — with a cwd ladder as
the fallback (`copilot-session.ts`); a lock only counts while its pid still names a Copilot
process. Droid (`~/.factory/sessions/<slug>/`) reuses Claude's entry shape with `id` /
`parentId`, has no rewind tree and no per-process metadata, so its current session is the
newest log for the cwd's slug (else the first ancestor directory with logs), read in file
order and never falling through to an older sibling. Both are mirrored here with fixtures
cut from real sessions on this machine. OpenCode is an API bridge in Plannotator and Gemini
CLI is env-detected only; neither has a format to mirror, so in Herdr they fall back to the
pane's screen text.

**Pi** (2026-08-28). Sessions live in `~/.pi/agent/sessions/--<cwd with its leading slash
dropped and `/`, `\`, `:` as `-`>--/<timestamp>_<uuid>.jsonl` (`PI_CODING_AGENT_SESSION_DIR`
or `PI_CODING_AGENT_DIR` override; legacy flat files directly under `sessions/` still
exist). Every entry carries `id`/`parentId` — messages, model and thinking-level changes,
compactions, custom entries — so the active branch is the chain from the newest entry with
an id, which is what pi's own `getBranch()` returns (no lane records are written to v3 files;
operation records are in-memory only, so there is no on-disk turn marker). Unlike Claude
Code there is **no file-order fallback**: a chain that cannot be reconstructed yields nothing
rather than the wrong messages. Rendering matches Plannotator's `pi-extension/assistant-message.ts`
exactly: a `message` entry whose `content` is an array, text = the `text` blocks joined with
`\n`, whitespace-only (toolCall-only) entries skipped, one entry = one message keyed by the
entry id, timestamps normalized to ISO (numbers are Unix ms). There is
no pid registry, so a running pi is matched by cwd (the Herdr launcher passes the agent
pane's cwd as `PLANNOTATOR_TUI_CWD`), newest first, skipping sessions that hold no message
yet. pi exports `PI_CODING_AGENT=true` and `AI_AGENT=pi` into the shells it spawns; both
select the pi host after the Codex marker.

**Oh My Pi and Hermes CLI** (2026-08-29, plannotator-tui#24, #25). OMP is a pi harness: same
entry format, same encoded-cwd layout, rooted at `~/.omp/agent/sessions`, and it reuses pi's
`PI_CODING_AGENT_SESSION_DIR` / `PI_CODING_AGENT_DIR` overrides (`oh-my-pi/packages/coding-agent/src/cli/args.ts`),
so `omp` is pi's reader over another root. Its `OMPCODE` marker selects it last of all the
markers, as Plannotator orders them, and `AI_AGENT=omp` first, before pi's own flag which OMP
also sets. Hermes CLI has no transcript files: conversations are rows in SQLite
(`~/.hermes/state.db`, `HERMES_HOME` override, WAL mode), addressed by the session id Herdr
reports. The reader opens the database read-only (`mode=ro`, so the live WAL is visible),
falls back to `immutable=1` only when the shared-memory index cannot be mapped (a stale read
beats touching a running agent's store), and issues one query over
`idx_messages_session_active`, newest first. Both arrive from Herdr through `agent_session`
(`kind: path | id`) rather than host+pid discovery; a transcript path handed over without a
host name is recognised by its first lines (`sniff`), so any Herdr-integrated agent that
writes one of the known formats works without a host table entry.

Source-verified 2026-08-29 against `NousResearch/hermes-agent` (`hermes_state_common.py`,
`hermes_state.py`, `hermes_state_search.py`, `hermes_constants.py`) and `oh-my-pi`
(`packages/utils/src/procmgr.ts`): Hermes writes `messages.timestamp` with `time.time()`
(Unix seconds); `active=0, compacted=0` rows are rewinds the user took back and
`active=0, compacted=1` rows are compaction archives, so `active = 1` is the right filter;
the session id Herdr reports is `sessions.id`; `HERMES_HOME` else `~/.hermes`
(`%LOCALAPPDATA%\hermes` on Windows). omp exports only `OMPCODE=1` and `CLAUDECODE=1` to
child shells, no pi marker, so the marker chain reaches `OMPCODE` correctly.

**OpenCode** (2026-08-29). OpenCode 1.x keeps sessions, messages and parts in one SQLite
database, not transcript files: `$OPENCODE_DB`, else `<xdg data>/opencode/opencode.db` where
the xdg data dir is `$XDG_DATA_HOME` or `~/.local/share` on every platform (`packages/core/src/global.ts`
uses `xdg-basedir`, which has no Windows branch; `packages/core/src/database/database.ts`
names the file). A message's text is its `part` rows of type `text` in creation order;
parts flagged `synthetic` (context OpenCode injects) or `ignored` are not something the user
or model wrote and are skipped, as are `reasoning`, `tool` and `step-*` parts, so an
aborted assistant turn with no text does not take a picker slot. Herdr's OpenCode manifest
carries no session id, so the reader picks the newest top-level (`parent_id IS NULL`,
subagent sessions excluded), unarchived (`time_archived IS NULL`) session whose
`session.directory` equals the agent pane's cwd (`PLANNOTATOR_TUI_CWD`), falling back to the
newest session started in an ancestor of that cwd, since OpenCode records its launch
directory and the pane may have moved. `--session-id` addresses a session directly. The
database is opened read-only exactly as for Hermes (shared opener, `mode=ro` then
`immutable=1`); one query joins `message` and `part` newest-message-first. `OPENCODE=1` in the
environment now selects this host instead of reporting it unsupported. Verified live against
opencode 1.18.23 (`~/.local/share/opencode/opencode.db`, 45 MB, WAL) on 2026-08-29.

**OpenCode 2** (2026-08-30, plannotator-tui#40). The `opencode2` binary (`anomalyco/opencode`,
branch `beta`, `packages/cli`) keeps the same data directory and, on the standard channels
(`latest`, `dev`, `beta`, `next`, `prod`), the same `opencode.db`, but writes to new tables:
`session_v2` (the columns this reader uses are unchanged: `directory`, `parent_id`,
`time_updated`, `time_archived`) and `session_message` (`type` is the role, `seq` the order,
`data` the payload; an assistant's text is `content[].type == "text"`, a user's is `text`;
`synthetic`, `system`, `skill`, `shell`, `compaction` and the `*-switched` rows are not
rendered). There is no `part` table. Other channels use `opencode-<channel>.db` beside it
(`packages/cli/src/server-process.ts`). After the v1 migration both table families hold data,
so the reader now considers every `opencode*.db` in the data dir and both schemas, and picks
the newest top-level unarchived session for the cwd across all of them; a `--session-id` is
looked up in whichever table holds it. Verified against the `beta` source
(`packages/core/src/session/sql.ts`, `packages/schema/src/session-message.ts`,
`packages/util/src/global-roots.ts`) and a mixed-schema fixture reproducing the report.


## 15. Diagrams and images are art blocks, not a graphics protocol (2026-09-08)

A Mermaid fence and an image-only paragraph render as a picture. Two choices shaped it.

**Cells, not pixels.** Images render as Unicode half-blocks (`▀`, foreground = upper pixel,
background = lower) rather than kitty/sixel graphics. The app is a grid of styled cells:
everything downstream — scrolling, clipping, the selection's row map, `--snapshot`, a
`TestBackend` assertion — operates on cells. A real raster image would need out-of-band
escapes placed at absolute positions, which the buffer diff knows nothing about and which
scroll wrong the moment the document moves. Half-blocks also need no terminal capability
negotiation and degrade to the same picture everywhere truecolor works. With two pixels per
cell each sub-pixel is square, so fitting the picture to `cols × 2·rows` keeps aspect ratio
with no correction factor.

**Art stands for its block.** A rendered character normally maps to a source byte (`srcmap`),
which is what makes a selection quotable. Box-drawing is not the author's text, so an art
block carries no per-cell offsets: `cells` are all `None` and no selection can start inside a
picture. But a block-level comment must still store a quote the anchor can resolve, so
`rendered_in_range` returns the *source* an art block stands for — the Mermaid source, or
`![alt](url)` — instead of the empty string it would otherwise produce. Without that,
commenting on a diagram wrote an annotation with no `originalText`.

Consequences worth knowing: art never word-wraps (a narrow terminal clips a diagram, as it
clips a wide table); an image is re-sampled from a bounded thumbnail on resize, because unlike
text its shape is chosen to fit the width; and an image alongside text in one paragraph keeps
its normal rendering, because the text is content someone may want to quote.

## 16. Mermaid rendering shells out, once per document (2026-09-08)

`grok-mermaid` is the only Mermaid-to-terminal layout engine worth using and it is JavaScript,
so the binary shells out to Node with an embedded bridge script (`include_str!`). This is the
one runtime dependency outside the binary, which is why it is failure-shaped everywhere: no
`node`, no `grok-mermaid`, a diagram the renderer rejects, or a renderer that hangs past
`timeout_ms` all fall back to showing the fence as an ordinary code block. Nothing about a
diagram can fail a document.

Two costs forced the shape. Node's startup is ~65 ms and dwarfs laying out a diagram, so all
of a document's diagrams go through **one** process: `--bench` on twenty diagrams went from
1375 ms to 79 ms. And the first *renderer-level* failure sets a process-wide flag, so a machine
without Node spawns one process per run rather than one per fence — twenty fences cost 22 ms
instead of twenty failed spawns. The bridge distinguishes the two cases explicitly (`kind:
"renderer"` versus a per-diagram `ok: false`) rather than making the caller match on error
strings.

The reply's diagram count is never trusted over the request's: results are zipped back by
input index so a short or malformed reply cannot shift a diagram onto the wrong block.

## 17. One palette, named by meaning, shared with the diagram renderer (2026-09-08)

Colors were literals spread across `layout` (markdown), `art/mermaid` (diagram spans) and
`app/draw`, `app/header`, `app/pick` (chrome), including two `Color::Indexed` values that
`--snapshot`'s mark map compared against by hand. That made the palette impossible to retune
and easy to break: changing an annotation background silently broke the mark map.

`theme.rs` now holds one token per thing that has a color, named for its **meaning**
(`accent`, `muted`, `border`) rather than for a color. Meaning is what lets a config file match
an outside palette without this crate knowing anything about it, and it is what keeps the
document, the diagram and the chrome coherent when any one of them is retuned. Values are
parsed by `ratatui`'s `Color::FromStr`, so `#41464e`, `cyan`, `238` and `reset` all work and a
typo names the token and the value. Defaults are the previous literals exactly, so an existing
install looks unchanged.

The diagram mapping is deliberately **pi's**: `grok-mermaid` labels each span
`border`/`text`/`edge`/`edgeLabel`/`title`, and pi renders those as `borderMuted`, `text`,
`accent`, `muted` and bold `accent`. Adopting the same mapping means a reviewer pointing these
tokens at the agent's own theme sees a diagram exactly as the agent drew it — the plan is not
re-read in different colors than it was written. It also replaced two choices that were merely
inherited: edges were `Blue` while focused borders were `Cyan`, and edge labels shared
`LightYellow` with inline code, so a diagram never looked like part of the same app.

`--snapshot`'s mark map now reads the resolved theme instead of literals, so a themed run still
reports which annotation covers each cell.

## 18. Obsidian embeds are parsed by hand, behind a flag (2026-09-08)

`AGENTS.md` says we never interpret markdown ourselves, and every other detection in this crate
obeys it: the mermaid fence's info string and the image destination both come from
`pulldown-cmark`'s event stream. `![[image.png]]` cannot. It is not Markdown — it is Obsidian's
own wiki-embed — so `pulldown-cmark` correctly returns it as plain text and no amount of walking
the event stream will find it. Vaults written in Obsidian use this form for *every* attachment,
which made it the one syntax the image renderer could not see.

So `art/obsidian.rs` parses it directly, and that is the only sanctioned exception to the rule.
It is contained: one function that accepts a paragraph which is exactly one embed (the same
"nothing else in the paragraph" test the Markdown form uses), and rejects anything with a stray
bracket, a line break or a second embed.

It is **off by default** (`[image] obsidian_embeds`). A CommonMark document that happens to
contain `![[x]]` means nothing by it and must not grow a picture; only someone who knows their
document is a vault note turns it on.

Resolution follows Obsidian, not the filesystem: relative to the note, then from the vault root
(nearest ancestor holding `.obsidian`), then **by bare filename anywhere in the vault**, which
is Obsidian's shortest-path form and the reason a filename index exists at all. The index is
built at most once per document and only when a document actually contains an embed, bounded at
20k entries and skipping hidden directories, `node_modules`, `target` and `.git`. First match
wins, so a change in walk order cannot flip which of two same-named files is chosen.

Only decodable extensions resolve. `![[note.md]]` is a transclusion, not a picture; rendering it
as one would be a lie, and following it is a different feature.

## 19. A destination is a name, and a badge picks its own foreground (2026-09-08)

Two defects with one cause: decision 17 made every colour a token but left two values that only
worked for the palette they were first written against.

The Send button painted `Color::Black` on a themed fill. On the old green that was 2.6:1; on a
darker themed green it fell to **1.64:1**, unreadable. A badge cannot hardcode a foreground it did
not choose the background for, so `theme::readable_on` derives it from the fill's relative
luminance (WCAG, the 0.179 threshold browsers use) and returns black or white. It takes a colour,
not a theme, because it reads no palette. An indexed or named fill has no channels to inspect and
assumes a dark terminal.

The button also named the destination by pane id — `pi in w18:p1P`. A pane id answers "which
process", not "where did my review go", which is the question a reviewer actually has. Herdr turns
out not to help directly: an agent pane carries **no label of its own** (`pane get` returns
`label: null`); what a human recognises is the *tab* label. So the destination resolves pane label
(plugin panes set one) → tab label → pane id, costing one extra `herdr tab get` at startup and only
when the pane is unlabelled. `HerdrAgent` carries the name beside the pane, because delivery still
addresses the pane — the name is only ever what the human is told.

## 20. Tables are laid out here, not by tui-markdown (2026-09-08)

`tui-markdown` sizes a table's columns from its content alone - `column_widths` is
`max(cell.width())` - and `Options` has no width knob, so a table can never be asked to fit. A
table preserves columns, so one wider than the pane was clipped: the tail of every row and the
right border silently gone, and because clipping drops the cell map too, the lost text could not
even be selected. Reading a table you cannot finish reading is the one thing a review pane must
not do.

Showing the block's raw Markdown instead was tried first. It loses nothing, and it is not a
rendering - `| a | b |` wrapped over three lines is harder to read than the table was. Rejected on
sight of it.

So `table.rs` lays the table out: shrink the widest column by one until the row fits, then wrap
each cell inside its column. One column at a time rather than a proportional formula, because
"always shrink the widest" is a rule a reader can check against the output. The table stays a
table at every width.

Cells come from `pulldown-cmark`'s event stream with source ranges, so this is not a second
Markdown parser - and every rendered character keeps the byte it came from, which makes a
selection inside a wide table work for the first time. Widths are display widths throughout.

**Corrected 2026-09-09.** This decision originally said a test asserts one offset entry per display
column, "because the selection map indexes by column". That was wrong, and it was wrong in the
direction that hides a bug. Two maps exist and they are indexed differently: `Row.cells` is
per-column, because hit-testing takes a column; `LineOffsets` is per-**char**, because `cells_of`
pulls one entry per char. Emitting the per-column form into the per-char path mis-anchors every
character after a wide one, cumulatively, so selecting `x` in `echo 日x` stored `日`. The test
asserted the intermediate (`offsets.len() == line.width()`) rather than the invariant, so it passed
while the bug was live, and a later seat copied the same shape into the code-block renderer on the
strength of this paragraph. The invariant to assert is behavioural: the byte under a given column is
the byte of the character rendered at that column.

Below `MIN_COLUMN` per column - six columns in thirty - no drawn table is readable, so one
labelled record per row takes over, the way `psql \x` expands a wide result.

Not yet honoured: cell alignment (`:---:`). Everything is left-aligned in the wrapped path.

## 21. A sent review is cleared, once it is archived (2026-09-08)

Decision 13 recorded the send so the button could say "Sent" after a restart. It left the
annotations in place, which made every later send carry them again: an agent that had already
acted on a comment received it a second time, and a third. Reported from use - two items in one
review had been implemented before they arrived.

So a successful send clears what it covered. The annotations are not lost: `archive.rs` has
already written the feedback text, the quoted selections and their bodies to
`{data_dir}/feedback/<project>/index.jsonl` with a Markdown copy under `records/`, which is the
durable record and is shared with the Plannotator web app.

The order matters and is the whole safety argument. `archive_submission` reports whether it
wrote, and nothing is cleared unless it did, because `send.rs` already said the annotation store
is the recovery copy when the archive cannot write. With the archive off
(`PLANNOTATOR_FEEDBACK_HISTORY=0`) a send therefore keeps everything and says so in the status
line. A refused or failed delivery clears nothing either, matching what archiving already did.

Only *placed* annotations are cleared. An orphan - one whose quote no longer exists in the file -
was never in the feedback, so clearing it would discard something nobody has read.

`[review] clear_on_send` turns it off, because this is a divergence from decision 13 rather than a
correction of it: someone who wants the reviewer to keep showing what was sent can have that.

## 22. The reviewer is for agents to present to, so the tree is gone (2026-09-09)

The file tree was the last thing in the app that existed for browsing. Joan does not browse it:
agents present documents to it, and he reviews what arrived. So the tree is deleted rather than
hidden, and documents arrive as a **set** with one tab each above the header, `Tab` walking them
forward with a wrap. `Shift-Tab` was specified and then dropped: forward-only is enough for a handful
of documents, and the key is better spent elsewhere.

Tabs are named by file and grow a parent component only for the members of a colliding group, the way
editors disambiguate, so `plan.md` beside `notes.md` stays short even when two other documents in the
set collide. The footer spells out the whole path, home-relative, elided in the middle so the
basename survives: losing the file name would leave the least useful half of a path on screen.

A folder argument still works, expanding to the markdown files beneath it, because the CLI contract
predates this and a folder is a reasonable thing to hand over.

What made this more than deletion: the tree's row list was the **file list**. `E`, `record_delivery`
and `clear_sent` all walked it to find annotated files, and it fed per-file counts. Deleting it
without replacing that list would have made sends silently stop covering files, which is why
`docs.rs` owns it now and a test pins that every document is covered.

Tabs sit above the header, not below, because that is where nvim's tabline is and the reference was
explicit. The row is hidden entirely for a single document, so presenting one file costs no chrome.

## 23. Code blocks are laid out here too, and highlighted (2026-09-09)

`tui-markdown` rendered the fence as literal ```` ```bash ```` text (the `code_block_fence` hook was
never overridden) and a code block preserves columns, so a 138-character command showed 58 characters
and looked complete. A truncated shell command that looks whole is something you might copy and run.

So code blocks join tables in being laid out here: the fence is hidden, the language becomes a dim
label, a left rule marks the block, and long lines **wrap at the column edge** with a dim marker on
continuation rows. Breaking exactly at the edge keeps the characters unchanged, so a copy is the whole
command; breaking at spaces would read as if a quoted string had ended.

Highlighting is syntect, added **directly** and never through `tui-markdown`'s `highlight-code`
feature: that declares `syntect` with default features, which pulls oniguruma (C), and feature
unification means it cannot be switched off downstream. Direct with `regex-fancy` is pure Rust,
verified by the absence of `onig` and `bindgen` from the tree.

It costs ~2.5 MB of binary, measured with the dependency actually exercised. An earlier measurement
said +0 KB because nothing referenced it yet and LTO had stripped it: a dependency's cost cannot be
measured until something calls it.

`default-syntaxes` lacks HCL, Terraform and TOML, which are exactly what this vault reviews, so those
`.sublime-syntax` files are vendored with their source and licence recorded. Scopes map onto the nine
`syntax*` tokens pi already defines, so a block looks the same here as in the agent that wrote it.

`HIGHLIGHT_CEILING` bounds the work per document, because a 2,800-block stress corpus otherwise pays
unbounded cost; past the ceiling a block keeps its label, its layout and its source map, and loses
only colour. `[code] highlight = false` turns it off, though it cannot reclaim the binary size.

## 24. What changed is measured against `HEAD`, and git may be absent (2026-09-09)

The gutter's first column bars what differs from `HEAD`, so a review starts from what the agent
touched. `HEAD` rather than "as first presented", because that is what "changed since the last
commit" means and it covers staged and unstaged together. The block marker moves to the second
gutter column; the gutter was already two wide with one column unused, so nothing grew.

Four kinds, four theme tokens, not reusing the annotation kinds: added, changed, deleted drawn on the
row **following** the gap since a removed line has no row, and untracked in its own colour.

Untracked means git has never been told about the file, **not** absent from `HEAD`. Keying on `HEAD`
membership reported every `git add`ed file as untracked, when it is tracked and its lines are added,
which is what `gitsigns.nvim` shows and the whole reason untracked has a separate colour. A tracked
file in a repository before its first commit is all added: nothing to diff against, so every line is
new rather than unknown.

A row is not a source line, and the design's own wording ("a row's first mapped source byte gives
its line") was wrong. Prose reflows, so one row carries several source lines; asking only about the
first byte hid a modified line whenever an unchanged one started the row, which is most of a
paragraph. A row is barred if anything in it changed, most telling kind winning: changed over added,
untracked last. Found by demonstrating the feature, not by a test, and pinned on the drawn buffer
because the defect was in the drawing rather than in the parser or the map.

The parser is pure and the runner infallible. No repository, no `HEAD`, or no git at all gives no
bars and never an error, because a document must still render on a machine without git. One read per
document open and per `r`, never while drawing.

## 25. One table describes every key (2026-09-09)

The footer hint was hand-written prose in `draw.rs`, so it could disagree with the bindings, and it
did: it advertised `t hide` for a tree that had been deleted. The keys had also multiplied past the
point where a footer could carry them.

`KEYS` in `src/app/help.rs` is now the single description. The `?` overlay and the footer hint both
render from it, and a test walks `input.rs`, `compose.rs`, `pick.rs` and `help.rs` for every key the
handler answers to, failing the build when one is bound but not described. The exemption list carries
a reason per entry.

The overlay must **admit what it cannot show**. Its first version silently clipped two groups in a
short pane, which is precisely the drift the module exists to prevent, only worse: a help screen that
looks complete and is not. Scrolling is not required for a keymap; honest disclosure is, so shown
plus admitted always equals the total.

## 26. The tab row is clickable, and the footer advertises only `?` (2026-09-09)

Two changes pulling the same way: the tab row becomes usable with the mouse, and the footer stops
restating what the overlay already says in full.

`draw_tabs` records each visible tab's column span **as it lays it out**, because after truncation
and the hidden-count markers nothing else knows where a label ended up. A hidden tab has no span,
which is correct rather than a gap: there is nothing on screen to click, and `Tab` reaches it.
Clicking the tab already open returns focus to the document instead of re-reading the file, since
that is what clicking a tab means when you are in the notes rail.

The footer listed every hinted key for the current scope, which spent a third of a narrow row on a
list that `?` gives in full, and those columns are worth more to the document's path. It now carries
`?` alone. The shedding logic stays, because one item still has to fit or be dropped whole.

Three tests failed on that change and were **rewritten rather than joined by new ones**: the hint no
longer varies with focus, so the test that asserted it did now asserts it does not, and the
wide-footer test asserts the other keys are absent. Keeping a passing test beside a contradicting
one is how a suite starts describing two different programs.

The click test takes the span from `geometry` rather than guessing a column, so it cannot pass
against an empty tab row. Verified by mutation: recording no spans fails it.
