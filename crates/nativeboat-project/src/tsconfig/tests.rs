use super::*;
use crate::tests::fixture::Fixture;

fn emit(root_dir: &str, out_dir: &str) -> Emit {
    Emit {
        root_dir: root_dir.to_owned(),
        out_dir: out_dir.to_owned(),
    }
}

fn emits_of(fixture: &Fixture, package: &str) -> Vec<Emit> {
    emits(fixture.root(), &serde_json::from_str(package).unwrap())
}

#[test]
fn output_directories_mirror_the_source_root() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{ // The application.
                "compilerOptions": {"rootDir": ".", "outDir": "dist"},
                "include": ["src/**/*.ts"]}"#,
        ),
        ("src/main/main.ts", ""),
    ]);
    assert_eq!(emits_of(&fixture, "{}"), vec![emit("", "dist")]);
}

#[test]
fn source_roots_default_to_the_common_directory_of_the_program() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions": {"outDir": "out"}, "include": ["src"], "exclude": ["src/skipped"]}"#,
        ),
        ("src/main/main.ts", ""),
        ("src/main/window/window.ts", ""),
        ("src/main/types.d.ts", ""),
        ("src/main/page.html", ""),
        ("src/skipped/other.ts", ""),
        ("src/.hidden/other.ts", ""),
        ("src/node_modules/dep/index.ts", ""),
        // Without `allowJs`, JavaScript files are not part of the program.
        ("src/script.js", ""),
    ]);
    assert_eq!(emits_of(&fixture, "{}"), vec![emit("src/main", "out")]);
    // Listed files and the default `**/*` include.
    let listed = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions": {"outDir": "build", "allowJs": true}, "files": ["app/main.ts", "app/missing.ts"]}"#,
        ),
        ("app/main.ts", ""),
        ("lib/other.js", ""),
    ]);
    assert_eq!(emits_of(&listed, "{}"), vec![emit("app", "build")]);
    let everything = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions": {"outDir": "build", "allowJs": true}}"#,
        ),
        ("app/main.ts", ""),
        ("lib/other.js", ""),
        ("build/app/main.js", ""),
    ]);
    assert_eq!(emits_of(&everything, "{}"), vec![emit("", "build")]);
    // Without an output directory, files are emitted beside their sources;
    // a program without files emits nothing.
    let beside = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"include": ["src/*.ts", "missing/**/*"]}"#,
        ),
        ("src/main.ts", ""),
    ]);
    assert_eq!(emits_of(&beside, "{}"), vec![emit("src", "src")]);
    let empty = Fixture::new(&[("tsconfig.json", r#"{"files": []}"#)]);
    assert!(emits_of(&empty, "{}").is_empty());
}

#[test]
fn configurations_extend_others() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"extends": "./config/base", "include": ["src/**/*"]}"#,
        ),
        (
            "config/base.json",
            r#"{"extends": "@tsconfig/node20/tsconfig.json", "compilerOptions": {"outDir": "../dist"}}"#,
        ),
        (
            "node_modules/@tsconfig/node20/tsconfig.json",
            r#"{"compilerOptions": {"rootDir": "../../../src"}}"#,
        ),
        ("src/main.ts", ""),
        // Lists of bases; the later ones win.
        (
            "tsconfig.main.json",
            r#"{"extends": ["./config/base.json", "./config/other.json", "./config/missing.json"]}"#,
        ),
        (
            "config/other.json",
            r#"{"compilerOptions": {"outDir": "../out", "rootDir": "../src"}}"#,
        ),
        // Packages may name their configuration without its file name.
        (
            "tsconfig.package.json",
            r#"{"extends": "shared-config", "compilerOptions": {"rootDir": "src"}}"#,
        ),
        (
            "node_modules/shared-config/tsconfig.json",
            r#"{"compilerOptions": {"outDir": "../../lib"}}"#,
        ),
        // Cycles end.
        (
            "tsconfig.cycle.json",
            r#"{"extends": "./tsconfig.cycle.json", "compilerOptions": {"rootDir": "src", "outDir": "cycle"}}"#,
        ),
    ]);
    assert_eq!(
        emits_of(&fixture, "{}"),
        vec![
            emit("src", "cycle"),
            emit("src", "dist"),
            emit("src", "out"),
            emit("src", "lib"),
        ]
    );
}

#[test]
fn configurations_come_from_scripts_and_references() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"files": [], "references": [{"path": "./electron"}, {"path": "./tsconfig.web.json"}, {"path": "./missing.json"}, {}]}"#,
        ),
        (
            "electron/tsconfig.json",
            r#"{"compilerOptions": {"rootDir": ".", "outDir": "../dist/electron"}, "references": [{"path": ".."}]}"#,
        ),
        (
            "tsconfig.web.json",
            r#"{"compilerOptions": {"noEmit": true, "rootDir": "web"}}"#,
        ),
        (
            "tsconfig.types.json",
            r#"{"compilerOptions": {"emitDeclarationOnly": true, "rootDir": "src", "outDir": "types"}}"#,
        ),
        (
            "config/build.json",
            r#"{"compilerOptions": {"rootDir": "../src", "outDir": "../build"}}"#,
        ),
        ("tsconfig.txt", "not a configuration"),
    ]);
    let package = r#"{"scripts": {
        "build": "tsc -p config/build.json && tsc --build \"electron\" --verbose",
        "watch": "npx tsc --project ./config/build.json -w; vue-tsc -p other.json",
        "other": "tsc -p",
        "lint": "eslint ."}}"#;
    assert_eq!(
        emits_of(&fixture, package),
        vec![emit("src", "build"), emit("electron", "dist/electron")]
    );
    // Type checks name configurations that emit, but write nothing.
    let checks = r#"{"scripts": {
        "typecheck": "tsc --project ./config/build.json --noEmit --skipLibCheck",
        "types": "tsc -p config/build.json --emitDeclarationOnly && tsc -p tsconfig.types.json --noEmit false"}}"#;
    assert_eq!(
        emits_of(&fixture, checks),
        vec![emit("electron", "dist/electron")]
    );
    let not_scripts = r#"{"scripts": {"build": 1}}"#;
    assert_eq!(
        emits_of(&fixture, not_scripts),
        vec![emit("electron", "dist/electron")]
    );
}

#[test]
fn paths_outside_the_root_climb_out_of_it() {
    assert_eq!(
        relative_to(
            Path::new("C:/repo/apps/desktop"),
            Path::new("C:/repo/packages/shared")
        ),
        "../../packages/shared"
    );
    assert_eq!(
        relative_to(Path::new("C:/repo"), Path::new("c:/REPO/src")),
        "src"
    );
    assert_eq!(relative_to(Path::new("C:/repo"), Path::new("C:/repo")), "");
}
