'use strict';
const internalBinding = require('internal/binding');
const binding = internalBinding('url');
const out = [];
function captured(fn) { try { const result = fn(); return { result, components: Array.from(binding.urlComponents) }; } catch(error) { return { name:error.name, code:error.code, message:error.message, input:error.input, base:error.base }; } }
for (const [input,base] of [
 ['https://user:pass@Exämple.com:443/a/../b?x=1#f'], ['../c?x#y','https://example.org/a/b'],
 ['http://0x7f000001'], ['http://[2001:0db8:0:0:0:0:0:1]:80/a'], ['file:///C:/a/../b'],
 ['data:text/plain,你好#x'], ['custom://user:pw@host:123/path?q#h'], ['blob:https://example.com/id'],
 ['http://'], ['http://[bad]'], ['path'], ['https://good','bad base'], ['?query','file:///C:/a/b']
]) { out.push([input,base,captured(()=>binding.parse(input,base,true)),captured(()=>binding.parse(input,base,false)),binding.canParse(input,base)]); }
const href='https://user:password@example.org:8443/a/b?x=1#hash';
for (const [action,value] of [[0,'http:'],[0,'file:'],[0,'data:'],[1,'other:99'],[1,'other:bad'],[1,'[::1]:42'],[2,'other'],[2,'other:99'],[3,'80'],[3,'99junk'],[3,'65536'],[3,''],[4,'a@b'],[5,'a:b'],[6,'/a b/../c'],[7,'?q=中 a'],[7,''],[8,'#中 a'],[8,''],[9,'https://new/path'],[9,'bad']]) out.push([action,value,captured(()=>binding.update(href,action,value))]);
for(const href of ['file:///C:/a/b','data:text/plain,a','custom:opaque','custom://host/path'])for(const action of [0,1,2,3,4,5,6,7,8])out.push([href,action,captured(()=>binding.update(href,action,['http:','host:2','host','99','name','pw','/path','?x','#y'][action]))]);
for(const domain of ['', 'Exämple.com', 'faß.de', 'xn--fa-hia.de', '0x7f000001', '[::1]', 'a:b', 'bad host', 'example.com/path', '%65xample.com'])out.push([domain,binding.domainToASCII(domain),binding.domainToUnicode(domain)]);
for(const href of ['http://u:p@host:80/a','https://host/a','file:///C:/a','data:a','blob:https://example.com/id','bad'])out.push([href,captured(()=>binding.getOrigin(href))]);
for(const [path,windows,host]of[['/a b/#x?y%z',false],['C:\a b\#x?y%z',true],['/share/a b',true,'server'],['/你好',false]])out.push([path,windows,host,captured(()=>binding.pathToFileURL(path,windows,host))]);
for (const [href,action,value] of [
 ['https://host/a',0,'file:'],['https://host:44/a',0,'file:'],['file://host/a',0,'http:'],['file:///a',0,'file:'],['http://host/a',0,'1bad:'],['http://host/a',0,'h\tttp:'],
 ['https://host/a',1,'[::1]junk'],['https://host/a',2,'[::1]junk'],['https://host/a',1,'[::1]:'],['https://host:42/a',1,'other:'],['https://host/a',3,'junk'],['https://host/a',3,' 99'],
 ['custom://host:42/a',1,''],['custom://host/a',1,''],['https://host/a',1,''],['file:///a',1,'localhost']
]) {const result=binding.update(href,action,value);out.push([href,action,value,result,Array.from(binding.urlComponents)]);}

for (let mask=0;mask<16;mask++) {
 const flags=[Boolean(mask&1),Boolean(mask&2),Boolean(mask&4),Boolean(mask&8)];
 out.push(['format',flags,binding.format('https://user:pw@xn--exmple-cua.com:42/a?q#f',...flags)]);
}
for(const [path,windows,hostname] of [['C:\\a b\\#x?y%z',true],['C:\\a\\[]^|~%\\你好',true,'server'],['/a\\b',false],['/a\0\t\n\r',false]])out.push(['path',path,windows,hostname,captured(()=>binding.pathToFileURL(path,windows,hostname))]);

for(const [input,base] of [['file://server/C:/a'],['/C:/b','file://server/share/a'],['file:/C:/b','file://server/share/a'],['file:///C:/b','file://server/share/a'],['C:/b','file://server/share/a'],['../C:/b','file://server/share/a']])out.push(['file-parse',input,base,captured(()=>binding.parse(input,base,true))]);
out.push(['file-host-clear',captured(()=>binding.update('file://server/C:/a',2,''))]);

console.log(JSON.stringify(out));
