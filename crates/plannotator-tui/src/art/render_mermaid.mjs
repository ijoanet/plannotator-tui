// Renders every Mermaid diagram in a document to styled terminal rows using grok-mermaid.
//
// stdin:  {"sources":["graph TD ...", "sequenceDiagram ..."]}
// stdout: {"ok":true,"diagrams":[{"ok":true,"rows":[[{"cls","text"}]],"width":N}|{"ok":false},...]}
//       | {"ok":false,"kind":"renderer","error":"..."}
//
// One process per document, not per diagram: Node's startup dominates the cost of laying out
// a diagram by an order of magnitude, so a document with twenty diagrams must not pay it
// twenty times.
//
// A top-level `ok:false` means this setup can never render (no grok-mermaid) and the caller
// stops trying; a per-diagram `ok:false` means that one diagram did not render and the caller
// keeps the fence as written. Every failure exits 0: not rendering is not an error here.
//
// grok-mermaid is ESM-only and resolved out of PLANNOTATOR_MERMAID_BASE rather than this
// script's own location, because the script is embedded in the Rust binary and executed with
// `node -e`: it has no directory of its own to resolve from. `createRequire().resolve` honours
// the package's "exports" map without executing it, then a dynamic import loads the result.
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

function unusable(error) {
  process.stdout.write(JSON.stringify({ ok: false, kind: "renderer", error: String(error) }));
}

function load(base) {
  // createRequire resolves relative to a file, so a directory needs a trailing separator.
  const require = createRequire(pathToFileURL(base.endsWith("/") ? base : `${base}/`));
  return import(pathToFileURL(require.resolve("grok-mermaid")).href);
}

async function readStdin() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  return input;
}

function renderOne(render, source) {
  try {
    const art = render(source);
    if (!art || !Array.isArray(art.styled)) return { ok: false };
    return { ok: true, rows: art.styled, width: art.width ?? 0 };
  } catch {
    return { ok: false };
  }
}

async function main() {
  const base = process.env.PLANNOTATOR_MERMAID_BASE;
  if (!base) {
    unusable("PLANNOTATOR_MERMAID_BASE is not set");
    return;
  }

  let render;
  try {
    ({ render } = await load(base));
  } catch (error) {
    unusable(`grok-mermaid not resolvable from ${base}: ${error}`);
    return;
  }

  let sources;
  try {
    ({ sources } = JSON.parse(await readStdin()));
    if (!Array.isArray(sources)) throw new Error("sources is not an array");
  } catch (error) {
    unusable(`bad request: ${error}`);
    return;
  }

  const diagrams = sources.map((source) => renderOne(render, source));
  process.stdout.write(JSON.stringify({ ok: true, diagrams }));
}

main().catch(unusable);
