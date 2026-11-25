'use strict';

// TLS in compiled applications: Node's defaults and socket classes. Secure
// connections and servers are not available yet: they fail as net's do.

const net = require('net');

class TLSSocket extends net.Socket {
  constructor(socket, options) {
    super(options);
    this.encrypted = true;
    this.authorized = false;
  }
}

class Server extends net.Server {}

function connect(...args) {
  return new TLSSocket().connect(...args);
}

function createServer(options, listener) {
  const server = new Server();
  const connection = typeof options === 'function' ? options : listener;
  if (connection) {
    server.on('secureConnection', connection);
  }
  return server;
}

module.exports = {
  CLIENT_RENEG_LIMIT: 3,
  CLIENT_RENEG_WINDOW: 600,
  DEFAULT_CIPHERS: 'TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA256:ECDHE-RSA-AES256-SHA384:DHE-RSA-AES256-SHA384:ECDHE-RSA-AES256-SHA256:DHE-RSA-AES256-SHA256:HIGH:!aNULL:!eNULL:!EXPORT:!DES:!RC4:!MD5:!PSK:!SRP:!CAMELLIA',
  DEFAULT_ECDH_CURVE: 'auto',
  DEFAULT_MIN_VERSION: 'TLSv1.2',
  DEFAULT_MAX_VERSION: 'TLSv1.3',
  TLSSocket,
  Server,
  connect,
  createServer,
};
