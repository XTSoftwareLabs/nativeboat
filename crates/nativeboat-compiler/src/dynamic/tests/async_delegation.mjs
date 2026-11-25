const out = [];
const log = (...values) => out.push(values.map(v => String(v)).join(':'));
const kind = e => e instanceof Error ? e.constructor.name : String(e);
const show = r => `${r.value}/${r.done}`;
async function* wrap(source) { return yield* source; }
export let result = '';
export const completed = (async () => {
  const array = wrap([Promise.resolve(1), 2]);
  log('array', show(await array.next()), show(await array.next('sent')), show(await array.next()));
  for await (const value of wrap('a🚀')) log('string', value);
  async function* inner() { yield 3; yield await Promise.resolve(4); return Promise.resolve(5); }
  const nested = wrap(inner());
  log('nested', show(await nested.next()), show(await nested.next()), show(await nested.next()));
  for (const sync of [false, true]) {
    let calls = 0;
    const iterator = {
      get next() { log('get-next', sync); return function(...args) {
        log('next', sync, this === iterator, args.length, args[0]);
        this.next = () => { throw 'next replaced'; };
        return { get done() {log('get-done', sync); return ++calls > 2;},
          get value() {log('get-value', sync); return Promise.resolve(calls);} };
      }; },
      throw(value) {log('throw-method', sync, value); return {done:false,value:Promise.resolve('recovered')};},
      return(value) {log('return-method', sync, value); return {done:false,value:Promise.resolve('cleanup')};},
    };
    // A setter allows replacement while preserving the initially captured next.
    Object.defineProperty(iterator, 'next', {configurable:true,
      get: Object.getOwnPropertyDescriptor(iterator,'next').get,
      set(value) {Object.defineProperty(this,'next',{value,writable:true,configurable:true});}});
    const source = {[sync ? Symbol.iterator : Symbol.asyncIterator]() {log('iterator', sync);return iterator;},
      [sync ? Symbol.asyncIterator : Symbol.iterator]: undefined};
    const g = wrap(source);
    log('first', sync, show(await g.next('ignored')));
    log('throw-step', sync, show(await g.throw('why')));
    log('return-step', sync, show(await g.return(Promise.resolve('r'))));
    log('after-return-step', sync, show(await g.next('next-input')));
    log('end-step', sync, show(await g.next('last-input')));
  }
  for (const sync of [false, true]) {
    for (const action of ['throw','return']) {
      const iterator = {next() {return {done:false,value:1};}};
      if(action==='throw') iterator.return = function(...args) {log('close',sync,args.length);return {done:true,value:Promise.resolve(8)};};
      const g = wrap({[sync ? Symbol.iterator : Symbol.asyncIterator]() {return iterator;}});
      await g.next();
      try {log('missing',sync,action,show(await g[action](Promise.resolve('sent'))));}
      catch(e) {log('missing-error',sync,action,e instanceof Promise ? await e : kind(e));}
    }
    const iterator = {next() {return {done:false,value:1};},
      return(value) {log('return-final',sync,value);return {done:true,value:Promise.resolve(9)};},
      throw(value) {log('throw-final',sync,value);return {done:true,value:Promise.resolve(10)};}};
    for (const action of ['return','throw']) {
      const g = wrap({[sync ? Symbol.iterator : Symbol.asyncIterator]() {return iterator;}});
      await g.next();
      const r = await g[action]('sent');
      log('final',sync,action,r.done,r.value instanceof Promise,await r.value);
    }
    for (const method of ['next','return','throw']) {
      for (const failure of ['getter','call','primitive','done','value','reject']) {
        let count=0;
        const iterator = {next() {return {done:false,value:1};}};
        const bad = () => {
          if(failure==='call') throw 'call';
          if(failure==='primitive') return 7;
          if(failure==='reject') return sync ? {done:true,value:Promise.reject('reject')} : Promise.reject('reject');
          return {get done() {if(failure==='done')throw 'done';return true;},
            get value() {if(failure==='value')throw 'value';return 12;}};
        };
        Object.defineProperty(iterator,method,{get() {
          if(method==='next')return function() {if(count++===0)return {done:false,value:1};return bad();};
          if(failure==='getter')throw 'getter';return bad;
        }});
        const g=wrap({[sync ? Symbol.iterator : Symbol.asyncIterator]() {return iterator;}});
        await g.next();
        try {log('failure-result',sync,method,failure,show(await g[method]('x')));}
        catch(e) {log('failure-error',sync,method,failure,kind(e));}
      }
    }
  }
  for (const finish of [undefined, () => 3, () => Promise.reject('close reject')]) {
    const g=wrap({[Symbol.asyncIterator]() {return {next(){return {value:1};},return:finish};}});
    await g.next();
    try {await g.throw('missing');}catch(e){log('async-close-error',kind(e));}
  }
  const rejectedYield = Promise.reject('yield reject');
  rejectedYield.catch(()=>{});
  const recovering = { [Symbol.asyncIterator]() {return {
    next(){return {done:false,value:rejectedYield};},
    throw(reason){log('yield-reject-forwarded',reason);return {done:true,value:13};}
  };}};
  log('recovered',show(await wrap(recovering).next()));
  const rejectReturn = wrap(recovering);
  log('recovered-next',show(await rejectReturn.next()));
  const accepting = wrap({[Symbol.asyncIterator]() {return {
    next() {return {done:false,value:1};},
    throw(reason) {log('return-rejection-forwarded',reason);return {done:true,value:15};}
  };}});
  await accepting.next();
  log('return-rejection-recovered',show(await accepting.return(Promise.reject('return rejected'))));
  const getter = {[Symbol.asyncIterator]() {return {get next() {throw 'next getter';}};}};
  try {await wrap(getter).next();}catch(e){log('initial-next-getter',kind(e));}
  async function* pendingReturn() {try {yield 1;return Promise.resolve(2);}finally{log('return-finally');}}
  const pending=pendingReturn();
  log('pending-first',show(await pending.next()));
  log('pending-final',show(await pending.next()));
  const queued=wrap([1,2]);
  const queue=[queued.next(),queued.next('a'),queued.return(Promise.resolve(14)),queued.next(),queued.throw('late').catch(kind)];
  log('queue',(await Promise.all(queue)).map(r=>typeof r==='string'?r:show(r)).join(','));
  for(const source of [null, 2, {[Symbol.asyncIterator]:1}, {[Symbol.asyncIterator]() {return 3;}}]) {
    try {await wrap(source).next();}catch(e){log('not-iterable',kind(e));}
  }
  result=out.join('\n');
})();
