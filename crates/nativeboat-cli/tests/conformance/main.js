// Differential checks of JavaScript behavior libraries rely on: each line
// prints a label and a value. expected.txt holds Node 22's output; the
// compiled program must print the same.
const out = (label, value) => {
  let text;
  try {
    text = typeof value === 'function' ? value() : value;
    text = typeof text === 'string' ? text : JSON.stringify(text);
  } catch (error) {
    text = 'THROWS ' + (error && error.constructor && error.constructor.name) + ': ' + (error && error.message);
  }
  console.log(label + ' => ' + text);
};
const tag = (value) => Object.prototype.toString.call(value);

// Type tags and typeof
out('tags', [/a/, new Date(0), [], {}, null, undefined, 1, 's', true, Symbol.iterator, 1n, () => 1,
  new Map(), new Set(), new WeakMap(), new Error('e'), new TypeError('t'), Promise.resolve(), new Uint8Array(1),
  new ArrayBuffer(1), new DataView(new ArrayBuffer(1)), JSON, Math, Reflect].map(tag));
out('typeof', [null, [], () => 1, class {}, 1n, Symbol(), undefined].map((v) => typeof v));
out('instanceof', [[] instanceof Array, /a/ instanceof RegExp, new Date() instanceof Date, Object.create(null) instanceof Object]);

// Object and property semantics
out('keys order', Object.keys({ b: 1, 2: 1, a: 1, 1: 1, [Symbol('s')]: 1 }));
out('entries', Object.entries({ x: 1, y: [2] }));
out('assign', Object.assign({}, { a: 1 }, null, 'xy', { b: 2 }));
out('spread', { ...{ a: 1, b: 2 }, ...'hi', c: 3 });
out('freeze', () => { const o = Object.freeze({ a: 1 }); o.a = 2; return [o.a, Object.isFrozen(o)]; });
out('defineProperty', () => {
  const o = {}; Object.defineProperty(o, 'x', { get() { return 42; }, enumerable: false });
  return [o.x, Object.keys(o), JSON.stringify(o), Object.getOwnPropertyNames(o)];
});
out('descriptors', Object.getOwnPropertyDescriptor({ get a() { return 1; } }, 'a').get.name);
out('proto', [Object.getPrototypeOf([]) === Array.prototype, Object.getPrototypeOf(Object.create(null))]);
out('hasOwn', [Object.hasOwn({ a: 1 }, 'a'), ({}).hasOwnProperty.call({ b: 1 }, 'b'), 'toString' in {}]);
out('fromEntries', Object.fromEntries(new Map([['a', 1], ['b', 2]])));
out('groupBy', () => Object.groupBy([1, 2, 3, 4], (n) => (n % 2 ? 'odd' : 'even')));
out('structuredClone', () => structuredClone({ a: [1, { b: 2 }], d: new Date(0) }));

// JSON
out('json', JSON.stringify({ a: undefined, b: () => 1, c: NaN, d: -0, e: [undefined, () => 1], f: new Date(0), g: 'é ' }));
out('json indent', JSON.stringify({ a: [1, { b: 2 }] }, null, 2));
out('json replacer', JSON.stringify({ a: 1, b: 2, c: { a: 3 } }, ['a', 'c']));
out('json parse reviver', JSON.parse('{"a":1,"b":[2,3]}', (k, v) => (typeof v === 'number' ? v * 10 : v)));
out('json errors', () => { try { JSON.parse('{bad'); } catch (e) { return e instanceof SyntaxError; } });
out('json bigint', () => JSON.stringify({ a: 1n }));
out('json toJSON', JSON.stringify({ toJSON() { return { x: 1 }; } }));

// Numbers
out('number format', [0.1 + 0.2, 1e21, 1e-7, 123.456.toFixed(1), (0.000001234).toPrecision(2), (255).toString(16),
  (255).toString(2), (-7.5).toString(36), Number('0x1f'), Number(' 12 '), Number(''), Number('1e3'), parseInt('08'),
  parseFloat('3.14abc'), (1234.5678).toFixed(2), (1e21).toFixed(2), Number.MAX_SAFE_INTEGER, 5e-324, -0 + '']);
out('locale number', () => (1234567.891).toLocaleString('en-US'));
out('math', [Math.round(-0.5), Math.round(2.5), Math.max(), Math.min(), Math.sign(-3), Math.trunc(-4.7), Math.hypot(3, 4),
  Math.cbrt(27), Math.clz32(1), Math.fround(5.5), Math.imul(3, 4), Math.log2(8), Math.expm1(0), 2 ** 10, (-8) % 3]);
out('bigint', [(2n ** 64n).toString(), (-7n / 2n).toString(), BigInt('123') + 1n + '', BigInt.asUintN(8, 257n) + '']);

// Strings
out('string methods', ['abc'.padStart(5, '-'), 'abc'.at(-1), ' x '.trim(), 'a-b-c'.split('-', 2), 'aaa'.replaceAll('a', 'b'),
  'Hello'.localeCompare('hello'), 'ß'.toUpperCase(), 'İ'.toLowerCase().length, 'abc'.codePointAt(1), '😀'.length,
  [...'😀a'].length, 'abc'.substring(2, 0), 'abc'.substr(-2, 1), 'a,b'.split(/(,)/), String.raw`a\nb`, 'x'.repeat(3),
  'abc'.normalize('NFD'), 'é'.normalize('NFD').length, 'Abc'.startsWith('b', 1), encodeURIComponent('a b&é'),
  decodeURIComponent('%E4%B8%AD'), escape('ä'), 'aBc'.toLocaleUpperCase()]);
out('template', `${1}${'a'}${null}${undefined}${[1, 2]}${{}}`);
out('string compare', ['a' < 'B', 'Z' < 'a', ['b', 'a', 'C', 'é'].sort(), ['b', 'a', 'C', 'é'].sort((a, b) => a.localeCompare(b))]);

// Regular expressions
out('regex groups', /(?<year>\d{4})-(?<month>\d\d)/.exec('on 2024-05').groups);
out('regex lookbehind', 'price $10 and $20'.match(/(?<=\$)\d+/g));
out('regex sticky', () => { const r = /a/y; r.lastIndex = 1; return [r.test('ba'), r.lastIndex]; });
out('regex unicode', [/\u{1F600}/u.test('😀'), /^.$/u.test('😀'), /^.$/.test('😀'), /\p{L}+/u.exec('中文abc')[0]]);
out('regex matchAll', [...'a1b2c3'.matchAll(/[a-z](\d)/g)].map((m) => [m[0], m[1], m.index]));
out('regex replace fn', 'a-b-c'.replace(/-(\w)/g, (_, c, offset) => c.toUpperCase() + offset));
out('regex replace patterns', 'John Smith'.replace(/(\w+)\s(\w+)/, '$2, $1 $$ $&'));
out('regex split', 'a1b22c333'.split(/\d+/));
out('regex flags', [/a/gimsuy.flags, new RegExp('a', 'dg').hasIndices, /x/.source, new RegExp('/').source, RegExp('').source]);
out('regex lastIndex', () => { const r = /o/g; const s = 'foo'; const seen = []; let m; while ((m = r.exec(s))) seen.push(m.index); return seen; });
out('regex backreference', /(a)\1/.test('aa'));
out('regex dotall', /a.b/s.test('a\nb'));
out('regex empty', 'abc'.split(''));

// Arrays
out('array methods', [[3, 1, 10, 2].sort(), [3, 1, 10, 2].sort((a, b) => a - b), [1, [2, [3, [4]]]].flat(2),
  [1, 2, 3].findLast((n) => n < 3), [1, 2, 3].at(-1), Array.from({ length: 3 }, (_, i) => i * 2),
  [, 1].includes(undefined), [NaN].indexOf(NaN), [NaN].includes(NaN), [1, 2, 3].toReversed(), [1, 2, 3].with(1, 9),
  [5, 1, 4].toSorted(), Array.of(7), [1, 2, 3].reduceRight((a, b) => a + b), [1, 2, 3, 4].copyWithin(0, 2)]);
out('array holes', [[, 1].length, Object.keys([, 1]), [, 1].map((x) => 2), JSON.stringify([, 1]), [1, , 3].join()]);
out('array sort stable', [{ k: 1, v: 'a' }, { k: 0, v: 'b' }, { k: 1, v: 'c' }].sort((x, y) => x.k - y.k).map((x) => x.v));
out('array length', () => { const a = [1, 2, 3]; a.length = 1; a[4] = 5; return [a.length, a]; });
out('array iter', [...[1, 2].entries(), ...[1, 2].keys()]);

// Dates
out('date', () => { const d = new Date(Date.UTC(2020, 1, 29, 12, 30, 15, 250)); return [d.toISOString(), d.getTime(), d.getUTCDay(), JSON.stringify(d)]; });
out('date parse', [Date.parse('2020-01-01'), Date.parse('2020-01-01T00:00:00Z'), Date.parse('garbage'), new Date(NaN).getTime()]);

// Classes, functions, scope
out('classes', () => {
  class A { #x = 1; static s = 'S'; get x() { return this.#x; } static make() { return new this(); } toString() { return 'A!'; } }
  class B extends A { constructor() { super(); this.y = 2; } }
  const b = B.make();
  return [b.x, b.y, B.s, `${b}`, b instanceof A, Object.getPrototypeOf(B) === A, B.name, typeof A];
});
out('function props', [function named(a, b = 1, ...c) {}.length, (() => 1).name, (function () {}).name, Math.max.name]);
out('closures', (() => { const fns = []; for (let i = 0; i < 3; i++) fns.push(() => i); return fns.map((f) => f()); })());
out('var hoist', (() => { const r = typeof hoisted; var hoisted = 1; return r; })());
out('this', (function () { 'use strict'; return this; })());
out('getter this', ({ v: 3, get w() { return this.v * 2; } }).w);
out('optional', [({ a: { b: null } }).a?.b?.c, null ?? 'd', 0 || 'e', 0 ?? 'f', ({}).x?.()]);
out('destructure', (() => { const { a = 1, b: { c } = { c: 2 }, ...rest } = { d: 4, e: 5 }; const [x, , y = 9, ...z] = [1, 2, undefined, 4, 5]; return [a, c, rest, x, y, z]; })());
out('labels', (() => { let n = 0; outer: for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) { if (j === 1) continue outer; if (i === 2) break outer; n++; } return n; })());
out('switch', ((v) => { switch (v) { case 1: return 'one'; case '1': return 'string'; default: return 'other'; } })('1'));
out('exceptions', () => { try { null.x; } catch (e) { return [e instanceof TypeError, e.name]; } });
out('finally', (() => { try { return 'try'; } finally { console.log('finally ran'); } })());
out('error props', () => { const e = new Error('msg', { cause: 'why' }); return [e.message, e.cause, e.name, String(e), 'stack' in e]; });

// Iterators, generators, collections
out('generator', (() => { function* g() { const x = yield 1; yield x * 2; return 3; } const it = g(); return [it.next(), it.next(5), it.next(), it.next()]; })());
out('spread iter', [...new Set([1, 1, 2]), ...new Map([[1, 'a']]).values()]);
out('map order', (() => { const m = new Map(); m.set('b', 1).set('a', 2); m.delete('b'); m.set('b', 3); return [...m]; })());
out('set ops', (() => { const s = new Set([1, 2]); s.add(NaN); s.add(NaN); s.add(-0); return [s.size, s.has(0), [...s]]; })());
out('symbol', [Symbol('d').description, Symbol.for('k') === Symbol.for('k'), Object.getOwnPropertySymbols({ [Symbol.iterator]: 1 }).length]);
out('proxy', (() => { const p = new Proxy({}, { get: (t, k) => `got ${String(k)}`, has: () => true }); return [p.foo, 'x' in p]; })());
out('reflect', [Reflect.ownKeys({ a: 1, [Symbol.iterator]: 1 }).length, Reflect.has({ a: 1 }, 'a'), Reflect.apply(Math.max, null, [1, 3])]);
out('typed arrays', [Array.from(new Uint8Array([256, -1, 3.7])), new Float32Array([1.1])[0], new Int16Array(new Uint8Array([1, 2]).buffer)[0]]);

// Coercion and equality
out('coercion', [1 + '2', '3' * '4', [] + {}, [1] == 1, null == undefined, null == 0, NaN == NaN, '' == 0, [] == false,
  +[], +{}, `${[1, [2, 3]]}`, String(Symbol('x')), 1 < 2 < 3, 3 > 2 > 1, '10' < '9', 10 < 9]);
out('object to primitive', `${{ toString() { return 'ts'; }, valueOf() { return 'vo'; } }}` + ({ valueOf() { return 5; } } + 1));

// Async ordering
const order = [];
setTimeout(() => order.push('timeout'), 0);
Promise.resolve().then(() => order.push('micro1')).then(() => order.push('micro2'));
queueMicrotask(() => order.push('queued'));
(async () => { order.push('async start'); await null; order.push('after await'); })();
order.push('sync');
setTimeout(() => {
  out('event order', order);
  Promise.allSettled([Promise.reject(new Error('x')), Promise.resolve(1)]).then((r) => out('allSettled', r.map((x) => x.status)));
  Promise.any([Promise.reject(1), Promise.resolve(2)]).then((v) => out('any', v));
}, 10);
out('names', (() => {
  const o = { m() {}, f: function () {}, a: () => 1, get g() { return 1; }, set g(v) {}, 'q-r'() {}, 5() {}, [Symbol('k')]() {} };
  const g = Object.getOwnPropertyDescriptor(o, 'g');
  const sym = Object.getOwnPropertySymbols(o)[0];
  const arrow = () => 1; let later; later = function () {};
  class C { method() {} static s() {} get v() { return 1; } #p() {} }
  return [o.m.name, o.f.name, o.a.name, g.get.name, g.set.name, o['q-r'].name, o[5].name, o[sym].name, arrow.name, later.name,
    C.prototype.method.name, C.s.name, Object.getOwnPropertyDescriptor(C.prototype, 'v').get.name, C.name];
})());
out('clone types', () => {
  const c = structuredClone({ m: new Map([[1, 'a']]), s: new Set([2]), r: /x/g, e: new TypeError('t'), u: new Uint8Array([1, 2]), n: new Number(4) });
  return [c.m.get(1), c.s.has(2), String(c.r), c.e instanceof TypeError, c.e.message, Array.from(c.u), typeof c.n, c.n + 1];
});
out('for-in order', (() => { const o = Object.create({ inherited: 1 }); o.b = 1; o[2] = 1; o.a = 1; const k = []; for (const x in o) k.push(x); return k; })());
out('toPrimitive', `${{ [Symbol.toPrimitive](hint) { return hint; } }}` + ({ [Symbol.toPrimitive](hint) { return hint; } } + ''));
out('new.target', (() => { function F() { return new.target === F; } return [new F() instanceof F, F()]; })());
out('super', (() => { class A { greet() { return 'A'; } static who() { return 'sA'; } } class B extends A { greet() { return super.greet() + 'B'; } static who() { return super.who() + 'B'; } } return [new B().greet(), B.who()]; })());
out('static block', (() => { class S { static v; static { S.v = 'init'; } } return S.v; })());
out('tagged', ((s, ...v) => [s.raw, v])`a${1}\n${2}`);
out('getter inherit', (() => { class A { get x() { return 1; } } class B extends A {} return [new B().x, 'x' in new B(), Object.keys(new B())]; })());
out('defineProperties', Object.keys(Object.defineProperties({}, { a: { value: 1, enumerable: true }, b: { value: 2 } })));
out('array misc', [Array(3).length, [1, , 3].map((x) => x * 2), [undefined, 3, null, 1].sort(), ['b', undefined, 'a'].sort(), [3, 1, 2].sort(() => 0)]);
out('delete', (() => { const a = [1, 2, 3]; delete a[1]; return [a.length, 1 in a, a]; })());
out('url', (() => { const u = new URL('https://user:pw@example.com:8080/p/a%20th?q=1&r=a+b#h'); return [u.host, u.pathname, u.searchParams.get('r'), u.hash, u.origin, String(new URL('../x', 'https://a.b/c/d/e'))]; })());
out('urlsearchparams', (() => { const p = new URLSearchParams('a=1&b=2&a=3'); p.append('c', 'x y'); return [p.getAll('a'), p.toString()]; })());
out('text encoding', [Array.from(new TextEncoder().encode('é中')), new TextDecoder().decode(new Uint8Array([228, 184, 173]))]);
out('date local', (() => { const d = new Date(2020, 0, 15, 10, 30); return [d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getDay(), typeof d.getTimezoneOffset()]; })());
out('date methods', (() => { const d = new Date(Date.UTC(2021, 11, 31)); d.setUTCDate(d.getUTCDate() + 1); return [d.toISOString(), new Date(0).toUTCString()]; })());
out('intl', () => [new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(1234.5), new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).format(new Date(0))]);
out('weakref', typeof WeakRef);
out('globalThis', [typeof globalThis.setTimeout, typeof globalThis.queueMicrotask, typeof globalThis.structuredClone, typeof globalThis.atob, typeof btoa === 'function' ? btoa('hi') : 'none', typeof AbortController]);
out('exponent', [2 ** 3 ** 2, (-2) ** 2]);
out('comma void', [(1, 2), void 0]);
out('getter setter class', (() => { class T { #v = 0; get v() { return this.#v; } set v(x) { this.#v = x * 2; } static #count = 0; static inc() { return ++T.#count; } static has(o) { return #v in o; } } const t = new T(); t.v = 5; return [t.v, T.inc(), T.inc(), T.has(t), T.has({})]; })());
out('async iter', 'pending');
(async () => {
  async function* gen() { yield 1; await null; yield 2; }
  const values = [];
  for await (const v of gen()) values.push(v);
  out('for await', values);
  out('promise race', await Promise.race([new Promise((r) => setTimeout(() => r('slow'), 20)), Promise.resolve('fast')]));
  try { await Promise.reject(new Error('rejected')); } catch (e) { out('await reject', e.message); }
})();
out('segmenter grapheme', [...new Intl.Segmenter('en', { granularity: 'grapheme' }).segment('e\u0301👨‍👩‍👧a')].map((s) => [s.segment, s.index]));
out('segmenter word', [...new Intl.Segmenter('en', { granularity: 'word' }).segment("Hello, world! It's 3.5")].map((s) => [s.segment, s.isWordLike]));
out('segmenter sentence', [...new Intl.Segmenter('en', { granularity: 'sentence' }).segment('One. Two? Three!')].map((s) => s.segment));
out('segmenter containing', (() => { const s = new Intl.Segmenter('en', { granularity: 'word' }).segment('ab cd'); return [s.containing(4).segment, s.containing(9), new Intl.Segmenter('en').resolvedOptions()]; })());
out('collator', [['b', 'a', 'B', 'ä', 'z', 'Z'].sort(new Intl.Collator('en').compare), ['a10', 'a2', 'a1'].sort(new Intl.Collator('en', { numeric: true }).compare),
  new Intl.Collator('en', { sensitivity: 'base' }).compare('a', 'Á'), new Intl.Collator('de').compare('ä', 'z'), new Intl.Collator('en').resolvedOptions().sensitivity]);
out('number format', [new Intl.NumberFormat('en-US').format(1234567.891), new Intl.NumberFormat('de-DE').format(1234567.891),
  new Intl.NumberFormat('en-US', { style: 'currency', currency: 'EUR' }).format(-5.5), new Intl.NumberFormat('ja-JP', { style: 'currency', currency: 'JPY' }).format(1234.5),
  new Intl.NumberFormat('en-US', { style: 'percent' }).format(0.256), new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 }).format(2.55),
  new Intl.NumberFormat('en-US', { minimumFractionDigits: 2 }).format(3), new Intl.NumberFormat('en-US', { notation: 'compact' }).format(12345),
  new Intl.NumberFormat('en-US', { notation: 'compact', compactDisplay: 'long' }).format(1500000), new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer-per-hour' }).format(50),
  new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 }).format(123456), new Intl.NumberFormat('en-US', { useGrouping: false }).format(12345),
  new Intl.NumberFormat('en-US', { signDisplay: 'always' }).format(5), new Intl.NumberFormat('en-IN').format(12345678), new Intl.NumberFormat('en-US', { minimumIntegerDigits: 3 }).format(7)]);
out('number resolved', (() => { const o = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'usd' }).resolvedOptions(); return [o.locale, o.currency, o.minimumFractionDigits, o.maximumFractionDigits]; })());
out('date format', (() => { const d = new Date(Date.UTC(2024, 2, 5, 14, 7, 9)); const f = (l, o) => new Intl.DateTimeFormat(l, { timeZone: 'UTC', ...o }).format(d); return [f('en-US'), f('en-GB'), f('de-DE'), f('ja-JP'),
  f('en-US', { dateStyle: 'full' }), f('en-US', { dateStyle: 'medium', timeStyle: 'short' }), f('en-US', { year: 'numeric', month: 'long', day: 'numeric' }),
  f('en-US', { hour: '2-digit', minute: '2-digit' }), f('en-US', { hour: 'numeric', hour12: false }), f('en-US', { weekday: 'short' }), f('zh-CN', { dateStyle: 'long' })]; })());
out('date resolved', (() => { const o = new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).resolvedOptions(); return [o.locale, o.timeZone, o.year, o.month, o.day]; })());
out('plural', [new Intl.PluralRules('en').select(1), new Intl.PluralRules('en').select(2), new Intl.PluralRules('en', { type: 'ordinal' }).select(2), new Intl.PluralRules('en', { type: 'ordinal' }).select(23), new Intl.PluralRules('ar').select(3), new Intl.PluralRules('ja').select(1)]);
out('relative', [new Intl.RelativeTimeFormat('en').format(-1, 'day'), new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(-1, 'day'), new Intl.RelativeTimeFormat('en', { style: 'short' }).format(3, 'months'), new Intl.RelativeTimeFormat('es').format(2, 'hour')]);
out('list', [new Intl.ListFormat('en').format(['a', 'b', 'c']), new Intl.ListFormat('en', { type: 'disjunction' }).format(['a', 'b']), new Intl.ListFormat('de').format(['x', 'y', 'z'])]);
out('intl misc', [Intl.getCanonicalLocales(['EN-us', 'zh-hans-cn']), Intl.NumberFormat.supportedLocalesOf('en-US'), Object.prototype.toString.call(Intl), typeof Intl.Collator('en').compare, String(new Intl.NumberFormat())]);
out('intl errors', () => new Intl.NumberFormat('en', { style: 'currency' }));
