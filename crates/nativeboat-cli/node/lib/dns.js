'use strict';

// nativeboat: Node's `dns` for the native runtime. `lookup` resolves names
// through the system resolver (`GetAddrInfoW`), as Node's does through
// getaddrinfo, in the resolver's order ('verbatim'), or IPv4 first or IPv6
// first when asked. Queries that go to DNS servers (`resolve*`, `reverse`,
// `lookupService`) fail with ENOTIMP: the runtime has no DNS client.

const { isIP } = require('internal/net');

const tcp = internalBinding('tcp');

const ADDRCONFIG = 1024;
const ALL = 256;
const V4MAPPED = 2048;

const codes = {
  NODATA: 'ENODATA',
  FORMERR: 'EFORMERR',
  SERVFAIL: 'ESERVFAIL',
  NOTFOUND: 'ENOTFOUND',
  NOTIMP: 'ENOTIMP',
  REFUSED: 'EREFUSED',
  BADQUERY: 'EBADQUERY',
  BADNAME: 'EBADNAME',
  BADFAMILY: 'EBADFAMILY',
  BADRESP: 'EBADRESP',
  CONNREFUSED: 'ECONNREFUSED',
  TIMEOUT: 'ETIMEOUT',
  EOF: 'EOF',
  FILE: 'EFILE',
  NOMEM: 'ENOMEM',
  DESTRUCTION: 'EDESTRUCTION',
  BADSTR: 'EBADSTR',
  BADFLAGS: 'EBADFLAGS',
  NONAME: 'ENONAME',
  BADHINTS: 'EBADHINTS',
  NOTINITIALIZED: 'ENOTINITIALIZED',
  LOADIPHLPAPI: 'ELOADIPHLPAPI',
  ADDRGETNETWORKPARAMS: 'EADDRGETNETWORKPARAMS',
  CANCELLED: 'ECANCELLED',
};

let defaultOrder = 'verbatim';

function invalidArgument(name, expected, value) {
  const error = new TypeError(`The "${name}" argument must be ${expected}. Received ${typeof value}`);
  error.code = 'ERR_INVALID_ARG_TYPE';
  return error;
}

function options(value) {
  if (typeof value === 'number') return { family: value, all: false, order: defaultOrder };
  if (value === undefined || value === null) return { family: 0, all: false, order: defaultOrder };
  if (typeof value !== 'object') throw invalidArgument('options', 'of type object', value);
  let family = value.family ?? 0;
  if (family === 'IPv4') family = 4;
  if (family === 'IPv6') family = 6;
  let order = value.order ?? defaultOrder;
  if (value.verbatim === false) order = 'ipv4first';
  return { family, all: value.all === true, order };
}

// [[address, family], ...] filtered by family and ordered as asked.
function arrange(list, { family, order }) {
  let entries = family === 4 || family === 6 ? list.filter((entry) => entry[1] === family) : list;
  if (order === 'ipv4first') entries = [...entries].sort((a, b) => a[1] - b[1]);
  if (order === 'ipv6first') entries = [...entries].sort((a, b) => b[1] - a[1]);
  return entries;
}

function notFound(hostname) {
  const error = new Error(`getaddrinfo ENOTFOUND ${hostname}`);
  error.errno = 'ENOTFOUND';
  error.code = 'ENOTFOUND';
  error.syscall = 'getaddrinfo';
  error.hostname = hostname;
  return error;
}

function lookup(hostname, value, callback) {
  if (typeof value === 'function') {
    callback = value;
    value = undefined;
  }
  if (typeof callback !== 'function') throw invalidArgument('callback', 'of type function', callback);
  if (hostname !== undefined && hostname !== null && typeof hostname !== 'string') {
    throw invalidArgument('hostname', 'of type string', hostname);
  }
  const settings = options(value);
  const answer = (error, entries) => {
    if (error) return callback(error);
    if (settings.all) return callback(null, entries.map(([address, family]) => ({ address, family })));
    if (!entries.length) return callback(notFound(hostname));
    callback(null, entries[0][0], entries[0][1]);
  };
  if (!hostname) {
    const family = settings.family === 6 ? 6 : 4;
    process.nextTick(answer, null, [[family === 6 ? '::1' : '127.0.0.1', family]]);
    return {};
  }
  const literal = isIP(hostname);
  if (literal) {
    process.nextTick(answer, null, [[hostname, literal]]);
    return {};
  }
  tcp.lookup(hostname, (error, list) => {
    if (error) return answer(error);
    const entries = arrange(list, settings);
    answer(entries.length ? null : notFound(hostname), entries);
  });
  return {};
}

function notImplemented(syscall, name) {
  const error = new Error(`${syscall} ENOTIMP ${name ?? ''}`.trim());
  error.errno = 'ENOTIMP';
  error.code = 'ENOTIMP';
  error.syscall = syscall;
  if (name !== undefined) error.hostname = name;
  return error;
}

function unsupported(syscall) {
  return function (name, ...rest) {
    const callback = rest[rest.length - 1];
    if (typeof callback !== 'function') throw invalidArgument('callback', 'of type function', callback);
    process.nextTick(callback, notImplemented(syscall, name));
    return {};
  };
}

function lookupService(address, port, callback) {
  if (typeof callback !== 'function') throw invalidArgument('callback', 'of type function', callback);
  process.nextTick(callback, notImplemented('getnameinfo', address));
}

const queries = {
  resolve: unsupported('queryA'),
  resolve4: unsupported('queryA'),
  resolve6: unsupported('queryAaaa'),
  resolveAny: unsupported('queryAny'),
  resolveCaa: unsupported('queryCaa'),
  resolveCname: unsupported('queryCname'),
  resolveMx: unsupported('queryMx'),
  resolveNaptr: unsupported('queryNaptr'),
  resolveNs: unsupported('queryNs'),
  resolvePtr: unsupported('queryPtr'),
  resolveSoa: unsupported('querySoa'),
  resolveSrv: unsupported('querySrv'),
  resolveTxt: unsupported('queryTxt'),
  reverse: unsupported('getHostByAddr'),
};

let servers = [];

class Resolver {
  constructor() {
    this._servers = [];
  }

  cancel() {}

  getServers() { return [...this._servers]; }

  setServers(list) { this._servers = [...list]; }

  setLocalAddress() {}
}
Object.assign(Resolver.prototype, queries);

function promisify(method) {
  return (name, ...rest) => new Promise((resolve, reject) => {
    method(name, ...rest, (error, value) => (error ? reject(error) : resolve(value)));
  });
}

let promises;

module.exports = {
  lookup,
  lookupService,
  Resolver,
  getDefaultResultOrder() { return defaultOrder; },
  setDefaultResultOrder(order) { defaultOrder = order; },
  setServers(list) { servers = [...list]; },
  getServers() { return [...servers]; },
  ADDRCONFIG,
  ALL,
  V4MAPPED,
  ...codes,
  ...queries,
  get promises() {
    if (promises === undefined) {
      const promiseQueries = {};
      for (const [name, method] of Object.entries(queries)) promiseQueries[name] = promisify(method);
      class PromiseResolver extends Resolver {}
      Object.assign(PromiseResolver.prototype, promiseQueries);
      promises = {
        lookup(hostname, value) {
          return new Promise((resolve, reject) => {
            lookup(hostname, value, (error, address, family) => {
              if (error) return reject(error);
              resolve(Array.isArray(address) ? address : { address, family });
            });
          });
        },
        lookupService(address, port) {
          return new Promise((resolve, reject) => {
            lookupService(address, port, (error, hostname, service) => (error ? reject(error) : resolve({ hostname, service })));
          });
        },
        Resolver: PromiseResolver,
        getDefaultResultOrder: module.exports.getDefaultResultOrder,
        setDefaultResultOrder: module.exports.setDefaultResultOrder,
        getServers: module.exports.getServers,
        setServers: module.exports.setServers,
        ...codes,
        ...promiseQueries,
      };
    }
    return promises;
  },
};
