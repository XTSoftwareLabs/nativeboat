 'use strict';
process.noDeprecation=true;
const u=require('internal/fs/utils');
const internalBinding=require('internal/binding');
const constants=internalBinding('constants').fs;
const assert=require('internal/assert');
const {once,deprecate}=require('internal/util');
const {validateUint32}=require('internal/validators');
const out=[];const record=(fn)=>{try{out.push(fn());}catch(e){out.push([e.name,e.code,e.message]);}};
out.push(Object.getPrototypeOf(constants)===null,Object.entries(constants));
const fields=[1,0x81b6,2,3,4,5,4096,9007199254740993n,7,8,1234,567000000,1235,0,-1,500000000,0,0];
for(const C of [Float64Array,BigInt64Array]){
 const values=new C(fields.map(x=>C===BigInt64Array?BigInt(x):Number(x)));
 const s=u.getStatsFromBinding(values);
 out.push(Object.keys(s),s.constructor.name,[s.dev,s.mode,s.nlink,s.uid,s.gid,s.rdev,s.blksize,s.ino,s.size,s.blocks].map(String));
 out.push([s.atimeMs,s.mtimeMs,s.ctimeMs,s.birthtimeMs].map(String),[s.atimeNs,s.mtimeNs,s.ctimeNs,s.birthtimeNs].map(String));
 out.push(s.isFile(),s.isDirectory(),s.isSymbolicLink(),s.isBlockDevice(),s.isFIFO(),s.isSocket(),s.isCharacterDevice());
 out.push(s.atime.toISOString(),s.mtime.toISOString(),s.ctime.toISOString(),s.birthtime.toISOString(),s.atime===s.atime);
 out.push(Object.getOwnPropertyDescriptor(s,'atime').writable,Object.getOwnPropertyDescriptor(s,'atime').enumerable,Object.getOwnPropertyDescriptor(s,'atime').configurable);
 const doubled=new C([...values,...values]);out.push(String(u.getStatsFromBinding(doubled,18).ino));
 const fs=u.getStatFsFromBinding(new C([1n,4096n,20n,10n,9n,8n,7n].map(x=>C===BigInt64Array?x:Number(x))));out.push(Object.keys(fs),Object.values(fs).map(String));
}
for(let type=0;type<=7;type++){
 const d=new u.Dirent('ü.txt',type,'C:\\repo');out.push(d.name,d.parentPath,d.path,[d.isFile(),d.isDirectory(),d.isSymbolicLink(),d.isFIFO(),d.isSocket(),d.isCharacterDevice(),d.isBlockDevice()]);
}
for(const flags of ['r','rs','sr','r+','rs+','sr+','w','wx','xw','w+','wx+','xw+','a','ax','xa','as','sa','a+','ax+','xa+','as+','sa+',0,null,undefined,'invalid',2n])record(()=>u.stringToFlags(flags));
for(const type of [undefined,null,'file','dir','junction','invalid',3])record(()=>u.stringToSymlinkType(type));
for(const x of [0,4.5,'3.25',new Date(1234),false,{},Infinity])record(()=>u.toUnixTimestamp(x));
for(const opts of [undefined,null,()=>{},'utf8',{encoding:'utf8'},{encoding:'bad'},5])record(()=>u.getOptions(opts,{flag:'r'}));
for(const x of [0,1,4294967295,-1,4294967296,0.1,'1'])record(()=>validateUint32(x,'size'));
record(()=>assert(false,'stat assertion'));record(()=>assert.fail());
let n=0;const receiver={n:2};const cb=once(function(x){n++;return this.n+x;},{preserveReturnValue:true});out.push(cb.call(receiver,3),cb.call(receiver,9),n);
const onceUndefined=once(()=>5);out.push(onceUndefined(),onceUndefined());
function Original(x){this.x=x;this.target=new.target;}const Deprecated=deprecate(Original,'message','DEP1234');const obj=new Deprecated(3);out.push(obj.x,obj instanceof Original,obj instanceof Deprecated,obj.target===Deprecated);
function Derived(){}const other=Reflect.construct(Deprecated,[7],Derived);out.push(other.x,other instanceof Derived,other.target===Derived);
console.log(JSON.stringify(out));
