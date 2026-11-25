'use strict';

// nativeboat: Node's `node:sqlite` for the native runtime, which bundles no
// SQLite engine: the module loads, so feature probes find it, and opening a
// database reports that SQLite is not available.

function unavailable() {
  const error = new Error('node:sqlite is not available in a native build');
  error.code = 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM';
  return error;
}

class DatabaseSync {
  constructor() {
    throw unavailable();
  }
}

class StatementSync {
  constructor() {
    const error = new TypeError('Illegal constructor');
    error.code = 'ERR_ILLEGAL_CONSTRUCTOR';
    throw error;
  }
}

module.exports = {
  DatabaseSync,
  StatementSync,
  constants: {
    SQLITE_CHANGESET_OMIT: 0,
    SQLITE_CHANGESET_REPLACE: 1,
    SQLITE_CHANGESET_ABORT: 2,
  },
};
