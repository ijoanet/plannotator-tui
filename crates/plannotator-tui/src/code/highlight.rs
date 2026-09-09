//! Syntax highlighting, mapped onto the theme's nine syntax tokens.
//!
//! syntect is used **directly** rather than through `tui-markdown`'s `highlight-code` feature:
//! that feature declares `syntect` with default features, which selects the Oniguruma regex
//! engine (a C dependency), and Cargo's feature unification means a dependent cannot switch it
//! back off. Asked for with `regex-fancy` instead, syntect is pure Rust.
//!
//! Only the syntax definitions are used, never syntect's themes. A theme here is nine semantic
//! tokens, so a scope is classified into one of those and the palette decides the color. That is
//! what makes highlighted code match the agent that produced it rather than a Sublime theme.
//!
//! Loading the syntax set costs real time and memory, so it happens once per process, lazily: a
//! document with no code block never pays for it.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxSet};

/// HCL and TOML, absent from `default-syntaxes`. See `VENDORED.md` for provenance and licences.
const TOML_SYNTAX: &str = include_str!("TOML.sublime-syntax");
const TERRAFORM_SYNTAX: &str = include_str!("Terraform.sublime-syntax");

/// What a character means, independent of any palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    /// Not classified: body text of the block.
    Plain,
    Comment,
    Keyword,
    Function,
    Variable,
    Str,
    Number,
    Type,
    Operator,
    Punctuation,
}

/// Scope prefix to token, **most specific first**: the first match wins, so
/// `variable.function` must be tested before `variable`, and `keyword.operator` before `keyword`.
///
/// `meta.*` is deliberately absent. It marks structure rather than meaning and appears in almost
/// every stack, so classifying it would colour whole lines.
const RULES: [(&str, Token); 14] = [
    ("comment", Token::Comment),
    ("keyword.operator", Token::Operator),
    ("keyword", Token::Keyword),
    ("storage.modifier", Token::Keyword),
    ("storage.type", Token::Type),
    ("support.type", Token::Type),
    ("support.function", Token::Function),
    ("entity.name.function", Token::Function),
    ("variable.function", Token::Function),
    ("entity.name", Token::Type),
    ("variable", Token::Variable),
    ("string", Token::Str),
    ("constant", Token::Number),
    ("punctuation", Token::Punctuation),
];

/// Tokens a language name maps to a syntax that `find_syntax_by_token` would miss.
///
/// Terraform's own extensions are `tf` and `nomad`, so an `hcl` fence would not find it.
const ALIASES: [(&str, &str); 4] = [("hcl", "tf"), ("terraform", "tf"), ("tfvars", "tf"), ("hcl2", "tf")];

/// Scopes that delegate to whatever they delimit rather than classifying themselves.
///
/// `punctuation.definition.comment` is the `#` of a comment and
/// `punctuation.definition.string` is a string's quote. Both are the innermost scope on the
/// character, so classifying them as punctuation would leave a comment's `#` and a string's
/// quotes a different color from the comment and the string. Every editor colors them with what
/// they open. `punctuation.section`, `.separator`, `.accessor` and the rest are punctuation in
/// their own right and are not listed here.
const DELEGATING: &str = "punctuation.definition";

struct Syntaxes {
    set: SyntaxSet,
    rules: Vec<(Scope, Token)>,
    delegating: Option<Scope>,
}

fn syntaxes() -> &'static Syntaxes {
    static SYNTAXES: OnceLock<Syntaxes> = OnceLock::new();
    SYNTAXES.get_or_init(|| {
        let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
        for (name, text) in [("TOML", TOML_SYNTAX), ("Terraform", TERRAFORM_SYNTAX)] {
            // A vendored syntax that will not load is not worth failing a document over: the
            // block still renders, just without colour.
            if let Ok(definition) = SyntaxDefinition::load_from_str(text, true, Some(name)) {
                builder.add(definition);
            }
        }
        let rules = RULES
            .iter()
            .filter_map(|&(selector, token)| Scope::new(selector).ok().map(|scope| (scope, token)))
            .collect();
        Syntaxes { set: builder.build(), rules, delegating: Scope::new(DELEGATING).ok() }
    })
}

/// Classify every character of `lines` for `language`, or `None` if it is not a language we know.
///
/// The result has one token per character of each line, so the caller can wrap and paint without
/// knowing anything about scopes.
pub(super) fn classify(language: &str, lines: &[String]) -> Option<Vec<Vec<Token>>> {
    let language = language.trim().to_lowercase();
    if language.is_empty() {
        return None;
    }
    let syntaxes = syntaxes();
    let token =
        ALIASES.iter().find(|(alias, _)| *alias == language).map_or(language.as_str(), |(_, target)| target);
    let syntax = syntaxes.set.find_syntax_by_token(token)?;

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        // syntect's syntaxes are the newline variants, so each line must carry its own.
        let with_newline = format!("{line}\n");
        let Ok(ops) = state.parse_line(&with_newline, &syntaxes.set) else {
            // A line that will not parse leaves the rest unclassified rather than mislabelled.
            out.push(vec![Token::Plain; line.chars().count()]);
            continue;
        };
        out.push(classify_line(line, &ops, &mut stack, syntaxes));
    }
    Some(out)
}

/// Walk one line's scope changes, recording the token in force at each character.
fn classify_line(
    line: &str,
    ops: &[(usize, syntect::parsing::ScopeStackOp)],
    stack: &mut ScopeStack,
    syntaxes: &Syntaxes,
) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::with_capacity(line.chars().count());
    let mut at = 0usize;
    for (offset, op) in ops {
        extend(&mut tokens, line, at, *offset, token_for(stack, syntaxes));
        let _ = stack.apply(op);
        at = *offset;
    }
    extend(&mut tokens, line, at, line.len(), token_for(stack, syntaxes));
    // A trailing newline the parser counted but the line does not have.
    tokens.truncate(line.chars().count());
    tokens
}

/// Push `token` once per character of `line[from..to]`.
fn extend(tokens: &mut Vec<Token>, line: &str, from: usize, to: usize, token: Token) {
    let Some(text) = line.get(from..to.min(line.len())) else { return };
    tokens.extend(std::iter::repeat_n(token, text.chars().count()));
}

/// The token for the innermost scope that any rule matches.
///
/// Innermost first, because the most specific scope is the one that describes the character:
/// inside a quoted string, an interpolated variable is still a variable. A delimiter scope is
/// skipped so the thing it delimits decides.
fn token_for(stack: &ScopeStack, syntaxes: &Syntaxes) -> Token {
    stack
        .scopes
        .iter()
        .rev()
        .filter(|scope| !syntaxes.delegating.is_some_and(|d| d.is_prefix_of(**scope)))
        .find_map(|scope| {
            syntaxes.rules.iter().find(|(selector, _)| selector.is_prefix_of(*scope)).map(|&(_, token)| token)
        })
        .unwrap_or(Token::Plain)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// The token each character of `line` was classified as, as a lookup by substring.
    fn token_of(language: &str, line: &str, needle: &str) -> Token {
        let tokens = classify(language, &[line.to_owned()]).expect("a known language");
        let row = tokens.first().expect("one line");
        let at = line.find(needle).expect("the needle is in the line");
        let index = line.get(..at).map_or(0, |p| p.chars().count());
        *row.get(index).expect("a token for it")
    }

    #[test]
    fn an_unknown_language_is_not_highlighted() {
        assert!(classify("", &["x".to_owned()]).is_none());
        assert!(classify("no-such-language", &["x".to_owned()]).is_none());
    }

    #[test]
    fn bash_words_are_classified_by_meaning() {
        let line = "gh pr list --limit 60 # note";
        assert_eq!(token_of("bash", line, "gh"), Token::Function, "a command is a call");
        assert_eq!(token_of("bash", line, "#"), Token::Comment, "a comment's marker is comment");
        assert_eq!(token_of("bash", line, "note"), Token::Comment);
        assert_eq!(token_of("bash", line, "--"), Token::Variable, "an option's dashes are its own");
    }

    #[test]
    fn a_delimiter_takes_the_color_of_what_it_delimits() {
        // The quotes belong to the string and the # to the comment, not to punctuation.
        assert_eq!(token_of("rust", r#"let s = "hi";"#, "\""), Token::Str);
        assert_eq!(token_of("bash", "echo x # done", "#"), Token::Comment);
        // Punctuation that delimits nothing is still punctuation.
        assert_eq!(token_of("toml", "[table]", "["), Token::Punctuation);
    }

    #[test]
    fn a_string_and_a_number_are_classified_in_rust() {
        assert_eq!(token_of("rust", r#"let s = "hi";"#, r#""hi""#), Token::Str);
        assert_eq!(token_of("rust", "let n = 42;", "42"), Token::Number);
        assert_eq!(token_of("rust", "pub fn f() {}", "pub"), Token::Keyword);
        assert_eq!(token_of("rust", "pub fn f() {}", "fn"), Token::Type, "fn is storage.type");
    }

    #[test]
    fn hcl_resolves_through_the_terraform_syntax() {
        // The fence people actually write is ```hcl, which Terraform does not claim.
        let line = r#"resource "aws_vpc" "main" {"#;
        assert_eq!(token_of("hcl", line, "resource"), Token::Keyword);
        assert_eq!(token_of("hcl", line, "aws_vpc"), Token::Type);
        assert_eq!(token_of("terraform", line, "resource"), Token::Keyword, "and by its own name");
    }

    #[test]
    fn toml_is_available_although_the_defaults_omit_it() {
        assert_eq!(token_of("toml", "n = 42", "42"), Token::Number);
        assert_eq!(token_of("toml", "[table]", "table"), Token::Type, "a section name");
    }

    #[test]
    fn every_character_of_a_line_gets_exactly_one_token() {
        let lines = vec!["# a comment".to_owned(), String::new(), "echo hi".to_owned()];
        let tokens = classify("bash", &lines).expect("bash");
        assert_eq!(tokens.len(), lines.len());
        for (line, row) in lines.iter().zip(&tokens) {
            assert_eq!(row.len(), line.chars().count(), "line {line:?}");
        }
    }

    #[test]
    fn a_more_specific_scope_wins_over_the_string_containing_it() {
        // An interpolated variable inside a double-quoted string is still a variable.
        assert_eq!(token_of("bash", r#"echo "$HOME/x""#, "HOME"), Token::Variable);
    }
}
