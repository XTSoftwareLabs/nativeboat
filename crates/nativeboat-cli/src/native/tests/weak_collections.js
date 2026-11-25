'use strict';
const out = [];
const t = (f) => { try { return String(f()); } catch (e) { return `${e.constructor.name}: ${e.message}`; } };
const a = {}, b = [], s = Symbol('s');
const map = new WeakMap([[a, 1]]);
out.push(map.get(a), map.has(b), map.set(b, 2) === map, map.get(b), map.set(s, 3).get(s), map.delete(a), map.has(a), map.delete(a), map.get(1), map.has('x'), map.delete(null));
const set = new WeakSet([a]);
out.push(set.has(a), set.add(b) === set, set.has(b), set.delete(b), set.has(b), set.add(s).has(s), set.delete(1));
out.push(t(() => map.set(1, 2)), t(() => map.set(Symbol.for('r'), 1)), t(() => set.add('x')), t(() => new WeakMap([1])), t(() => new WeakMap([[1, 2]])));
out.push(t(() => WeakMap()), t(() => WeakSet()), t(() => WeakRef(a)), t(() => FinalizationRegistry(() => {})));
for (const [f, r] of [[WeakMap.prototype.get, {}], [WeakMap.prototype.set, new Map()], [WeakSet.prototype.add, []], [WeakSet.prototype.has, 1], [WeakRef.prototype.deref, map], [FinalizationRegistry.prototype.register, 'x']]) {
  out.push(t(() => f.call(r, a, 1)));
}
class Sub extends WeakMap {}
Sub.prototype.set = 5;
out.push(t(() => new Sub([])), t(() => new Sub().has(a)));
class Mine extends WeakSet { add(v) { out.push('added'); return super.add(v); } }
out.push(new Mine([a, b]).has(b), new Mine() instanceof WeakSet);
const ref = new WeakRef(a);
out.push(ref.deref() === a, new WeakRef(s).deref() === s, t(() => new WeakRef(1)), t(() => new WeakRef(Symbol.for('q'))));
const temporary = new WeakRef({ kept: true });
out.push(temporary.deref().kept);
const fr = new FinalizationRegistry((held) => out.push(`cleanup ${held}`));
const token = {};
out.push(fr.register(b, 'b', token), fr.register(s, 's'), t(() => fr.register(1, 2)), t(() => fr.register(a, a)), t(() => fr.register(a, 1, 2)), t(() => fr.unregister(2)), t(() => fr.unregister(Symbol.for('k'))));
out.push(fr.unregister(token), fr.unregister(token), fr.unregister({}), t(() => new FinalizationRegistry(1)));
for (const value of [map, set, ref, fr, WeakMap.prototype, WeakRef.prototype]) out.push(Object.prototype.toString.call(value));
out.push(Object.getOwnPropertyNames(WeakMap.prototype).join(), Object.getOwnPropertyNames(WeakSet.prototype).join(), Object.getOwnPropertyNames(WeakRef.prototype).join(), Object.getOwnPropertyNames(FinalizationRegistry.prototype).join());
out.push(WeakMap.length, WeakSet.length, WeakRef.length, FinalizationRegistry.length, WeakMap.prototype.set.length, FinalizationRegistry.prototype.unregister.length, WeakMap.name);
console.log(map, set, ref, fr, new Sub());
Promise.resolve().then(() => {
  out.push(temporary.deref().kept === true);
  console.log(out.join('\n'));
});
