// Builds Tailwind CSS stylesheets as @tailwindcss/vite does, with the
// project's own `tailwindcss` compiler. NativeBoat compiles this script into
// a native tool for each compiler and set of plugins, and runs it during
// builds: it reads a JSON request with the stylesheet, its folder and the
// class candidates found in the sources, and writes the built stylesheet as
// JSON.
//
// The build substitutes the compiler's path for TAILWINDCSS, and the plugin
// and configuration modules the stylesheets name, imported statically, for
// MODULES: `[specifier, module]` pairs.
import { compile } from 'TAILWINDCSS';
import fs from 'node:fs';

const modules = new Map(MODULES);

async function build({ css, base, candidates }) {
  const compiler = await compile(css, {
    base,
    async loadModule(id, from) {
      if (!modules.has(id)) {
        throw new Error(`the plugin or configuration "${id}" was not found at build time`);
      }
      const module = modules.get(id);
      return { path: id, base: from, module: module.default ?? module };
    },
    async loadStylesheet(id) {
      throw new Error(`the stylesheet "${id}" should have been inlined`);
    },
  });
  return compiler.build(candidates);
}

const [request, output] = process.argv.slice(-2);
const body = JSON.parse(fs.readFileSync(request, 'utf8'));
build(body)
  .then((css) => fs.writeFileSync(output, JSON.stringify({ css })))
  .catch((error) => fs.writeFileSync(output, JSON.stringify({ errors: [String(error && error.message || error)] })));
