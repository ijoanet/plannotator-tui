use syntect::parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxSet};

fn load(ss_builder: &mut syntect::parsing::SyntaxSetBuilder, path: &str, name: &str) {
    let text = std::fs::read_to_string(path).expect("read");
    match SyntaxDefinition::load_from_str(&text, true, Some(name)) {
        Ok(def) => {
            println!("LOADED {name}: scope={} exts={:?}", def.scope.build_string(), def.file_extensions);
            ss_builder.add(def);
        }
        Err(e) => println!("FAILED {name}: {e}"),
    }
}

fn main() {
    let mut b = SyntaxSet::load_defaults_newlines().into_builder();
    load(&mut b, "/tmp/toml.sublime-syntax", "TOML");
    load(&mut b, "/tmp/terraform.sublime-syntax", "Terraform");
    let ss = b.build();
    println!("total syntaxes: {}", ss.syntaxes().len());

    for (tok, code) in [
        ("toml", "# c\n[table]\nkey = \"val\"\nn = 42\nb = true\n"),
        ("tf", "# c\nresource \"aws_vpc\" \"main\" {\n  cidr_block = var.cidr\n  count = 3\n}\n"),
        ("hcl", "locals { x = 1 }\n"),
    ] {
        let Some(syntax) = ss.find_syntax_by_token(tok) else { println!("\n== {tok}: NO SYNTAX =="); continue };
        println!("\n== {tok} -> {} ==", syntax.name);
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        for line in code.lines() {
            let nl = format!("{line}\n");
            match state.parse_line(&nl, &ss) {
                Ok(ops) => {
                    let mut at = 0usize;
                    for (offset, op) in ops {
                        if offset > at {
                            let t = nl.get(at..offset).unwrap_or("");
                            if !t.trim().is_empty() {
                                let top: Vec<String> = stack.scopes.iter().map(|s| s.build_string()).collect();
                                println!("  {:?} <- {}", t, top.join(" "));
                            }
                        }
                        let _ = stack.apply(&op);
                        at = offset;
                    }
                }
                Err(e) => println!("  PARSE ERROR: {e}"),
            }
        }
    }
}
