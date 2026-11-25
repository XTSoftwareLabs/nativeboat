const out = [];
const log = (...values) => out.push(values.join(':'));
const error = (e) => e instanceof Error ? e.constructor.name : e;
export let result = '';
export const completed = (async () => {
  for await (const x of [1, Promise.resolve(2), 3]) log('array', x);
  for await (const x of 'a🛤') log('string', x);
  async function* values() {
    try { yield 4; yield await Promise.resolve(5); } finally { log('async-finally'); }
  }
  for await (const x of values()) { log('async', x); break; }
  function* sync() { try { yield 6; yield 7; } finally { log('sync-finally'); } }
  for await (const x of sync()) { log('sync', x); break; }
  const closures = [];
  for await (let x of [8, 9]) closures.push(() => x);
  log('closures', closures.map(f => f()).join(','));
  let assigned;
  for await (assigned of [10]) {}
  log('assigned', assigned);
  for await (const [x, { y }] of [[11, { y: 12 }]]) log('pattern', x, y);
  async function returning() {
    for await (const x of sync()) {
      try { return x; } finally { log('return-finally'); }
    }
  }
  log('returned', await returning());
  outer: for await (const x of [1, 2]) {
    for await (const y of sync()) { log('nested', x, y); continue outer; }
  }
  for await (const x of [13, 14]) { if (x === 13) continue; log('continue', x); }
  const methods = { async sum() { let n = 0; for await (const x of [1, 2]) n += x; return n; } };
  log('method', await methods.sum());
  async function* wrapping() { for await (const x of [3, 4]) yield x; }
  for await (const x of wrapping()) log('wrapped', x);
  for await (const x of [{ then(resolve) { log('thenable'); resolve(19); } }]) log('thenable-value', x);
  const finalValue = { [Symbol.iterator]() { return { next() { return { done: true, value: Promise.reject('final rejection') }; } }; } };
  try { for await (const x of finalValue) {} } catch (e) { log('final-value', e); }
  const head = { [Symbol.iterator]() { return { next() { return { value: null, done: false }; }, return() { log('head-close'); return {}; } }; } };
  try { for await (const { x } of head) {} } catch (e) { log('head-error', error(e)); }

  // The async protocol takes priority, and its values are not awaited.
  const preferred = {
    [Symbol.iterator]() { throw 'wrong protocol'; },
    [Symbol.asyncIterator]() {
      let i = 0;
      return {
        next() { return Promise.resolve({ done: i++ > 0, value: Promise.resolve(15) }); },
        return() { log('preferred-return'); return Promise.resolve({}); },
      };
    },
  };
  for await (const value of preferred) { log('preferred', value instanceof Promise); break; }

  for (const async of [false, true]) {
    for (const mode of ['next-throw', 'next-primitive', 'done-get', 'value-get', 'reject',
                        'return-throw', 'return-primitive', 'return-get', 'return-reject', 'missing', 'normal']) {
      let count = 0;
      const key = async ? Symbol.asyncIterator : Symbol.iterator;
      const iterator = {
        next() {
          log('next', async, mode, count);
          if (mode === 'next-throw') throw 'next';
          if (mode === 'next-primitive') return async ? Promise.resolve(1) : 1;
          if (mode === 'done-get') return { get done() { throw 'done'; } };
          if (mode === 'value-get') return { done: false, get value() { throw 'value'; } };
          if (mode === 'reject') return async ? Promise.reject('rejected') : { value: Promise.reject('rejected'), done: false };
          return { value: 16, done: count++ > 0 };
        },
        return() {
          log('close', async, mode, arguments.length);
          if (mode === 'return-throw') throw 'close';
          if (mode === 'return-primitive') return 1;
          if (mode === 'return-reject') return async ? Promise.reject('close rejected') : { value: Promise.reject('close rejected') };
          return { get done() { log('close-done', async); return true; },
                   get value() { log('close-value', async); return Promise.resolve(17); } };
        },
      };
      if (mode === 'return-get') Object.defineProperty(iterator, 'return', { get() { throw 'close getter'; } });
      if (mode === 'missing') delete iterator.return;
      const source = { [key]() { return iterator; } };
      try { for await (const x of source) { if (mode !== 'normal') break; log('item', async, x); } }
      catch (e) { log('caught', async, mode, error(e)); }
      if (mode.startsWith('return')) {
        count = 0;
        try { for await (const x of source) { throw 'original'; } }
        catch (e) { log('preserved', async, mode, error(e)); }
      }
    }
  }
  for (const source of [null, undefined, {}, { [Symbol.asyncIterator]: 1 },
                        { [Symbol.asyncIterator]() { return 1; } },
                        { [Symbol.asyncIterator]: null, [Symbol.iterator]() { return [18][Symbol.iterator](); } }]) {
    try { for await (const x of source) log('fallback', x); }
    catch (e) { log('protocol', error(e)); }
  }
  function* rejecting() { try { yield Promise.reject('yield rejection'); } finally { log('rejection-finally'); } }
  const rejected = rejecting();
  try { for await (const x of rejected) {} } catch (e) { log('yield-error', e); }
  log('still-open', rejected.next().done);

  // A generator return is a return completion, so cleanup failure wins.
  function* syncReturningGenerator() {
    const source = { [Symbol.iterator]() { return {
      next() { return { value: 20, done: false }; }, return() { throw 'sync cleanup'; },
    }; } };
    for (const x of source) yield x;
  }
  const syncReturned = syncReturningGenerator();
  syncReturned.next();
  try { syncReturned.return('end'); } catch (e) { log('sync-generator-return', e); }
  for (const mode of ['success', 'throw', 'reject', 'primitive']) {
    const source = { [Symbol.asyncIterator]() { return {
      next() { return { value: 20, done: false }; },
      return() {
        log('generator-close', mode);
        if (mode === 'throw') throw 'cleanup';
        if (mode === 'reject') return Promise.reject('cleanup rejection');
        if (mode === 'primitive') return 1;
        return {};
      },
    }; } };
    async function* returningGenerator() { for await (const x of source) yield x; }
    const returned = returningGenerator();
    await returned.next();
    try { log('generator-return', mode, (await returned.return('end')).value); }
    catch (e) { log('generator-return-error', mode, error(e)); }
    const thrown = returningGenerator();
    await thrown.next();
    try { await thrown.throw('original generator'); }
    catch (e) { log('generator-throw-error', mode, error(e)); }
  }

  for (const async of [false, true]) {
    const order = [];
    const tick = (i) => { if (i < 7) queueMicrotask(() => { order.push(`m${i}`); tick(i + 1); }); };
    tick(0);
    const source = async ? { async *[Symbol.asyncIterator]() { yield 1; } } : [1];
    for await (const x of source) { order.push('body'); break; }
    order.push('end');
    await Promise.resolve(); await Promise.resolve();
    log('order', async, order.join(','));
  }
  result = out.join('|');
})();
