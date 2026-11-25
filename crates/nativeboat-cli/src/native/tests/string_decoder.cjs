'use strict';
const {StringDecoder}=require('string_decoder');
const vectors=[
 [],[65],[65,66,67],[0x61,0xc2,0xa2,0x62],
 [0xe4,0xb8,0xad],[0xf0,0x9f,0x9a,0x80],
 [0xc0,0xaf],[0xed,0xa0,0x80],[0xf4,0x90,0x80,0x80],
 [0x80],[0x80,0x80,0x80,0x80,0x80],
 [0xc2,65],[0xe2,0x82,65],[0xf0,0x9f,0x9a,65],
 [0xff,0xfe,0x41,0],[0x3d,0xd8,0x80,0xde],
 [0x3d,0xd8,0x80],[0xf8,0xbf,0xbf,0xbf],
 [0xc2],[0xe2,0x82],[0xf0,0x9f,0x9a],
 [0x61,0x62,0x63,0x64,0x65,0x66,0x67],
];
const out=[];
const codes=s=>Array.from({length:s.length},(_,i)=>s.charCodeAt(i));
for(const encoding of ['utf8','utf16le','base64','base64url','ascii','latin1','hex']){
 for(let v=0;v<vectors.length;v++){
  const bytes=Buffer.from(vectors[v]);
  for(let split=0;split<=bytes.length;split++){
   const d=new StringDecoder(encoding);
   const first=d.write(bytes.subarray(0,split));
   const before=[d.lastNeed,d.lastTotal,Array.from(d.lastChar)];
   const second=d.write(bytes.subarray(split));
   const middle=[d.lastNeed,d.lastTotal,Array.from(d.lastChar)];
   const end=d.end();
   out.push(JSON.stringify([encoding,v,split,codes(first),before,codes(second),middle,codes(end),d.lastNeed,d.lastTotal,Array.from(d.lastChar)]));
   if(d.write(Buffer.from('reuse'))!==Buffer.from('reuse').toString(encoding) && !['utf16le','base64','base64url'].includes(encoding)) throw Error('reuse');
  }
 }
}
console.log(out.join('\n'));
