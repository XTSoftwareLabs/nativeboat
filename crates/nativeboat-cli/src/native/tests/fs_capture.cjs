'use strict';
const b=require('internal/binding')('fs');
const root=__ROOT__;
const invalid=Buffer.concat([Buffer.from(root+'/invalid-'),Buffer.from([255])]);
const out=[];
process.on('unhandledRejection',e=>out.push(['unhandled',shape(e)]));
const shape=e=>[e.code,e.errno,e.syscall,e.path?.endsWith('invalid-�'),e.dest?.endsWith('invalid-�')];
const record=(name,fn)=>{try{out.push([name,'return',fn()]);}catch(e){out.push([name,'throw',shape(e)]);}};
record('exists',()=>b.existsSync(invalid));
for(const name of ['stat','lstat']){
 record(name,()=>b[name](invalid,false,undefined));
 record(name+'-suppressed',()=>b[name](invalid,false,undefined,false));
}
record('rename-dest-sync',()=>b.rename(root+'/old',invalid));
record('rename-source-sync',()=>b.rename(invalid,root+'/new'));
const requests=[];
for(const [name,args] of [['stat',[invalid,false]],['lstat',[invalid,false]],['open',[invalid,0,0o666]],['rename',[root+'/old',invalid]],['rename-source',[invalid,root+'/new']]]){
 const actual=name==='rename-source'?'rename':name;
 const req=new b.FSReqCallback();
 req.oncomplete=function(error,value){out.push([name,'callback',this===req,shape(error),value]);};
 record(name+'-callback',()=>b[actual](...args,req));
 out.push([name,'after-call']);
 const p=b[actual](...args,b.kUsePromises);
 out.push([name,'promise-return',p instanceof Promise]);
 if(p)requests.push(p.then(v=>out.push([name,'fulfilled',v]),e=>out.push([name,'rejected',shape(e)])));
}
setImmediate(()=>console.log(JSON.stringify(out)));
