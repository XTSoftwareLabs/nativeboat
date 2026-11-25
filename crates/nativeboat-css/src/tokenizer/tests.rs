use super::*;

fn ident(name: &str) -> Token {
    Token::Ident(name.to_owned())
}

fn number(value: f64, integer: bool) -> Token {
    Token::Number { value, integer }
}

fn dimension(value: f64, unit: &str) -> Token {
    Token::Dimension {
        value,
        unit: unit.to_owned(),
    }
}

#[test]
fn punctuation_and_whitespace() {
    assert_eq!(
        tokens("( ) [ ] { } , : ; \t\n"),
        vec![
            Token::LeftParen,
            Token::Whitespace,
            Token::RightParen,
            Token::Whitespace,
            Token::LeftBracket,
            Token::Whitespace,
            Token::RightBracket,
            Token::Whitespace,
            Token::LeftBrace,
            Token::Whitespace,
            Token::RightBrace,
            Token::Whitespace,
            Token::Comma,
            Token::Whitespace,
            Token::Colon,
            Token::Whitespace,
            Token::Semicolon,
            Token::Whitespace,
        ]
    );
}

#[test]
fn numbers_percentages_and_dimensions() {
    assert_eq!(
        tokens("12 -3.5 +.5 .25 1e3 2E-2 3e+1 50% 10px -2em 1e 7.x"),
        vec![
            number(12.0, true),
            Token::Whitespace,
            number(-3.5, false),
            Token::Whitespace,
            number(0.5, false),
            Token::Whitespace,
            number(0.25, false),
            Token::Whitespace,
            number(1000.0, false),
            Token::Whitespace,
            number(0.02, false),
            Token::Whitespace,
            number(30.0, false),
            Token::Whitespace,
            Token::Percentage(50.0),
            Token::Whitespace,
            dimension(10.0, "px"),
            Token::Whitespace,
            dimension(-2.0, "em"),
            Token::Whitespace,
            dimension(1.0, "e"),
            Token::Whitespace,
            number(7.0, true),
            Token::Delim('.'),
            ident("x"),
        ]
    );
}

#[test]
fn identifiers_functions_and_at_keywords() {
    assert_eq!(
        tokens("color --main-bg -webkit-box rgb( @media @ - -1 _x é"),
        vec![
            ident("color"),
            Token::Whitespace,
            ident("--main-bg"),
            Token::Whitespace,
            ident("-webkit-box"),
            Token::Whitespace,
            Token::Function("rgb".to_owned()),
            Token::Whitespace,
            Token::AtKeyword("media".to_owned()),
            Token::Whitespace,
            Token::Delim('@'),
            Token::Whitespace,
            Token::Delim('-'),
            Token::Whitespace,
            number(-1.0, true),
            Token::Whitespace,
            ident("_x"),
            Token::Whitespace,
            ident("é"),
        ]
    );
}

#[test]
fn escapes_in_identifiers() {
    assert_eq!(
        tokens(r"\31 23 a\@b \0 \110000 -\41 \"),
        vec![
            ident("123"),
            Token::Whitespace,
            ident("a@b"),
            Token::Whitespace,
            ident("\u{fffd}\u{fffd}-A"),
            Token::Delim('\\'),
        ]
    );
    assert_eq!(tokens(" -\\41"), vec![Token::Whitespace, ident("-A")]);
    assert_eq!(tokens("\\\n"), vec![Token::Delim('\\'), Token::Whitespace]);
}

#[test]
fn hashes() {
    assert_eq!(
        tokens("#main #123 #-x #\\41 # "),
        vec![
            Token::Hash {
                value: "main".to_owned(),
                id: true
            },
            Token::Whitespace,
            Token::Hash {
                value: "123".to_owned(),
                id: false
            },
            Token::Whitespace,
            Token::Hash {
                value: "-x".to_owned(),
                id: true
            },
            Token::Whitespace,
            Token::Hash {
                value: "A".to_owned(),
                id: true
            },
            Token::Delim('#'),
            Token::Whitespace,
        ]
    );
}

#[test]
fn strings() {
    assert_eq!(
        tokens(
            r#""a\"b" 'c\'d' "\41 x" "tab\	" "line\
continued" "#
        ),
        vec![
            Token::String("a\"b".to_owned()),
            Token::Whitespace,
            Token::String("c'd".to_owned()),
            Token::Whitespace,
            Token::String("Ax".to_owned()),
            Token::Whitespace,
            Token::String("tab\t".to_owned()),
            Token::Whitespace,
            Token::String("linecontinued".to_owned()),
            Token::Whitespace,
        ]
    );
    assert_eq!(tokens("\"abc"), vec![Token::String("abc".to_owned())]);
    assert_eq!(
        tokens("'broken\nnext"),
        vec![Token::BadString, Token::Whitespace, ident("next")]
    );
    assert_eq!(tokens("'end\\"), vec![Token::String("end".to_owned())]);
}

#[test]
fn urls() {
    assert_eq!(
        tokens("url(a.png) URL( b.png  ) url(\"c.png\") url(d\\).png) url(e"),
        vec![
            Token::Url("a.png".to_owned()),
            Token::Whitespace,
            Token::Url("b.png".to_owned()),
            Token::Whitespace,
            Token::Function("url".to_owned()),
            Token::String("c.png".to_owned()),
            Token::RightParen,
            Token::Whitespace,
            Token::Url("d).png".to_owned()),
            Token::Whitespace,
            Token::Url("e".to_owned()),
        ]
    );
    assert_eq!(
        tokens("url(a b) url(a\"b) url(a\\\n) x url(a "),
        vec![
            Token::BadUrl,
            Token::Whitespace,
            Token::BadUrl,
            Token::Whitespace,
            Token::BadUrl,
            Token::Whitespace,
            ident("x"),
            Token::Whitespace,
            Token::Url("a".to_owned()),
        ]
    );
    assert_eq!(tokens("url(a\\"), vec![Token::BadUrl]);
}

#[test]
fn comments_and_html_markers() {
    assert_eq!(
        tokens("a/* x */b<!-- --> < /* open"),
        vec![
            ident("a"),
            ident("b"),
            Token::Cdo,
            Token::Whitespace,
            Token::Cdc,
            Token::Whitespace,
            Token::Delim('<'),
            Token::Whitespace,
        ]
    );
}

#[test]
fn offsets_are_recorded() {
    let spanned = tokenize("a  {b}");
    let offsets: Vec<usize> = spanned.iter().map(|token| token.offset).collect();
    assert_eq!(offsets, vec![0, 1, 3, 4, 5]);
}

#[test]
fn serialization() {
    let source = "a(#x \"q\\\"\" url(u) ! 1.5 50% 2px  <!-- --> : ; , [ ] ( ) { } @m";
    assert_eq!(
        serialize_tokens(&tokens(source)),
        "a(#x \"q\\\"\" url(u) ! 1.5 50% 2px <!-- --> : ; , [ ] ( ) { } @m"
    );
    assert_eq!(Token::BadString.to_css(), "");
    assert_eq!(Token::BadUrl.to_css(), "");
    assert_eq!(ident("1a").to_css(), "\\1a");
    assert_eq!(ident("a b").to_css(), "a\\ b");
    assert_eq!(Token::String("a\\b".to_owned()).to_css(), "\"a\\\\b\"");
    assert_eq!(format_number(3.0), "3");
    assert_eq!(format_number(-0.25), "-0.25");
    assert_eq!(format_number(1e20), "100000000000000000000");
}
