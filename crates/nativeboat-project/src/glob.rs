//! Glob patterns, as `tsconfig.json` and electron-builder use them:
//! `*`, `?` and extglob groups match within a path segment; `**` matches any
//! number of segments. Matching ignores case, like the Windows file system.

use std::collections::BTreeSet;
use std::path::Path;

enum Token {
    Star,
    Any,
    Literal(char),
    Group(char, Vec<Vec<Token>>),
}

// Bound nesting in configuration input. Unclosed or excessively nested groups
// retain their literal punctuation and ordinary wildcard characters.
fn parse(pattern: &[char], offset: &mut usize, nested: bool, depth: usize) -> Vec<Token> {
    let mut tokens = Vec::new();
    while let Some(&character) = pattern.get(*offset) {
        if nested && matches!(character, '|' | ')') {
            break;
        }
        let start = *offset;
        *offset += 1;
        if depth < 32
            && matches!(character, '@' | '?' | '+' | '*' | '!')
            && pattern.get(*offset) == Some(&'(')
        {
            *offset += 1;
            let mut alternatives = Vec::new();
            loop {
                alternatives.push(parse(pattern, offset, true, depth + 1));
                match pattern.get(*offset) {
                    Some('|') => *offset += 1,
                    Some(')') => {
                        *offset += 1;
                        tokens.push(Token::Group(character, alternatives));
                        break;
                    }
                    _ => {
                        *offset = start + 1;
                        tokens.push(token(character));
                        break;
                    }
                }
            }
        } else {
            tokens.push(token(character));
        }
    }
    tokens
}

fn token(character: char) -> Token {
    match character {
        '*' => Token::Star,
        '?' => Token::Any,
        literal => Token::Literal(literal),
    }
}

/// Positions reachable after a sequence. Position sets avoid backtracking
/// through every possible partition of stars and repeated alternatives.
fn positions(pattern: &[Token], text: &[char], start: usize, tail: &[&[Token]]) -> BTreeSet<usize> {
    let mut current = BTreeSet::from([start]);
    for (index, token) in pattern.iter().enumerate() {
        let mut next = BTreeSet::new();
        let mut suffix = vec![&pattern[index + 1..]];
        suffix.extend_from_slice(tail);
        for position in current {
            match token {
                Token::Star => next.extend(position..=text.len()),
                Token::Any => {
                    if position < text.len() {
                        next.insert(position + 1);
                    }
                }
                Token::Literal(character) => {
                    if text.get(position) == Some(character) {
                        next.insert(position + 1);
                    }
                }
                Token::Group('!', alternatives) => {
                    // Minimatch negates the alternative AND the rest of the
                    // segment. Merely negating a consumed prefix would allow
                    // e.g. a!(b)* to match abc by consuming bc.
                    if alternatives.iter().all(Vec::is_empty) {
                        next.extend(position + 1..=text.len());
                    } else if !alternatives
                        .iter()
                        .any(|alternative| complete(alternative, text, position, &suffix))
                    {
                        next.extend(position..=text.len());
                    }
                }
                Token::Group(kind, alternatives) => {
                    if matches!(kind, '?' | '*') {
                        next.insert(position);
                    }
                    let mut visited = BTreeSet::from([position]);
                    let mut pending = vec![position];
                    while let Some(from) = pending.pop() {
                        for alternative in alternatives {
                            for end in positions(alternative, text, from, &suffix) {
                                next.insert(end);
                                if matches!(kind, '*' | '+') && visited.insert(end) {
                                    pending.push(end);
                                }
                            }
                        }
                    }
                }
            }
        }
        current = next;
    }
    current
}

fn complete(pattern: &[Token], text: &[char], start: usize, tail: &[&[Token]]) -> bool {
    let mut current = positions(pattern, text, start, tail);
    for (index, pattern) in tail.iter().enumerate() {
        current = current
            .into_iter()
            .flat_map(|position| positions(pattern, text, position, &tail[index + 1..]))
            .collect();
    }
    current.contains(&text.len())
}

/// Whether `text` matches a segment, including ordered extglob alternatives.
fn wildcard(pattern: &[char], text: &[char]) -> bool {
    let tokens = parse(pattern, &mut 0, false, 0);
    if let [Token::Group(kind, alternatives)] = tokens.as_slice()
        && *kind != '!'
        && alternatives.iter().all(Vec::is_empty)
    {
        // A wholly empty positive extglob is literal in Minimatch; embedded
        // empty groups can still match zero characters.
        return pattern == text;
    }
    complete(&tokens, text, 0, &[])
}

/// Whether path segments match pattern segments.
pub fn matches(pattern: &[Vec<char>], path: &[Vec<char>]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, rest)) if *first == ['*', '*'] => {
            (0..=path.len()).any(|skip| matches(rest, &path[skip..]))
        }
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(segment, tail)| wildcard(first, segment) && matches(rest, tail)),
    }
}

fn segment(text: &str) -> Vec<char> {
    text.to_lowercase().chars().collect()
}

/// The segments of a path.
pub fn path_segments(path: &Path) -> Vec<Vec<char>> {
    path.components()
        .map(|component| segment(&component.as_os_str().to_string_lossy()))
        .collect()
}

/// The segments of a `/`-separated pattern or relative path.
pub fn segments(pattern: &str) -> Vec<Vec<char>> {
    pattern
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .map(segment)
        .collect()
}

/// Whether a text contains wildcards.
pub fn has_wildcard(text: &str) -> bool {
    text.contains(['*', '?']) || ["@(", "+(", "!("].iter().any(|group| text.contains(group))
}

#[cfg(test)]
mod tests;
