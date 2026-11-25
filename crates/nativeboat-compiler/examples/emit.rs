use nativeboat_compiler::{ModuleKind, ModuleSource, compile_sources};
use std::path::{Path, PathBuf};

fn main() {
    let input = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(&input).unwrap();
    let module = compile_sources(
        Path::new("main.js"),
        &mut |_, _, _| None::<PathBuf>,
        &mut |_| {
            Some(ModuleSource {
                text: text.clone(),
                kind: ModuleKind::EsModule,
            })
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    print!("{}", module.rust_source());
    for m in module.module_rust_parts().iter().flatten() {
        print!("{m}");
    }
}
