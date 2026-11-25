//! `text!("file")` embeds a text file like `include_str!`, scrambled, so
//! the program holds no readable copy of it. The path is relative to the
//! file that uses the macro. The macro evaluates to a `fn() -> &'static str`
//! that unscrambles the text on first use; function pointers fit in
//! constant tables as string slices do.

use proc_macro::{Group, Literal, Span, TokenStream, TokenTree};
use std::path::PathBuf;

#[proc_macro]
pub fn text(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(output) => output,
        Err(message) => format!("compile_error!({message:?})")
            .parse()
            .expect("an error invocation parses"),
    }
}

fn expand(input: TokenStream) -> Result<TokenStream, String> {
    let mut tokens = input.into_iter();
    let (Some(TokenTree::Literal(literal)), None) = (tokens.next(), tokens.next()) else {
        return Err("text! takes one string literal: the file's path".to_owned());
    };
    let relative = unquote(&literal.to_string())
        .ok_or_else(|| "text! takes one string literal: the file's path".to_owned())?;
    let path = resolve(&relative)?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    std::str::from_utf8(&bytes).map_err(|_| format!("{} is not UTF-8", path.display()))?;
    let seed = seed(&bytes);
    let scrambled = scramble(&bytes, seed);
    let path = path
        .to_str()
        .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))?;
    // The unused constant makes the file a dependency of the build, so an
    // edit recompiles; constants nothing reads are not part of the program.
    let code = format!(
        "{{
            fn concealed() -> &'static str {{
                const _: &[u8] = include_bytes!({path:?});
                static DATA: &[u8] = SCRAMBLED;
                static TEXT: ::std::sync::OnceLock<::std::string::String> =
                    ::std::sync::OnceLock::new();
                TEXT.get_or_init(|| {{
                    let mut state: u64 = {seed}u64;
                    let bytes = DATA
                        .iter()
                        .map(|byte| {{
                            state ^= state << 13;
                            state ^= state >> 7;
                            state ^= state << 17;
                            byte ^ (state >> 56) as u8
                        }})
                        .collect();
                    ::std::string::String::from_utf8(bytes).expect(\"embedded text is UTF-8\")
                }})
            }}
            concealed as fn() -> &'static str
        }}"
    );
    let mut output = TokenStream::new();
    for token in code
        .parse::<TokenStream>()
        .map_err(|error| format!("text! expansion: {error}"))?
    {
        output.extend([replace(token, &scrambled)]);
    }
    Ok(output)
}

/// Puts the scrambled bytes where the expansion names `SCRAMBLED`.
fn replace(token: TokenTree, scrambled: &[u8]) -> TokenTree {
    match token {
        TokenTree::Ident(ident) if ident.to_string() == "SCRAMBLED" => {
            TokenTree::Literal(Literal::byte_string(scrambled))
        }
        TokenTree::Group(group) => {
            let stream = group
                .stream()
                .into_iter()
                .map(|token| replace(token, scrambled))
                .collect();
            let mut replaced = Group::new(group.delimiter(), stream);
            replaced.set_span(group.span());
            TokenTree::Group(replaced)
        }
        other => other,
    }
}

/// The contents of a plain string literal.
fn unquote(literal: &str) -> Option<String> {
    let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('\\')).then(|| inner.to_owned())
}

/// The file a path names, relative to the file that uses the macro.
fn resolve(relative: &str) -> Result<PathBuf, String> {
    let caller = Span::call_site()
        .local_file()
        .ok_or_else(|| "text! is used outside a source file".to_owned())?;
    let caller = if caller.is_absolute() {
        caller
    } else {
        std::env::current_dir()
            .map_err(|error| format!("text!: {error}"))?
            .join(caller)
    };
    let directory = caller
        .parent()
        .ok_or_else(|| format!("{} has no folder", caller.display()))?;
    Ok(directory.join(relative))
}

/// A nonzero starting state that differs between files.
fn seed(bytes: &[u8]) -> u64 {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    hash | 1
}

fn scramble(bytes: &[u8], seed: u64) -> Vec<u8> {
    let mut state = seed;
    bytes
        .iter()
        .map(|byte| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            byte ^ (state >> 56) as u8
        })
        .collect()
}
