'use strict';

// nativeboat: Node's `inspector` for the native runtime, which has no V8
// inspector: no debugger is attached, so `url()` is undefined as in a Node
// process started without --inspect, the inspector console's methods do
// nothing, and opening the inspector or connecting a session throws Node's
// ERR_INSPECTOR_NOT_AVAILABLE.

const EventEmitter = require('events');

function notAvailable() {
  const error = new Error('Inspector is not available');
  error.code = 'ERR_INSPECTOR_NOT_AVAILABLE';
  return error;
}

function notActive() {
  const error = new Error('Inspector is not active');
  error.code = 'ERR_INSPECTOR_NOT_ACTIVE';
  return error;
}

class Session extends EventEmitter {
  connect() {
    throw notAvailable();
  }

  connectToMainThread() {
    throw notAvailable();
  }

  post(method, params, callback) {
    void method;
    void params;
    void callback;
    const error = new Error('Session is not connected');
    error.code = 'ERR_INSPECTOR_NOT_CONNECTED';
    throw error;
  }

  disconnect() {}
}

const consoleMethods = [
  'debug', 'error', 'info', 'log', 'warn', 'dir', 'dirxml', 'table', 'trace', 'group',
  'groupCollapsed', 'groupEnd', 'clear', 'count', 'countReset', 'assert', 'profile',
  'profileEnd', 'time', 'timeLog', 'timeEnd', 'timeStamp', 'context',
];
const inspectorConsole = {};
for (const name of consoleMethods) inspectorConsole[name] = function () {};

module.exports = {
  open() {
    throw notAvailable();
  },
  close() {},
  url() {
    return undefined;
  },
  waitForDebugger() {
    throw notActive();
  },
  console: inspectorConsole,
  Session,
  Network: {
    requestWillBeSent() {},
    responseReceived() {},
    loadingFinished() {},
    loadingFailed() {},
  },
};
