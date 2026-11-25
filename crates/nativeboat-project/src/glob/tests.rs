use super::*;

fn check(pattern: &str, path: &str) -> bool {
    matches(&segments(pattern), &segments(path))
}

#[test]
fn patterns_match_paths() {
    assert!(check("src/**/*.ts", "src/main.ts"));
    assert!(check("src/**/*.ts", "src/a/b/main.ts"));
    assert!(check("SRC/*.TS", "src/main.ts"));
    assert!(check("a?c/x", "abc/x"));
    assert!(check("**", "anything/at/all"));
    assert!(check("./a//b", "a/b"));
    assert!(!check("src/*.ts", "src/a/main.ts"));
    assert!(!check("src/*.ts", "src/main.tsx"));
    assert!(!check("a?c", "ac"));
    assert!(!check("a/b", "a"));
    assert!(!check("a", "a/b"));
    for pattern in ["a*", "a?", "a@(b)", "+(a)", "!(b)"] {
        assert!(has_wildcard(pattern));
    }
    assert!(!has_wildcard("a"));
    assert_eq!(path_segments(Path::new("A/b")), segments("a/B"));
}

#[test]
fn extglobs_match_alternatives_repetition_and_negated_suffixes() {
    for (pattern, path, expected) in [
        ("locales/**/!(*.min).json", "locales/en.json", true),
        ("locales/**/!(*.min).json", "locales/en.min.json", false),
        ("locales/**/!(*.min).json", "locales/nested/de.json", true),
        (
            "locales/**/!(*.min).json",
            "locales/nested/de.min.json",
            false,
        ),
        ("locales/**/!(*.min).json", "preference.json", false),
        ("src/@(main|preload).?(c|m)js", "src/main.js", true),
        ("src/@(main|preload).?(c|m)js", "src/preload.cjs", true),
        ("src/@(main|preload).?(c|m)js", "src/renderer.js", false),
        ("x+(a|bc)y", "xabcbcy", true),
        ("x+(a|bc)y", "xy", false),
        ("x*(a|bc)y", "xy", true),
        ("x*(a|bc)y", "xay", true),
        ("x*(a|bc)y", "xacy", false),
        ("x?(a|bc)y", "xabcy", false),
        ("x@(a|+(bc))y", "xbcbcy", true),
        ("a!(b)*", "abc", false),
        ("a!(b)*", "axbc", true),
        ("a!(b|c)d", "abd", false),
        ("a!(b|c)d", "axbd", true),
        ("a@(i|w!(x|y)z|j)b", "awxzb", false),
        ("a@(i|w!(x|y)z|j)b", "awqzb", true),
        ("a+(|b)c", "ac", true),
        ("a*(|b)c", "abbbc", true),
        ("a@(|b)c", "ac", true),
        ("a!(|b)c", "ac", false),
        ("a!(|b)c", "abc", false),
        ("a!(|b)c", "axc", true),
        ("x!()*y", "xy", false),
        ("x!()*y", "xaby", true),
        ("a!(@(b|x))d", "ad", true),
        ("a!(@(b|x))d", "abd", false),
        ("a@(b", "a@(b", true),
        ("a*(b", "ax(b", true),
        ("a?(b", "ax(b", true),
        ("a@(b|c", "ab", false),
    ] {
        assert_eq!(check(pattern, path), expected, "{pattern}: {path}");
    }
    let repeated = "a".repeat(128);
    assert!(check("x+(a|aa)y", &format!("x{repeated}y")));
    assert!(!check("x+(a|aa)y", &format!("x{repeated}z")));
    let nested = format!("{}x{}", "@(".repeat(33), ")".repeat(33));
    assert!(!check(&nested, "x"));
}

#[test]
fn extglob_matrix_agrees_with_minimatch() {
    // Frozen results from electron-builder's matcher, with Windows path
    // separators, case-insensitive matching and explicit extglob negation.
    // No Node process or dependency installation is needed to run this test.
    // Pure nested-only negation is tested separately: Minimatch 10.2.6 treats
    // e.g. !(@(b|x)) as an empty group, contrary to extglob complement semantics.
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("tests/minimatch.json")).unwrap();
    let files = oracle["files"].as_array().unwrap();
    let mut mismatches = Vec::new();
    for case in oracle["cases"].as_array().unwrap() {
        let pattern = case[0].as_str().unwrap();
        let expected = case[1].as_str().unwrap().as_bytes();
        for (file, expected) in files.iter().zip(expected) {
            let file = file.as_str().unwrap();
            if check(pattern, file) != (*expected == b'1') {
                mismatches.push(format!(
                    "{pattern}: {file:?}, expected {}",
                    *expected == b'1'
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
