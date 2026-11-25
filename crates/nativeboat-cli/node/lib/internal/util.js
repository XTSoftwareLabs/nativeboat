'use strict';

// nativeboat: the subset of Node.js v22.15.0 lib/internal/util.js that the
// bundled library modules use. The definitions are copied unchanged, except
// that lazyDOMException takes DOMException from the bundled
// internal/per_context/domexception instead of the messaging binding, and
// does not adjust stack traces, which native errors do not have.
// getInternalGlobal supplies a protected native RegExp constructor instead
// of evaluating a string in Node's private VM context.

const {
  ArrayPrototypePush,
  FunctionPrototypeCall,
  ObjectDefineProperties,
  ObjectDefineProperty,
  ObjectFreeze,
  ObjectGetOwnPropertyDescriptor,
  ObjectGetOwnPropertyDescriptors,
  ObjectGetPrototypeOf,
  ObjectSetPrototypeOf,
  ObjectValues,
  Promise,
  ReflectApply,
  ReflectConstruct,
  RegExpPrototypeGetDotAll,
  RegExpPrototypeGetGlobal,
  RegExpPrototypeGetHasIndices,
  RegExpPrototypeGetIgnoreCase,
  RegExpPrototypeGetMultiline,
  RegExpPrototypeGetSource,
  RegExpPrototypeGetSticky,
  RegExpPrototypeGetUnicode,
  SafeSet,
  SafeWeakMap,
  StringPrototypeReplace,
  StringPrototypeToLowerCase,
  Symbol,
  SymbolDispose,
  SymbolFor,
  SymbolReplace,
  SymbolSplit,
} = primordials;

const {
  defineLazyProperties,
} = internalBinding('util');
const { encodings } = internalBinding('string_decoder');
const { isPromise } = internalBinding('types');

const isWindows = process.platform === 'win32';
const isMacOS = process.platform === 'darwin';

const experimentalWarnings = new SafeSet();

const customInspectSymbol = SymbolFor('nodejs.util.inspect.custom');

/**
 * Move the "slow cases" to a separate function to make sure this function gets
 * inlined properly. That prioritizes the common case.
 * @param {unknown} enc
 * @returns {string | undefined} Returns undefined if there is no match.
 */
function normalizeEncoding(enc) {
  if (enc == null || enc === 'utf8' || enc === 'utf-8') return 'utf8';
  return slowCases(enc);
}

function slowCases(enc) {
  switch (enc.length) {
    case 4:
      if (enc === 'UTF8') return 'utf8';
      if (enc === 'ucs2' || enc === 'UCS2') return 'utf16le';
      enc = StringPrototypeToLowerCase(enc);
      if (enc === 'utf8') return 'utf8';
      if (enc === 'ucs2') return 'utf16le';
      break;
    case 3:
      if (enc === 'hex' || enc === 'HEX' ||
      StringPrototypeToLowerCase(enc) === 'hex')
        return 'hex';
      break;
    case 5:
      if (enc === 'ascii') return 'ascii';
      if (enc === 'ucs-2') return 'utf16le';
      if (enc === 'UTF-8') return 'utf8';
      if (enc === 'ASCII') return 'ascii';
      if (enc === 'UCS-2') return 'utf16le';
      enc = StringPrototypeToLowerCase(enc);
      if (enc === 'utf-8') return 'utf8';
      if (enc === 'ascii') return 'ascii';
      if (enc === 'ucs-2') return 'utf16le';
      break;
    case 6:
      if (enc === 'base64') return 'base64';
      if (enc === 'latin1' || enc === 'binary') return 'latin1';
      if (enc === 'BASE64') return 'base64';
      if (enc === 'LATIN1' || enc === 'BINARY') return 'latin1';
      enc = StringPrototypeToLowerCase(enc);
      if (enc === 'base64') return 'base64';
      if (enc === 'latin1' || enc === 'binary') return 'latin1';
      break;
    case 7:
      if (enc === 'utf16le' || enc === 'UTF16LE' ||
      StringPrototypeToLowerCase(enc) === 'utf16le')
        return 'utf16le';
      break;
    case 8:
      if (enc === 'utf-16le' || enc === 'UTF-16LE' ||
      StringPrototypeToLowerCase(enc) === 'utf-16le')
        return 'utf16le';
      break;
    case 9:
      if (enc === 'base64url' || enc === 'BASE64URL' ||
      StringPrototypeToLowerCase(enc) === 'base64url')
        return 'base64url';
      break;
    default:
      if (enc === '') return 'utf8';
  }
}

function emitExperimentalWarning(feature, messagePrefix, code, ctor) {
  if (experimentalWarnings.has(feature)) return;
  experimentalWarnings.add(feature);
  let msg = `${feature} is an experimental feature and might change at any time`;
  if (messagePrefix) {
    msg = messagePrefix + msg;
  }
  process.emitWarning(msg, 'ExperimentalWarning', code, ctor);
}

function getLazy(initializer) {
  let value;
  let initialized = false;
  return function() {
    if (initialized === false) {
      value = initializer();
      initialized = true;
    }
    return value;
  };
}

let _DOMException;
const lazyDOMExceptionClass = () => {
  _DOMException ??= require('internal/per_context/domexception').DOMException;
  return _DOMException;
};

const lazyDOMException = (message, name) => {
  _DOMException ??= require('internal/per_context/domexception').DOMException;
  return new _DOMException(message, name);
};

const kIsEncodingSymbol = Symbol('kIsEncodingSymbol');

function spliceOne(list, index) {
  for (; index + 1 < list.length; index++)
    list[index] = list[index + 1];
  list.pop();
}

function getConstructorOf(obj) {
  while (obj) {
    const descriptor = ObjectGetOwnPropertyDescriptor(obj, 'constructor');
    if (descriptor !== undefined &&
        typeof descriptor.value === 'function' &&
        descriptor.value.name !== '') {
      return descriptor.value;
    }

    obj = ObjectGetPrototypeOf(obj);
  }

  return null;
}

const kCustomPromisifiedSymbol = SymbolFor('nodejs.util.promisify.custom');
const kCustomPromisifyArgsSymbol = Symbol('customPromisifyArgs');

let validateFunction;

function promisify(original) {
  // Lazy-load to avoid a circular dependency.
  if (validateFunction === undefined)
    ({ validateFunction } = require('internal/validators'));

  validateFunction(original, 'original');

  if (original[kCustomPromisifiedSymbol]) {
    const fn = original[kCustomPromisifiedSymbol];

    validateFunction(fn, 'util.promisify.custom');

    return ObjectDefineProperty(fn, kCustomPromisifiedSymbol, {
      __proto__: null,
      value: fn, enumerable: false, writable: false, configurable: true,
    });
  }

  // Names to create an object from in case the callback receives multiple
  // arguments, e.g. ['bytesRead', 'buffer'] for fs.read.
  const argumentNames = original[kCustomPromisifyArgsSymbol];

  function fn(...args) {
    return new Promise((resolve, reject) => {
      ArrayPrototypePush(args, (err, ...values) => {
        if (err) {
          return reject(err);
        }
        if (argumentNames !== undefined && values.length > 1) {
          const obj = {};
          for (let i = 0; i < argumentNames.length; i++)
            obj[argumentNames[i]] = values[i];
          resolve(obj);
        } else {
          resolve(values[0]);
        }
      });
      if (isPromise(ReflectApply(original, this, args))) {
        process.emitWarning('Calling promisify on a function that returns a Promise is likely a mistake.',
                            'DeprecationWarning', 'DEP0174');
      }
    });
  }

  ObjectSetPrototypeOf(fn, ObjectGetPrototypeOf(original));

  ObjectDefineProperty(fn, kCustomPromisifiedSymbol, {
    __proto__: null,
    value: fn, enumerable: false, writable: false, configurable: true,
  });

  const descriptors = ObjectGetOwnPropertyDescriptors(original);
  const propertiesValues = ObjectValues(descriptors);
  for (let i = 0; i < propertiesValues.length; i++) {
    // We want to use null-prototype objects to not rely on globally mutable
    // %Object.prototype%.
    ObjectSetPrototypeOf(propertiesValues[i], null);
  }
  return ObjectDefineProperties(fn, descriptors);
}

promisify.custom = kCustomPromisifiedSymbol;

const kEnumerableProperty = { __proto__: null };
kEnumerableProperty.enumerable = true;
ObjectFreeze(kEnumerableProperty);

const colorRegExp = /\u001b\[\d\d?m/g; // eslint-disable-line no-control-regex

function removeColors(str) {
  return StringPrototypeReplace(str, colorRegExp, '');
}

let internalGlobal;
function getInternalGlobal() {
  if (internalGlobal == null) {
    // nativeboat: protected native intrinsics replace Node's VM bootstrap.
    internalGlobal = { RegExp: internalBinding('util').getProtectedRegExp() };
  }
  return internalGlobal;
}

function SideEffectFreeRegExpPrototypeExec(regex, string) {
  const { RegExp: RegExpFromAnotherRealm } = getInternalGlobal();
  return FunctionPrototypeCall(RegExpFromAnotherRealm.prototype.exec, regex, string);
}

const crossRealmRegexes = new SafeWeakMap();
function getCrossRealmRegex(regex) {
  const cached = crossRealmRegexes.get(regex);
  if (cached) return cached;

  let flagString = '';
  if (RegExpPrototypeGetHasIndices(regex)) flagString += 'd';
  if (RegExpPrototypeGetGlobal(regex)) flagString += 'g';
  if (RegExpPrototypeGetIgnoreCase(regex)) flagString += 'i';
  if (RegExpPrototypeGetMultiline(regex)) flagString += 'm';
  if (RegExpPrototypeGetDotAll(regex)) flagString += 's';
  if (RegExpPrototypeGetUnicode(regex)) flagString += 'u';
  if (RegExpPrototypeGetSticky(regex)) flagString += 'y';

  const { RegExp: RegExpFromAnotherRealm } = getInternalGlobal();
  const crossRealmRegex = new RegExpFromAnotherRealm(RegExpPrototypeGetSource(regex), flagString);
  crossRealmRegexes.set(regex, crossRealmRegex);
  return crossRealmRegex;
}

function SideEffectFreeRegExpPrototypeSymbolReplace(regex, string, replacement) {
  return getCrossRealmRegex(regex)[SymbolReplace](string, replacement);
}

function SideEffectFreeRegExpPrototypeSymbolSplit(regex, string, limit = undefined) {
  return getCrossRealmRegex(regex)[SymbolSplit](string, limit);
}

const kEmptyObject = ObjectFreeze({ __proto__: null });

const encodingsMap = { __proto__: null };
for (let i = 0; i < encodings.length; ++i)
  encodingsMap[encodings[i]] = i;

const codesWarned = new SafeSet();

let validateString;

function getDeprecationWarningEmitter(
  code, msg, deprecated, useEmitSync,
  shouldEmitWarning = () => true,
) {
  let warned = false;
  return function() {
    if (!warned && shouldEmitWarning()) {
      warned = true;
      if (code === 'ExperimentalWarning') {
        process.emitWarning(msg, code, deprecated);
      } else if (code !== undefined) {
        if (!codesWarned.has(code)) {
          const emitWarning = useEmitSync ?
            require('internal/process/warning').emitWarningSync :
            process.emitWarning;
          emitWarning(msg, 'DeprecationWarning', code, deprecated);
          codesWarned.add(code);
        }
      } else {
        process.emitWarning(msg, 'DeprecationWarning', deprecated);
      }
    }
  };
}


function deprecate(fn, msg, code, useEmitSync) {
  // Lazy-load to avoid a circular dependency.
  if (validateString === undefined)
    ({ validateString } = require('internal/validators'));

  if (code !== undefined)
    validateString(code, 'code');

  const emitDeprecationWarning = getDeprecationWarningEmitter(
    code, msg, deprecated, useEmitSync,
  );

  function deprecated(...args) {
    // TODO(joyeecheung): use getOptionValue('--no-deprecation') instead.
    if (!process.noDeprecation) {
      emitDeprecationWarning();
    }
    if (new.target) {
      return ReflectConstruct(fn, args, new.target);
    }
    return ReflectApply(fn, this, args);
  }

  // The wrapper will keep the same prototype as fn to maintain prototype chain
  ObjectSetPrototypeOf(deprecated, fn);
  if (fn.prototype) {
    // Setting this (rather than using Object.setPrototype, as above) ensures
    // that calling the unwrapped constructor gives an instanceof the wrapped
    // constructor.
    deprecated.prototype = fn.prototype;
  }

  return deprecated;
}


function once(callback, { preserveReturnValue = false } = kEmptyObject) {
  let called = false;
  let returnValue;
  return function(...args) {
    if (called) return returnValue;
    called = true;
    const result = ReflectApply(callback, this, args);
    returnValue = preserveReturnValue ? result : undefined;
    return result;
  };
}


function isError(e) {
  // An error from another realm is not an instance of this realm's Error.
  return require('internal/util/types').isNativeError(e) || e instanceof Error;
}

module.exports = {
  customPromisifyArgs: kCustomPromisifyArgsSymbol,
  isError,
  customInspectSymbol,
  defineLazyProperties,
  deprecate,
  emitExperimentalWarning,
  encodingsMap,
  getLazy,
  getConstructorOf,
  isMacOS,
  isWindows,
  kEmptyObject,
  kEnumerableProperty,
  kIsEncodingSymbol,
  lazyDOMException,
  lazyDOMExceptionClass,
  normalizeEncoding,
  once,
  promisify,
  removeColors,
  SideEffectFreeRegExpPrototypeExec,
  SideEffectFreeRegExpPrototypeSymbolReplace,
  SideEffectFreeRegExpPrototypeSymbolSplit,
  spliceOne,
  SymbolDispose,
};
