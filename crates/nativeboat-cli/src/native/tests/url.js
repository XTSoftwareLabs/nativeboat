import * as url from 'node:url';
const out = [];
const record = (fn) => { try { out.push(fn()); } catch (error) { out.push([error.name,error.code,error.message,error.input,error.base]); } };
const u = new URL('https://user:pw@Exämple.com:443/a/../b?x=1#f');
out.push(URL===url.URL,URL===globalThis.URL,URLSearchParams===url.URLSearchParams,URLSearchParams===globalThis['URLSearchParams']);
for(const key of ['href','origin','protocol','username','password','host','hostname','port','pathname','search','hash'])out.push([key,u[key]]);
out.push(u.toString(),u.toJSON(),Object.prototype.toString.call(u),u instanceof url.URL);
const params = u.searchParams;
out.push(params===u.searchParams,params instanceof URLSearchParams,params.constructor===url.URLSearchParams);
params.append('q','a b');out.push(u.href,params.toString());
params.delete('x');out.push(u.search,u.href);
u.search='?z=2+3';out.push(params===u.searchParams,params.get('z'),params.toString());
u.href='http://other:90/new?v=4#h';out.push(params===u.searchParams,params.toString(),u.href);
for(const [name,value]of [['protocol','https:'],['username','a@b'],['password','a:b'],['host','[::1]:99'],['hostname','example.org'],['port','443'],['pathname','/a b'],['hash','#中 a']]) {u[name]=value;out.push(name,u.href);}
out.push(URL.canParse('/x','https://host/a'),URL.canParse('bad'),URL.parse('bad'),URL.parse('../b','https://host/a').href);
record(()=>new URL());record(()=>new URL('bad'));record(()=>new URL('https://good','bad'));
record(()=>URL.prototype.toString.call({}));record(()=>URL.canParse());record(()=>URL.parse());
for(const domain of ['Exämple.com','faß.de','xn--fa-hia.de','0x7f000001','bad host'])out.push(domain,url.domainToASCII(domain),url.domainToUnicode(domain));
for(const [path,options]of [['C:\\a b\\#x?y%z',{windows:true}],['\\\\server\\share\\a b',{windows:true}],['/a b/#x?y%z',{windows:false}]]){
 const file=url.pathToFileURL(path,options);out.push(file.href,url.fileURLToPath(file,options));
}
for(const input of ['file:///C:/a%20b','file://server/share/a%20b'])out.push(url.fileURLToPath(input,{windows:true}));
record(()=>url.fileURLToPath('https://host/a'));record(()=>url.fileURLToPath('file:///a%2Fb',{windows:false}));record(()=>url.fileURLToPath('file:///a%5Cb',{windows:true}));record(()=>url.fileURLToPath('file://host/a',{windows:false}));
const legacy=url.parse('HTTP://user:pw@Exämple.com:80/a b?x=1#f',true);
out.push(legacy.protocol,legacy.hostname,legacy.pathname,legacy.query.x,legacy.href,url.format(legacy));
out.push(url.resolve('https://host/a/b','../c'),url.resolveObject('https://host/a/b','?x').href);
out.push(url.format(new URL('https://u:p@Exämple.com/a?q#f'),{unicode:true,auth:false,search:false,fragment:false}));
const options=url.urlToHttpOptions(new URL('https://u:p@Exämple.com:99/a?q#h'));
out.push(options.protocol,options.hostname,options.hash,options.search,options.pathname,options.path,options.href,options.port,options.auth);
record(()=>url.parse('http://[bad]'));record(()=>url.parse(5));record(()=>url.pathToFileURL(5));
console.log(JSON.stringify(out));
