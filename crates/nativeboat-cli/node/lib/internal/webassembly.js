'use strict';

// nativeboat: the WebAssembly JavaScript API, written for the native
// runtime over its own WebAssembly engine (internalBinding('wasm')):
// WebAssembly.Module, Instance, Memory, Table and Global, compile,
// instantiate, validate and their streaming forms, and the error classes,
// with V8's messages. Modules are compiled into the engine's instructions
// when they are made; their code runs in the engine, and imported
// JavaScript functions are called from it.

const binding = internalBinding('wasm');

class CompileError extends Error {}
class LinkError extends Error {}
class RuntimeError extends Error {}

for (const [ErrorClass, name] of [
  [CompileError, 'CompileError'],
  [LinkError, 'LinkError'],
  [RuntimeError, 'RuntimeError'],
]) {
  Object.defineProperty(ErrorClass.prototype, 'name', {
    value: name, writable: true, enumerable: false, configurable: true,
  });
}

function requireNew(target, name) {
  if (target === undefined) {
    throw new TypeError(`WebAssembly.${name} must be invoked with 'new'`);
  }
}

// WebIDL's [EnforceRange] unsigned long.
function enforceRange(value, context, name) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw new TypeError(`${context}: ${name} must be convertible to a valid number`);
  }
  const integer = Math.trunc(number);
  if (integer < 0 || integer > 0xFFFFFFFF) {
    throw new TypeError(`${context}: ${name} must be in the unsigned long range`);
  }
  return integer;
}

// A descriptor's size property, read as V8 reads it: `initial` or its
// alias `minimum`, and `maximum`, each within the type's upper bound.
function descriptorSize(descriptor, names, context, upper, required) {
  let value;
  let name = names[0];
  for (const candidate of names) {
    const read = descriptor[candidate];
    if (read !== undefined) {
      if (value !== undefined) {
        throw new TypeError(`${context}: The properties 'initial' and 'minimum' are not allowed at the same time`);
      }
      value = read;
      name = candidate;
    }
  }
  if (value === undefined) {
    if (required) throw new TypeError(`${context}: Property '${names[0]}' is required`);
    return undefined;
  }
  const number = enforceRange(value, context, `Property '${name}'`);
  if (number > upper) {
    throw new RangeError(`${context}: Property '${name}': value ${number} is above the upper bound ${upper}`);
  }
  return number;
}

function compileSync(source, context) {
  const result = binding.compile(source);
  if (result === undefined) throw new TypeError(`${context}: Argument 0 must be a buffer source`);
  if (typeof result === 'string') throw new CompileError(`${context}: ${result}`);
  return result;
}

function instantiateSync(module, importObject, context) {
  if (importObject !== undefined && (importObject === null || typeof importObject !== 'object') &&
      typeof importObject !== 'function') {
    throw new TypeError(`${context}: Argument 1 must be an object`);
  }
  const instance = binding.instantiate(module, importObject, context);
  Object.freeze(instance.exports);
  return instance;
}

class Module {
  constructor(bytes) {
    requireNew(new.target, 'Module');
    return compileSync(bytes, 'WebAssembly.Module()');
  }

  static imports(module) {
    return binding.moduleImports(module).map(({ module, name, kind }) => ({ module, name, kind }));
  }

  static exports(module) {
    return binding.moduleExports(module).map(({ name, kind }) => ({ name, kind }));
  }

  static customSections(module, sectionName) {
    if (sectionName === undefined) {
      binding.moduleExports(module);
      throw new TypeError('WebAssembly.Module.customSections(): Argument 1 is required');
    }
    return binding.customSections(module, sectionName);
  }
}

class Instance {
  constructor(module, importObject) {
    requireNew(new.target, 'Instance');
    if (!binding.isModule(module)) {
      throw new TypeError('WebAssembly.Instance(): Argument 0 must be a WebAssembly.Module');
    }
    return instantiateSync(module, importObject, 'WebAssembly.Instance()');
  }
}

class Memory {
  constructor(descriptor) {
    requireNew(new.target, 'Memory');
    const context = 'WebAssembly.Memory()';
    if (descriptor === null || typeof descriptor !== 'object') {
      throw new TypeError(`${context}: Argument 0 must be a memory descriptor`);
    }
    const initial = descriptorSize(descriptor, ['initial', 'minimum'], context, 65536, true);
    const maximum = descriptorSize(descriptor, ['maximum'], context, 65536, false);
    if (maximum !== undefined && maximum < initial) {
      throw new RangeError(`${context}: Property 'maximum': value ${maximum} is below the lower bound ${initial}`);
    }
    if (descriptor.shared) {
      throw new TypeError(`${context}: shared memory is not supported`);
    }
    return binding.newMemory(initial, maximum);
  }

  get buffer() {
    return binding.memoryBuffer(this);
  }

  grow(delta) {
    return binding.memoryGrow(this, enforceRange(delta, 'WebAssembly.Memory.grow()', 'Argument 0'));
  }
}

class Table {
  constructor(descriptor, value) {
    requireNew(new.target, 'Table');
    const context = 'WebAssembly.Table()';
    if (descriptor === null || typeof descriptor !== 'object') {
      throw new TypeError(`${context}: Argument 0 must be a table descriptor`);
    }
    const element = String(descriptor.element);
    if (element !== 'anyfunc' && element !== 'funcref' && element !== 'externref') {
      throw new TypeError(`${context}: Descriptor property 'element' must be a WebAssembly reference type`);
    }
    const initial = descriptorSize(descriptor, ['initial', 'minimum'], context, 10000000, true);
    const maximum = descriptorSize(descriptor, ['maximum'], context, 0xFFFFFFFF, false);
    if (maximum !== undefined && maximum < initial) {
      throw new RangeError(`${context}: Property 'maximum': value ${maximum} is below the lower bound ${initial}`);
    }
    return binding.newTable(element, initial, maximum, value);
  }

  get length() {
    return binding.tableLength(this);
  }

  get(index) {
    return binding.tableGet(this, enforceRange(index, 'WebAssembly.Table.get()', 'Argument 0'));
  }

  set(index, value) {
    const at = enforceRange(index, 'WebAssembly.Table.set()', 'Argument 0');
    if (arguments.length > 1) binding.tableSet(this, at, value);
    else binding.tableSet(this, at);
  }

  grow(delta, value) {
    const by = enforceRange(delta, 'WebAssembly.Table.grow()', 'Argument 0');
    return arguments.length > 1 ? binding.tableGrow(this, by, value) : binding.tableGrow(this, by);
  }
}

class Global {
  constructor(descriptor, value) {
    requireNew(new.target, 'Global');
    const context = 'WebAssembly.Global()';
    if (descriptor === null || typeof descriptor !== 'object') {
      throw new TypeError(`${context}: Argument 0 must be a global descriptor`);
    }
    const mutable = Boolean(descriptor.mutable);
    const type = String(descriptor.value);
    if (!['i32', 'i64', 'f32', 'f64', 'anyfunc', 'funcref', 'externref'].includes(type)) {
      throw new TypeError(`${context}: Descriptor property 'value' must be a WebAssembly type`);
    }
    return arguments.length > 1 ? binding.newGlobal(type, mutable, value) : binding.newGlobal(type, mutable);
  }

  get value() {
    return binding.globalGet(this);
  }

  set value(value) {
    binding.globalSet(this, value);
  }

  valueOf() {
    return binding.globalGet(this);
  }
}

binding.init({
  Module: Module.prototype,
  Instance: Instance.prototype,
  Memory: Memory.prototype,
  Table: Table.prototype,
  Global: Global.prototype,
  CompileError,
  LinkError,
  RuntimeError,
});

for (const [constructor, tag] of [
  [Module, 'WebAssembly.Module'],
  [Instance, 'WebAssembly.Instance'],
  [Memory, 'WebAssembly.Memory'],
  [Table, 'WebAssembly.Table'],
  [Global, 'WebAssembly.Global'],
]) {
  Object.defineProperty(constructor.prototype, Symbol.toStringTag, {
    value: tag, writable: false, enumerable: false, configurable: true,
  });
}

function validate(bytes) {
  return binding.validate(bytes);
}

function compile(bytes) {
  return new Promise((resolve) => resolve(compileSync(bytes, 'WebAssembly.compile()')));
}

function instantiate(source, importObject) {
  if (binding.isModule(source)) {
    return new Promise((resolve) => {
      resolve(instantiateSync(source, importObject, 'WebAssembly.instantiate()'));
    });
  }
  return new Promise((resolve) => {
    const module = compileSync(source, 'WebAssembly.instantiate()');
    resolve({ module, instance: instantiateSync(module, importObject, 'WebAssembly.instantiate()') });
  });
}

// The bytes of a fetched Response, checked as V8 checks streamed sources.
async function responseBytes(source, context) {
  const response = await source;
  const { Response } = require('internal/fetch');
  if (!(response instanceof Response)) {
    throw new TypeError(`${context}: Argument 0 must be provided and must be a Response`);
  }
  const type = response.headers.get('Content-Type');
  if (type === null || type.split(';')[0].trim().toLowerCase() !== 'application/wasm') {
    throw new TypeError(`${context}: Incorrect response MIME type. Expected 'application/wasm'.`);
  }
  if (!response.ok) {
    throw new TypeError(`${context}: HTTP status code is not ok`);
  }
  if (response.bodyUsed) {
    throw new TypeError(`${context}: Response body has already been used`);
  }
  return response.arrayBuffer();
}

async function compileStreaming(source) {
  const bytes = await responseBytes(source, 'WebAssembly.compileStreaming()');
  return compileSync(bytes, 'WebAssembly.compileStreaming()');
}

async function instantiateStreaming(source, importObject) {
  const bytes = await responseBytes(source, 'WebAssembly.instantiateStreaming()');
  const module = compileSync(bytes, 'WebAssembly.instantiateStreaming()');
  return { module, instance: instantiateSync(module, importObject, 'WebAssembly.instantiateStreaming()') };
}

const WebAssembly = {};
for (const [name, value] of Object.entries({
  compile,
  validate,
  instantiate,
  compileStreaming,
  instantiateStreaming,
  Module,
  Instance,
  Memory,
  Table,
  Global,
  CompileError,
  LinkError,
  RuntimeError,
})) {
  Object.defineProperty(WebAssembly, name, {
    value, writable: true, enumerable: false, configurable: true,
  });
}
Object.defineProperty(WebAssembly, Symbol.toStringTag, {
  value: 'WebAssembly', writable: false, enumerable: false, configurable: true,
});

module.exports = { WebAssembly };
