// fflate's asynchronous API over its synchronous one: callbacks on later
// turns, errors, termination and streams.
const fflate = require('fflate');

const order = [];
const bytes = (data) => Array.from(data).join(',');
const data = fflate.strToU8('hi');

const stop = fflate.gzip(data, (error, out) => {
  order.push(`gzip ${error} ${bytes(out)}`);
  fflate.gunzip(out, {}, (error, back) => {
    order.push(`gunzip ${error} ${fflate.strFromU8(back)}`);
    fflate.gunzip(data, (error, result) => {
      order.push(`bad ${error.message} ${error.code} ${result}`);
      streams();
    });
  });
});
order.push(`returned ${typeof stop}`);
fflate.gzip(data, () => order.push('terminated callback ran'))();
try {
  fflate.gzip(data);
} catch (error) {
  order.push(`no callback: ${error.message} ${error.code}`);
}
Promise.resolve().then(() => order.push('microtask'));

function streams() {
  const stream = new fflate.AsyncGzip((error, chunk, final) => {
    order.push(`chunk ${error} ${chunk && bytes(chunk)} ${final}`);
  });
  stream.ondrain = (size) => order.push(`drain ${size} ${stream.queuedSize}`);
  stream.push(fflate.strToU8('ab'));
  order.push(`queued ${stream.queuedSize}`);
  stream.push(fflate.strToU8('c'), true);
  stream.push(fflate.strToU8('d'));
  try {
    new fflate.AsyncGunzip().push(data);
  } catch (error) {
    order.push(`no handler: ${error.message} ${error.code}`);
  }
}

process.on('exit', () => console.log(order.join('\n')));
