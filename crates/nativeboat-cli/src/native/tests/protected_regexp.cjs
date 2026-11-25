'use strict';
const {
  SideEffectFreeRegExpPrototypeExec: exec,
  SideEffectFreeRegExpPrototypeSymbolReplace: replace,
  SideEffectFreeRegExpPrototypeSymbolSplit: split,
} = require('internal/util');
const out = [];
let hooks = 0;
const regex = /a/g;
regex.exec = () => { hooks++; throw new Error('own exec'); };
regex[Symbol.replace] = () => { hooks++; throw new Error('own replace'); };
regex[Symbol.split] = () => { hooks++; throw new Error('own split'); };
Object.defineProperty(regex, 'source', { get() { hooks++; throw new Error('own source'); } });
Object.defineProperty(regex, 'flags', { get() { hooks++; throw new Error('own flags'); } });
out.push(exec(regex, 'aa')[0], regex.lastIndex);
out.push(replace(regex, 'aba', 'x'), regex.lastIndex);
out.push(split(regex, 'aba').join('|'), regex.lastIndex, hooks);
const sticky = /a/y;
out.push(replace(sticky, 'aa', 'x'), replace(sticky, 'aa', 'x'), sticky.lastIndex);
RegExp.prototype.exec = () => { hooks++; throw new Error('prototype exec'); };
RegExp.prototype[Symbol.replace] = () => { hooks++; throw new Error('prototype replace'); };
RegExp.prototype[Symbol.split] = () => { hooks++; throw new Error('prototype split'); };
out.push(exec(regex, 'aa')[0], regex.lastIndex);
out.push(replace(regex, 'aba', 'x'), split(regex, 'aba').join('|'), hooks);
console.log(JSON.stringify(out));
