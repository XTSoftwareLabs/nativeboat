'use strict';
const qs = require('querystring');
const out = [];
const t = f => { try { return JSON.stringify(f()); } catch (e) { return `${e.name}:${e.code}:${e.message}`; } };
out.push(qs === require('node:querystring'), qs.encode === qs.stringify, qs.decode === qs.parse);
for (const text of ['', 'x y&=+%', '中🛤é', '\ud800', '\udc00', 'a\ud800z']) out.push(t(() => qs.escape(text)));
for (const value of [3, true, null, undefined, 1n, {}, { toString() { return 'custom text'; } }, Symbol('s')]) out.push(t(() => qs.escape(value)));
for (const text of ['a+b', 'x%20y', '%E4%B8%AD%F0%9F%9B%A4', '%', '%2', '%GG', '%C0%AF', '%FF', '%E2%82', '%ED%A0%80', '%00', '€']) {
  out.push(t(() => qs.unescape(text)), t(() => qs.unescape(text, true)), t(() => qs.unescapeBuffer(text, true).toString('hex')));
}
for (const value of [null, undefined, 3, [], {}, { a: 1, b: 'two words', c: ['x', 'y'], d: false },
  { a: null, b: undefined, c: NaN, d: Infinity, e: 1e21, f: 1n, g: {}, h: [] }]) out.push(t(() => qs.stringify(value)));
out.push(t(() => qs.stringify({ a: ['x', 'y'], b: 'a:b' }, ';', ':')));
out.push(t(() => qs.stringify({ a: ['x y', 'z'] }, '|', '~', { encodeURIComponent: s => `<${s}>` })));
out.push(t(() => qs.stringify({ a: 'x' }, '&', '=', { encodeURIComponent() { throw new Error('encoder'); } })));
for (const text of ['', 'a=1&b=two+words&a=3', '=x&&a&', '__proto__=safe&constructor=own',
  'a=%GG&a=%FF&b=%E2%82', 'a=b=c', 'a%20b=c%2Bd']) out.push(t(() => qs.parse(text)));
out.push(t(() => qs.parse('a:1;;a:2;;b:x:y', ';;', ':')));
for (const maxKeys of [0, 1, 2, -1, Infinity, NaN]) out.push(t(() => qs.parse('a=1&b=2&a=3', '&', '=', { maxKeys })));
out.push(t(() => Object.keys(qs.parse(Array.from({ length: 1003 }, (_, i) => `k${i}=x`).join('&'))).length));
out.push(t(() => qs.parse('a=x+y&b=z', '&', '=', { decodeURIComponent: s => `[${s}]` })));
out.push(t(() => qs.parse('a=%FF', '&', '=', { decodeURIComponent() { throw new Error('decoder'); } })));
out.push(Object.getPrototypeOf(qs.parse('a=1')) === null, t(() => qs.parse(3)));
const original = qs.escape;
qs.escape = s => String(s).toUpperCase();
out.push(t(() => qs.stringify({ key: 'value' })));
qs.escape = original;
console.log(out.join('\n'));
