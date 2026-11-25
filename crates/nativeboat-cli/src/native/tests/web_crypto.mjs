const s = crypto.subtle, enc = (t) => new TextEncoder().encode(t), hex = (b) => Buffer.from(b).toString('hex');
const out = [];
out.push(hex(await s.digest('SHA-256', enc('abc'))), hex(await s.digest({ name: 'sha-1' }, enc('abc'))));
const hk = await s.importKey('raw', enc('key'), { name: 'HMAC', hash: 'SHA-256' }, true, ['sign', 'verify']);
const mac = await s.sign('HMAC', hk, enc('data'));
out.push(hex(mac), await s.verify('HMAC', hk, mac, enc('data')), await s.verify('HMAC', hk, mac, enc('datb')));
out.push(JSON.stringify(await s.exportKey('jwk', hk)), JSON.stringify(hk.algorithm), hk.type, hk.extractable, JSON.stringify(hk.usages));
const ak = await s.importKey('raw', new Uint8Array(32).fill(9), 'AES-GCM', true, ['encrypt', 'decrypt']);
const sealed = await s.encrypt({ name: 'AES-GCM', iv: new Uint8Array(12).fill(4), additionalData: enc('header') }, ak, enc('secret message'));
out.push(hex(sealed), Buffer.from(await s.decrypt({ name: 'AES-GCM', iv: new Uint8Array(12).fill(4), additionalData: enc('header') }, ak, sealed)).toString());
out.push(JSON.stringify(await s.exportKey('jwk', ak)), JSON.stringify(ak.algorithm));
try { await s.decrypt({ name: 'AES-GCM', iv: new Uint8Array(12) }, ak, sealed); } catch (e) { out.push(e.name, e.message); }
const ck = await s.importKey('raw', new Uint8Array(16).fill(1), 'AES-CBC', false, ['encrypt']);
out.push(hex(await s.encrypt({ name: 'AES-CBC', iv: new Uint8Array(16).fill(2) }, ck, enc('0123456789abcdef'))));
try { await s.decrypt({ name: 'AES-CBC', iv: new Uint8Array(16).fill(2) }, ck, new Uint8Array(16)); } catch (e) { out.push(e.name); }
try { await s.exportKey('raw', ck); } catch (e) { out.push(e.message); }
const tk = await s.importKey('raw', new Uint8Array(16).fill(1), 'AES-CTR', false, ['encrypt']);
const counter = new Uint8Array(16).fill(0xab); counter[15] = 0xfe;
out.push(hex(await s.encrypt({ name: 'AES-CTR', counter, length: 4 }, tk, new Uint8Array(40))));
const pk = await s.importKey('raw', enc('pw'), 'PBKDF2', false, ['deriveBits', 'deriveKey']);
out.push(hex(await s.deriveBits({ name: 'PBKDF2', salt: enc('salt'), iterations: 1000, hash: 'SHA-256' }, pk, 160)));
const dk = await s.deriveKey({ name: 'PBKDF2', salt: enc('salt'), iterations: 1, hash: 'SHA-1' }, pk, { name: 'AES-GCM', length: 128 }, true, ['encrypt']);
out.push(hex(await s.exportKey('raw', dk)), JSON.stringify(dk.algorithm));
const xk = await s.importKey('raw', enc('key'), 'HKDF', false, ['deriveBits']);
out.push(hex(await s.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt: enc('salt'), info: enc('info') }, xk, 336)));
const gk = await s.generateKey({ name: 'HMAC', hash: 'SHA-512' }, true, ['sign']);
out.push(gk.algorithm.name, gk.algorithm.length, gk.algorithm.hash.name, (await s.exportKey('raw', gk)).byteLength);
const jk = await s.importKey('jwk', { kty: 'oct', k: 'YWJj', alg: 'HS256' }, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
out.push(hex(await s.sign('HMAC', jk, enc('x'))));
try { await s.sign('HMAC', ak, enc('x')); } catch (e) { out.push(e.name, e.message); }
try { await s.digest('MD5', enc('x')); } catch (e) { out.push(e.name, e.message); }
try { await s.importKey('raw', enc('k'), 'AES-GCM', false, ['encrypt']); } catch (e) { out.push(e.name, e.message); }
try { await s.importKey('raw', new Uint8Array(16), 'AES-GCM', false, ['sign']); } catch (e) { out.push(e.name, e.message); }
try { crypto.getRandomValues(new Float64Array(2)); } catch (e) { out.push(e.name, e.message); }
try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { out.push(e.name, e.message); }
out.push(typeof CryptoKey, hk instanceof CryptoKey);
console.log(out.join('\n'));
