// Compiles Vue single-file components as @vitejs/plugin-vue does in a
// production build, with the project's own @vue/compiler-sfc. NativeBoat
// compiles this script into a native tool once per compiler version and
// runs it during builds: it reads a JSON request naming the components and
// writes their module code and stylesheets as JSON.
//
// The compiler import names its browser ESM build by absolute path; the build
// substitutes the project's copy for COMPILER_SFC.
import { parse, compileScript, compileTemplate, compileStyle } from 'COMPILER_SFC';
import fs from 'node:fs';
import crypto from 'node:crypto';
import path from 'node:path';

const slash = (file) => file.split('\\').join('/');

// plugin-vue's component id: the root-relative path and, in production, the
// source, hashed.
const componentId = (root, file, source) =>
  crypto.createHash('sha256').update(slash(path.relative(root, file)) + source).digest('hex').slice(0, 8);

// Reads for type resolution of imported types.
const typeFs = {
  fileExists: (file) => fs.existsSync(file),
  readFile: (file) => (fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : undefined),
};

const message = (error) => (typeof error === 'string' ? error : error.message);

const exportHelper =
  'const _export_sfc = (sfc, props) => { const target = sfc.__vccOpts || sfc; for (const [key, val] of props) { target[key] = val; } return target; };';

function compile(root, file) {
  const filename = slash(file);
  const source = fs.readFileSync(file, 'utf8');
  const { descriptor, errors } = parse(source, { filename, sourceMap: false });
  if (errors.length) return { errors: errors.map(message) };
  const id = componentId(root, file, source);
  descriptor.id = id;
  const scoped = descriptor.styles.some((style) => style.scoped);
  const lang = descriptor.scriptSetup?.lang || descriptor.script?.lang;
  const expressionPlugins = lang && /tsx?$/.test(lang) ? ['typescript'] : [];
  const templateOptions = (bindings) =>
    descriptor.template && {
      id,
      filename,
      scoped,
      slotted: descriptor.slotted,
      isProd: true,
      ssrCssVars: descriptor.cssVars,
      transformAssetUrls: { includeAbsolute: true },
      compilerOptions: {
        scopeId: scoped ? `data-v-${id}` : undefined,
        bindingMetadata: bindings,
        expressionPlugins,
        sourceMap: false,
      },
    };
  const output = [];
  const attached = [];
  const inline = !!descriptor.scriptSetup && !descriptor.template?.src;
  let bindings;
  if (descriptor.script || descriptor.scriptSetup) {
    const script = compileScript(descriptor, {
      id,
      isProd: true,
      inlineTemplate: inline,
      templateOptions: templateOptions(undefined),
      sourceMap: false,
      genDefaultAs: '_sfc_main',
      fs: typeFs,
    });
    bindings = script.bindings;
    output.push(script.content);
  } else {
    output.push('const _sfc_main = {}');
  }
  if (descriptor.template && !inline) {
    const template = compileTemplate({ ...templateOptions(bindings), source: descriptor.template.content });
    if (template.errors.length) return { errors: template.errors.map(message) };
    output.push(template.code.replace(/\nexport (function|const) (render|ssrRender)/, '\n$1 _sfc_$2'));
    attached.push(['render', '_sfc_render']);
  }
  const styles = [];
  for (const [index, style] of descriptor.styles.entries()) {
    if (style.src || (style.lang && style.lang !== 'css') || style.module) {
      return { errors: [`<style> block ${index} uses ${style.src ? 'src' : style.module ? 'module' : `lang="${style.lang}"`}, which native builds do not compile yet`] };
    }
    const compiled = compileStyle({ source: style.content, filename, id: `data-v-${id}`, scoped: style.scoped, isProd: true });
    if (compiled.errors.length) return { errors: compiled.errors.map(message) };
    styles.push(compiled.code);
  }
  if (scoped) attached.push(['__scopeId', JSON.stringify(`data-v-${id}`)]);
  if (attached.length) {
    output.push(exportHelper);
    output.push(`export default /*#__PURE__*/_export_sfc(_sfc_main, [${attached.map(([key, value]) => `['${key}',${value}]`).join(',')}])`);
  } else {
    output.push('export default _sfc_main');
  }
  return { code: output.join('\n'), styles };
}

// Packaged Electron apps receive no script path: the paths are the last two
// arguments under Node and natively alike.
const [requestPath, outputPath] = process.argv.slice(-2);
const request = JSON.parse(fs.readFileSync(requestPath, 'utf8'));
const results = {};
for (const file of request.files) {
  try {
    results[file] = compile(request.root, file);
  } catch (error) {
    results[file] = { errors: [message(error)] };
  }
}
fs.writeFileSync(outputPath, JSON.stringify(results));
