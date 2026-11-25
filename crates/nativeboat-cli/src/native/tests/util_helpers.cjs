'use strict';
const { promisify, customPromisifyArgs, getConstructorOf, kEnumerableProperty, removeColors } = require('internal/util');
const out = [];
const check = fn => { try { return fn(); } catch (e) { return `${e.name}:${e.code || ''}:${e.message}`; } };
for (const value of [null, undefined, false, '', {}, [], 1, new Date(0), Object.create(null)]) {
  const ctor = getConstructorOf(value);
  out.push(ctor ? ctor.name : null);
}
let reads = 0;
const inherited = { get constructor() { reads++; throw new Error('getter ran'); } };
out.push(getConstructorOf(Object.create(inherited)).name, reads);
function unnamed() {}
Object.defineProperty(unnamed, 'name', { value: '' });
out.push(getConstructorOf(Object.create({ constructor: unnamed })).name);
out.push(Object.isFrozen(kEnumerableProperty), Object.getPrototypeOf(kEnumerableProperty) === null,
         Object.keys(kEnumerableProperty).join(), kEnumerableProperty.enumerable);
out.push(removeColors('plain'), removeColors('\u001b[31mred\u001b[0m\u001b[123m'));
function work(value, callback) { callback(null, this.prefix + value); }
work.extra = 3;
const extraSymbol = Symbol('extra');
work[extraSymbol] = 7;
let metadataReads = 0;
Object.defineProperty(work, 'metadata', { get() { metadataReads++; return 4; }, configurable: true });
const wrapped = promisify(work);
out.push(wrapped.name, wrapped.length, wrapped.extra, Object.getPrototypeOf(wrapped) === Object.getPrototypeOf(work),
         promisify(wrapped) === wrapped, wrapped[promisify.custom] === wrapped);
out.push(metadataReads, wrapped[extraSymbol], wrapped.metadata, metadataReads);
function customized() {}
const custom = value => value * 2;
customized[promisify.custom] = custom;
out.push(promisify(customized) === custom, custom[promisify.custom] === custom,
         Object.getOwnPropertyDescriptor(custom, promisify.custom).writable);
let customReads = 0;
function getterCustom() {}
Object.defineProperty(getterCustom, promisify.custom, { get() { customReads++; return custom; } });
out.push(promisify(getterCustom) === custom, customReads);
customized[promisify.custom] = 1;
out.push(check(() => promisify(1)), check(() => promisify(customized)));
async function run() {
  out.push(await wrapped.call({ prefix: 'native:' }, 5));
  out.push(await promisify(callback => callback(null))());
  out.push(await promisify(callback => callback(null, 1, 2))());
  out.push(await promisify(callback => callback(0, 3))());
  for (const fn of [callback => callback('failed'), () => { throw new Error('thrown'); }]) {
    try { await promisify(fn)(); } catch (error) { out.push(typeof error === 'string' ? error : error.message); }
  }
  function many(callback) { callback(null, 1, 2); }
  many[customPromisifyArgs] = ['first', 'second'];
  out.push(JSON.stringify(await promisify(many)()));
  function single(callback) { callback(null, 3); }
  single[customPromisifyArgs] = ['first', 'second'];
  out.push(await promisify(single)());
  process.noDeprecation = true;
  out.push(await promisify(callback => { callback(null, 9); return Promise.resolve(10); })());
  process.noDeprecation = false;
  console.log(JSON.stringify(out));
}
run();
