const out=[];
const record=(fn)=>{try{out.push(fn());}catch(e){out.push([e.name,e.message]);}};
const list=(array)=>Array.from(array,x=>String(x));
for(const C of [BigInt64Array,BigUint64Array]){
 out.push(C.name,C.length,C.BYTES_PER_ELEMENT,C.prototype.BYTES_PER_ELEMENT,typeof C,typeof globalThis[C.name]);
 const a=new C([0n,1n,-1n,9223372036854775808n,18446744073709551615n,18446744073709551616n,-18446744073709551617n,9007199254740993n]);
 out.push(list(a),a.length,a.byteLength,a.byteOffset,Object.prototype.toString.call(a),ArrayBuffer.isView(a));
 const opposite=C===BigInt64Array?BigUint64Array:BigInt64Array;
 out.push(list(new opposite(a)),list(new opposite(a.buffer)),list(C.of(true,'42',-3n)),list(C.from([1n,2n],x=>x+1n)));
 record(()=>C(1));record(()=>new C([1]));record(()=>new C(new Uint8Array(0)));record(()=>new Uint8Array(new C(0)));record(()=>new C([null]));record(()=>new C(['bad']));record(()=>new C([Symbol('x')]));
 a[0]='17';a[1]=true;out.push(list(a));
 record(()=>{a[99]=1;});record(()=>{a[-1]={valueOf(){return 4n;}};return a[-1];});
 Object.defineProperty(a,'0',{value:{valueOf(){return 9007199254740993n;}}});out.push(String(a[0]),Object.getOwnPropertyDescriptor(a,'0').writable);
 record(()=>Object.defineProperty(a,'0',{value:3}));record(()=>Reflect.defineProperty(a,'0',{value:'invalid'}));out.push(Reflect.defineProperty(a,'99',{value:'invalid'}));
 const b=new C([3n,-1n,2n,1n]);out.push(b.join('|'),list(b.sort()),list(b.toSorted((x,y)=>x<y?1:x>y?-1:0)),list(b.toReversed()),list(b.with(-1,9n)),list(b.map(x=>x+1n)),list(b.filter(x=>x>1n)));
 out.push(b.reduce((x,y)=>x+y,0n).toString(),b.includes(2n),b.includes(2),b.indexOf(2n),String(b.at(-1)),list(b.values()),Array.from(b.keys()));
 b.fill('7',1,3);out.push(list(b));b.copyWithin(1,0,2);out.push(list(b));b.reverse();out.push(list(b));
 const sub=b.subarray(1,3);sub[0]=5n;out.push(sub.buffer===b.buffer,sub.byteOffset,list(b),list(b.slice(1,3)));
 b.set(b.subarray(0,3),1);out.push(list(b));record(()=>b.set(new Uint8Array(0)));record(()=>b.set(new Uint8Array(9)));record(()=>b.set([1]));record(()=>b.fill(1,0,0));record(()=>b.with(99,1));record(()=>b.with(99,1n));
 const species=new C([1n,2n]);species.constructor={ [Symbol.species]:opposite };out.push(list(species.slice()),list(species.map(x=>x+1n)),list(species.filter(()=>true)));
 species.constructor={ [Symbol.species]:Uint8Array };record(()=>species.slice());record(()=>species.subarray());record(()=>species.map(x=>x));out.push(list(species.map(()=>3)),list(species.slice(0,0)));record(()=>species.filter(()=>true));
 const events=[];
 const like={length:2,get 0(){events.push('get0');return {valueOf(){events.push('convert0');return 1n;}};},get 1(){events.push('get1');return {valueOf(){events.push('convert1');return 2n;}};}};
 out.push(list(new C(like)),events.slice());events.length=0;new C(2).set(like);out.push(events.slice());events.length=0;out.push(list(C.from(like)),events.slice());
}
const bytes=new ArrayBuffer(24);const dv=new DataView(bytes);
for(const little of [true,false]){
 dv.setBigInt64(1,-9007199254740993n,little);out.push(dv.getBigInt64(1,little).toString(),dv.getBigUint64(1,little).toString(),Array.from(new Uint8Array(bytes).slice(1,9)));
 dv.setBigUint64(3,18446744073709551617n,little);out.push(dv.getBigUint64(3,little).toString());
}
record(()=>dv.setBigInt64(0,1));record(()=>dv.setBigUint64(99,1));record(()=>dv.setBigUint64(99,1n));record(()=>dv.getBigInt64(99));record(()=>DataView.prototype.getBigInt64.call({}));
dv.setBigInt64(0,'-2',true);out.push(dv.getBigInt64(0,true).toString(),list(new BigInt64Array(bytes,0,1)));
console.log(JSON.stringify(out));
