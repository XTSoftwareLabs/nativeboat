'use strict';
const internalBinding = require('internal/binding');
const { markTransferMode } = require('internal/worker/js_transferable');
const metadata = require('internal/process/per_thread');
const binding = internalBinding('util');
const marker = binding.privateSymbols.transfer_mode_private_symbol;
const out = [metadata.platform, metadata.arch, metadata.version === process.version];
out.push(typeof marker, String(marker), marker === internalBinding('util').privateSymbols.transfer_mode_private_symbol);
out.push(binding.constants.kDisallowCloneAndTransfer, binding.constants.kTransferable, binding.constants.kCloneable);
for (const value of [undefined, null, 1, 'text', true, Symbol('x'), 1n]) {
  out.push(markTransferMode(value, true, true) === undefined);
}
for (const flags of [[false, false], [true, false], [false, true], [true, true]]) {
  const object = {};
  markTransferMode(object, ...flags);
  out.push(object[marker], Reflect.ownKeys(object).length, Object.getOwnPropertySymbols(object).length);
  const descriptor = Object.getOwnPropertyDescriptor(object, marker);
  out.push(descriptor.writable, descriptor.enumerable, descriptor.configurable);
}
for (const object of [Object.preventExtensions({}), Object.seal({}), Object.freeze({}), [], () => {}]) {
  markTransferMode(object, true, true);
  out.push(object[marker], Object.getOwnPropertySymbols(object).length);
}
const parent = {};
markTransferMode(parent, true);
const child = Object.create(parent);
out.push(child[marker] === undefined, marker in child, marker in parent);
markTransferMode(child, false, true);
out.push(child[marker], parent[marker]);
const copy = { ...parent };
out.push(copy[marker] === undefined, Object.assign({}, parent)[marker] === undefined, Reflect.ownKeys(Object.getOwnPropertyDescriptors(parent)).length);
Object.freeze(parent);
markTransferMode(parent, true, true);
out.push(parent[marker], Object.getOwnPropertyDescriptor(parent, marker).writable, delete parent[marker], parent[marker] === undefined);
try { Object.defineProperty(Object.preventExtensions({}), marker, { value: 1 }); } catch (error) { out.push(error.name); }
const restricted = {};
Object.defineProperty(restricted, marker, { value: 4, writable: false, configurable: false, enumerable: true });
Object.preventExtensions(restricted);
out.push(Object.getOwnPropertyDescriptor(restricted, marker).enumerable, Reflect.ownKeys(restricted).length);
try { markTransferMode(restricted, true); } catch (error) { out.push(error.name); }
try { delete restricted[marker]; } catch (error) { out.push(error.name); }
console.log(JSON.stringify(out));
