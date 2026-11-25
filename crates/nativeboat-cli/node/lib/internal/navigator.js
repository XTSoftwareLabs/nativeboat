'use strict';

// nativeboat: Node.js v22's `navigator` global for programs outside pages
// (pages have the DOM's): the processor count, the default locale, the
// platform and Node's user agent.

const { availableParallelism } = require('os');

// Node's platform names, as browsers spell them.
function navigatorPlatform() {
  switch (process.platform) {
    case 'darwin': return 'MacIntel';
    case 'win32': return 'Win32';
    case 'linux': return process.arch === 'ia32' ? 'Linux i686' : process.arch === 'x64' ? 'Linux x86_64' : `Linux ${process.arch}`;
    case 'freebsd': return process.arch === 'ia32' ? 'FreeBSD i386' : process.arch === 'x64' ? 'FreeBSD amd64' : `FreeBSD ${process.arch}`;
    case 'openbsd': return process.arch === 'ia32' ? 'OpenBSD i386' : process.arch === 'x64' ? 'OpenBSD amd64' : `OpenBSD ${process.arch}`;
    case 'sunos': return process.arch === 'ia32' ? 'SunOS i86pc' : `SunOS ${process.arch}`;
    case 'aix': return 'AIX';
    default: return `${process.platform[0].toUpperCase()}${process.platform.slice(1)} ${process.arch}`;
  }
}

const kInitialize = Symbol('kInitialize');

function defaultLocale() {
  try {
    return new Intl.Collator().resolvedOptions().locale || 'en-US';
  } catch {
    return 'en-US';
  }
}

class Navigator {
  constructor(key) {
    if (key !== kInitialize) {
      throw new TypeError('Illegal constructor');
    }
  }

  get hardwareConcurrency() {
    return availableParallelism();
  }

  get language() {
    return defaultLocale();
  }

  get languages() {
    return [this.language];
  }

  get platform() {
    return navigatorPlatform();
  }

  get userAgent() {
    return `Node.js/${process.version.slice(1).split('.')[0]}`;
  }
}

Object.defineProperty(Navigator.prototype, Symbol.toStringTag, {
  value: 'Navigator',
  configurable: true,
});

const navigator = new Navigator(kInitialize);

module.exports = {
  Navigator,
  navigator,
};
