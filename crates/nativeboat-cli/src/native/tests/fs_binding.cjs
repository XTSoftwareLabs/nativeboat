'use strict';
const b = require('internal/binding')('fs');
const { getStatsFromBinding } = require('internal/fs/utils');
const { Buffer } = require('buffer');
const root = __ROOT__;
const file = root + '/native-中🚀.txt';
const out=[];
const log=(...values)=>out.push(values.map(String).join(':'));
for(const name of ['statValues','bigintStatValues','statFsValues','bigintStatFsValues']) log('shape',name,b[name].constructor.name,b[name].length);
b.writeFileUtf8(file,'nativeboat',0x301,0o666);
log('utf8',b.readFileUtf8(file,0),b.existsSync(file),b.existsSync(root+'/missing'));
const stat=b.stat(file,false,undefined);
log('stat',stat===b.statValues,stat[8],getStatsFromBinding(stat).isFile());
stat[18]=12345;
log('lstat',b.lstat(file,false,undefined)===stat,stat[18]);
const big=b.stat(file,true,undefined);
log('bigint',big===b.bigintStatValues,String(big[8]));
log('nonboolean',b.stat(file,1,undefined)===stat);
const before=Array.from(stat);
log('missing',b.stat(root+'/missing',false,undefined,false)===undefined,before.every((v,i)=>v===stat[i]));
try{b.stat(root+'/missing',false,undefined);}catch(e){log('error',e.code,e.errno,e.syscall,Object.getPrototypeOf(e)===Error.prototype,Object.keys(e).join(','),e.path.endsWith('missing'));}
const fd=b.open(file,2,0o666);
log('fd-stat',b.fstat(fd,false,undefined)===stat,stat[8]);
const buffer=Buffer.alloc(8,99);
log('read',b.read(fd,buffer,2,3,1),buffer.toString('hex'));
const ctx={};
log('write',b.writeString(fd,'XY',2,'utf8',undefined,ctx));
log('changed',b.readFileUtf8(file,0));
const vectors=[Buffer.alloc(2),Buffer.alloc(3)];
log('readv',b.readBuffers(fd,vectors,0),vectors.map(v=>v.toString()).join('|'));
b.ftruncate(fd,6);
b.fsync(fd);
b.close(fd);
try{b.fstat(fd,false,undefined);}catch(e){log('closed',e.code,e.syscall,e.errno);}
const writeCtx={};
log('write-error',b.writeBuffer(fd,buffer,0,1,-1,undefined,writeCtx),writeCtx.code,writeCtx.syscall);
const dir=root+'/nested/a/b';
log('mkdir',b.mkdir(dir,0o777,true).endsWith('nested'),b.mkdir(dir,0o777,true));
log('readdir',b.readdir(root,'utf8',false).join('|'));
const names=b.readdir(root,'buffer',true);
log('dir-types',names[0].map(v=>v.toString()).join('|'),names[1].join('|'));
log('statfs',b.statfs(root,false)===b.statFsValues,b.statfs(root,true)===b.bigintStatFsValues);
const copy=root+'/copy';
b.copyFile(file,copy,0);
log('copy',b.readFileUtf8(copy,0));
b.rename(copy,root+'/renamed');
b.unlink(root+'/renamed');
log('removed',b.existsSync(root+'/renamed'));
b.link(file,root+'/hard');
log('hardlink',b.readFileUtf8(root+'/hard',0));
b.unlink(root+'/hard');
for(const path of [file,file+'/child']){
 try{b.mkdir(path,0o777,true);}catch(e){log('mkdir-file',e.code,e.syscall);}
}
b.chmod(file,0o666);
b.utimes(file,1700000000,1700000001);
const timed=b.open(file,2,0o666);
b.fchmod(timed,0o666);
b.futimes(timed,1700000002,1700000003);
log('times',b.fstat(timed,false,undefined)[12]);
b.fdatasync(timed);
b.close(timed);
const temp=b.mkdtemp(root+'/temp-XXXXXX','utf8');
log('temporary',b.existsSync(temp),temp.startsWith(root));
b.rmdir(temp);
b.mkdir(root+'/target',0o777,false);
b.symlink(root+'/target',root+'/junction',2);
log('junction',b.readlink(root+'/junction','utf8').endsWith('target'),b.realpath(root+'/junction','utf8').endsWith('target'));
b.unlink(root+'/junction');
b.rmdir(root+'/target');
b.mkdir(root+'/rawnames',0o777,false);
const rawpath=Buffer.concat([Buffer.from(root+'/rawnames/x'),Buffer.from([0xed,0xa0,0x80])]);
b.writeFileUtf8(rawpath,'raw',0x301,0o666);
log('raw-name',b.readdir(root+'/rawnames','buffer',false)[0].toString('hex'),b.readdir(root+'/rawnames','utf8',false)[0],b.readFileUtf8(rawpath,0));
const invalidpath=Buffer.concat([Buffer.from(root+'/rawnames/bad'),Buffer.from([255])]);
try{b.stat(invalidpath,false,undefined);}catch(e){log('invalid-name',e.code,e.syscall,e.path.endsWith('bad�'));}
try{b.symlink(rawpath,root+'/absent/link',2);}catch(e){log('raw-link-error',e.syscall,e.path.endsWith('x���'),e.dest.endsWith('link'));}
b.unlink(rawpath);
b.rmdir(root+'/rawnames');
const request=()=>new Promise((resolve,reject)=>{
 const req=new b.FSReqCallback();
 req.oncomplete=function(error,result){if(error)reject(error);else{log('callback',this===req,result===stat,result[8]);resolve();}};
 b.stat(file,false,req);log('submitted');
});
request().then(async()=>{
 const promised=await b.stat(file,false,b.kUsePromises);
 log('promise',promised!==stat,promised.length,promised[8]);
 const big=await b.stat(file,true,b.kUsePromises);
 log('promise-big',big!==b.bigintStatValues,big.length,String(big[8]));
 try{await b.stat(root+'/missing',false,b.kUsePromises);}catch(e){log('promise-error',e.code,e.syscall);}
 const req=new b.FSReqCallback(true);
 await new Promise(resolve=>{req.oncomplete=function(e,v){log('callback-big',e===null,v===b.bigintStatValues,String(v[8]));resolve();};b.stat(file,false,req);});
 console.log(out.join('\n'));
});
