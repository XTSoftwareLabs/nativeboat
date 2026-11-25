'use strict';

// nativeboat: the primordials the bundled Node.js library modules use, made
// the way Node.js v22.15.0 lib/internal/per_context/primordials.js makes
// them: static members copied (Promise statics bound to Promise), prototype
// methods and getters uncurried.
// They are captured when this module is first required, where Node captures
// them at startup. The Safe collections are the intrinsic Map, Set,
// WeakMap, WeakSet, WeakRef and FinalizationRegistry. Native errors have no
// stacks, so ErrorCaptureStackTrace does nothing. Array buffers are never
// detached, so ArrayBufferPrototypeTransfer copies where the runtime has no
// ArrayBuffer.prototype.transfer.

const { bind, call } = Function.prototype;
const uncurryThis = bind.bind(call);
const getter = (object, key) => uncurryThis(Object.getOwnPropertyDescriptor(object, key).get);
const IteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const TypedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const ArrayIteratorPrototype = Object.getPrototypeOf([][Symbol.iterator]());
const StringIteratorPrototype = Object.getPrototypeOf(''[Symbol.iterator]());

function createSafeIterator(factory, next) {
  class SafeIterator {
    constructor(iterable) {
      this._iterator = factory(iterable);
    }

    next() {
      return next(this._iterator);
    }

    [Symbol.iterator]() {
      return this;
    }
  }

  Object.setPrototypeOf(SafeIterator.prototype, null);
  Object.freeze(SafeIterator.prototype);
  return Object.freeze(SafeIterator);
}

const SafeArrayIterator = createSafeIterator(
  uncurryThis(Array.prototype[Symbol.iterator]),
  uncurryThis(ArrayIteratorPrototype.next),
);
const SafeStringIterator = createSafeIterator(
  uncurryThis(String.prototype[Symbol.iterator]),
  uncurryThis(StringIteratorPrototype.next),
);

const AsyncIteratorPrototype =
  Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype);

function transferArrayBuffer(buffer, length = buffer.byteLength) {
  const copy = new ArrayBuffer(length);
  new Uint8Array(copy).set(new Uint8Array(buffer, 0, Math.min(length, buffer.byteLength)));
  return copy;
}

module.exports = {
  Array,
  ArrayFrom: Array.from,
  uncurryThis,
  ArrayBuffer,
  ArrayBufferIsView: ArrayBuffer.isView,
  ArrayBufferPrototypeGetByteLength: getter(ArrayBuffer.prototype, 'byteLength'),
  ArrayBufferPrototypeGetDetached: getter(ArrayBuffer.prototype, 'detached'),
  ArrayBufferPrototypeSlice: uncurryThis(ArrayBuffer.prototype.slice),
  ArrayBufferPrototypeTransfer: ArrayBuffer.prototype.transfer
    ? uncurryThis(ArrayBuffer.prototype.transfer) : transferArrayBuffer,
  AsyncIteratorPrototype,
  BigIntPrototypeValueOf: uncurryThis(BigInt.prototype.valueOf),
  BooleanPrototypeValueOf: uncurryThis(Boolean.prototype.valueOf),
  DataView,
  ArrayIsArray: Array.isArray,
  ArrayPrototypeForEach: uncurryThis(Array.prototype.forEach),
  ArrayPrototypeFilter: uncurryThis(Array.prototype.filter),
  ArrayPrototypeIndexOf: uncurryThis(Array.prototype.indexOf),
  ArrayPrototypeMap: uncurryThis(Array.prototype.map),
  ArrayPrototypeReduce: uncurryThis(Array.prototype.reduce),
  ArrayPrototypeIncludes: uncurryThis(Array.prototype.includes),
  ArrayPrototypeJoin: uncurryThis(Array.prototype.join),
  ArrayPrototypePop: uncurryThis(Array.prototype.pop),
  ArrayPrototypePush: uncurryThis(Array.prototype.push),
  ArrayPrototypeReverse: uncurryThis(Array.prototype.reverse),
  ArrayPrototypeShift: uncurryThis(Array.prototype.shift),
  ArrayPrototypeSlice: uncurryThis(Array.prototype.slice),
  ArrayPrototypeSplice: uncurryThis(Array.prototype.splice),
  ArrayPrototypeUnshift: uncurryThis(Array.prototype.unshift),
  ArrayPrototypeToSorted: uncurryThis(Array.prototype.toSorted),
  ArrayPrototypeSymbolIterator: uncurryThis(Array.prototype[Symbol.iterator]),
  ArrayIteratorPrototypeNext: uncurryThis(ArrayIteratorPrototype.next),
  BigInt,
  BigIntPrototypeToString: uncurryThis(BigInt.prototype.toString),
  Boolean,
  Date,
  DateNow: Date.now,
  DatePrototypeGetTime: uncurryThis(Date.prototype.getTime),
  Error,
  ErrorCaptureStackTrace: function captureStackTrace() {},
  ErrorPrototype: Error.prototype,
  Float32Array,
  Float64Array,
  FunctionPrototypeBind: uncurryThis(Function.prototype.bind),
  FunctionPrototypeCall: uncurryThis(Function.prototype.call),
  FunctionPrototypeSymbolHasInstance: uncurryThis(Function.prototype[Symbol.hasInstance]),
  Int8Array,
  MathAbs: Math.abs,
  IteratorPrototype,
  MathCeil: Math.ceil,
  MathFloor: Math.floor,
  MathMax: Math.max,
  MathMaxApply: (values) => Math.max.apply(Math, values),
  MathMin: Math.min,
  MathPow: Math.pow,
  MathRound: Math.round,
  MathSign: Math.sign,
  MathTrunc: Math.trunc,
  Number,
  NumberIsInteger: Number.isInteger,
  NumberIsFinite: Number.isFinite,
  NumberIsNaN: Number.isNaN,
  NumberMAX_SAFE_INTEGER: Number.MAX_SAFE_INTEGER,
  NumberMIN_SAFE_INTEGER: Number.MIN_SAFE_INTEGER,
  NumberParseInt: Number.parseInt,
  NumberPrototypeToString: uncurryThis(Number.prototype.toString),
  NumberPrototypeValueOf: uncurryThis(Number.prototype.valueOf),
  ObjectAssign: Object.assign,
  ObjectDefineProperties: Object.defineProperties,
  ObjectDefineProperty: Object.defineProperty,
  ObjectEntries: Object.entries,
  ObjectFreeze: Object.freeze,
  ObjectIs: Object.is,
  ObjectGetOwnPropertyDescriptor: Object.getOwnPropertyDescriptor,
  ObjectGetOwnPropertyDescriptors: Object.getOwnPropertyDescriptors,
  ObjectGetOwnPropertySymbols: Object.getOwnPropertySymbols,
  ObjectGetPrototypeOf: Object.getPrototypeOf,
  ObjectKeys: Object.keys,
  ObjectPrototypeHasOwnProperty: uncurryThis(Object.prototype.hasOwnProperty),
  ObjectPrototypePropertyIsEnumerable: uncurryThis(Object.prototype.propertyIsEnumerable),
  ObjectPrototypeToString: uncurryThis(Object.prototype.toString),
  ObjectSetPrototypeOf: Object.setPrototypeOf,
  ObjectValues: Object.values,
  Promise,
  PromiseReject: Promise.reject.bind(Promise),
  PromiseResolve: Promise.resolve.bind(Promise),
  PromisePrototypeThen: uncurryThis(Promise.prototype.then),
  PromiseWithResolvers: Promise.withResolvers.bind(Promise),
  SafePromiseAll: (promises, mapFn) => Promise.all(mapFn ? Array.from(promises, mapFn) : promises),
  SafePromisePrototypeFinally: uncurryThis(Promise.prototype.finally),
  ReflectApply: Reflect.apply,
  ReflectConstruct: Reflect.construct,
  ReflectGet: Reflect.get,
  ReflectGetOwnPropertyDescriptor: Reflect.getOwnPropertyDescriptor,
  ReflectOwnKeys: Reflect.ownKeys,
  RegExp,
  RegExpPrototypeExec: uncurryThis(RegExp.prototype.exec),
  RegExpPrototypeGetDotAll: getter(RegExp.prototype, 'dotAll'),
  RegExpPrototypeGetGlobal: getter(RegExp.prototype, 'global'),
  RegExpPrototypeGetHasIndices: getter(RegExp.prototype, 'hasIndices'),
  RegExpPrototypeGetIgnoreCase: getter(RegExp.prototype, 'ignoreCase'),
  RegExpPrototypeGetMultiline: getter(RegExp.prototype, 'multiline'),
  RegExpPrototypeGetSource: getter(RegExp.prototype, 'source'),
  RegExpPrototypeGetSticky: getter(RegExp.prototype, 'sticky'),
  RegExpPrototypeGetUnicode: getter(RegExp.prototype, 'unicode'),
  RegExpPrototypeSymbolReplace: uncurryThis(RegExp.prototype[Symbol.replace]),
  RegExpPrototypeTest: uncurryThis(RegExp.prototype.test),
  SafeMap: Map,
  SafeSet: Set,
  SafeArrayIterator,
  SafeStringIterator,
  SafeFinalizationRegistry: FinalizationRegistry,
  SafeWeakMap: WeakMap,
  SafeWeakRef: WeakRef,
  SafeWeakSet: WeakSet,
  String,
  StringFromCharCode: String.fromCharCode,
  StringPrototypeCharAt: uncurryThis(String.prototype.charAt),
  StringPrototypeCodePointAt: uncurryThis(String.prototype.codePointAt),
  StringPrototypeStartsWith: uncurryThis(String.prototype.startsWith),
  StringPrototypeCharCodeAt: uncurryThis(String.prototype.charCodeAt),
  StringPrototypeEndsWith: uncurryThis(String.prototype.endsWith),
  StringPrototypeIncludes: uncurryThis(String.prototype.includes),
  StringPrototypeIndexOf: uncurryThis(String.prototype.indexOf),
  StringPrototypeLastIndexOf: uncurryThis(String.prototype.lastIndexOf),
  StringPrototypeRepeat: uncurryThis(String.prototype.repeat),
  StringPrototypeReplace: uncurryThis(String.prototype.replace),
  StringPrototypeSlice: uncurryThis(String.prototype.slice),
  StringPrototypeSplit: uncurryThis(String.prototype.split),
  StringPrototypeToWellFormed: uncurryThis(String.prototype.toWellFormed),
  StringPrototypeToLowerCase: uncurryThis(String.prototype.toLowerCase),
  StringPrototypeToUpperCase: uncurryThis(String.prototype.toUpperCase),
  StringPrototypeTrim: uncurryThis(String.prototype.trim),
  StringPrototypeValueOf: uncurryThis(String.prototype.valueOf),
  Symbol,
  SymbolHasInstance: Symbol.hasInstance,
  StringIteratorPrototypeNext: uncurryThis(StringIteratorPrototype.next),
  SymbolAsyncIterator: Symbol.asyncIterator,
  SymbolDispose: Symbol.dispose,
  SymbolFor: Symbol.for,
  SymbolIterator: Symbol.iterator,
  SymbolPrototypeValueOf: uncurryThis(Symbol.prototype.valueOf),
  SymbolSpecies: Symbol.species,
  SymbolReplace: Symbol.replace,
  SymbolSplit: Symbol.split,
  SymbolToPrimitive: Symbol.toPrimitive,
  SymbolToStringTag: Symbol.toStringTag,
  TypeError,
  TypedArrayPrototypeAt: uncurryThis(TypedArrayPrototype.at),
  TypedArrayPrototypeIncludes: uncurryThis(TypedArrayPrototype.includes),
  TypedArrayPrototypeFill: uncurryThis(TypedArrayPrototype.fill),
  TypedArrayPrototypeGetBuffer: getter(TypedArrayPrototype, 'buffer'),
  TypedArrayPrototypeGetByteLength: getter(TypedArrayPrototype, 'byteLength'),
  TypedArrayPrototypeGetByteOffset: getter(TypedArrayPrototype, 'byteOffset'),
  TypedArrayPrototypeGetLength: getter(TypedArrayPrototype, 'length'),
  TypedArrayPrototypeGetSymbolToStringTag: getter(TypedArrayPrototype, Symbol.toStringTag),
  TypedArrayPrototypeSet: uncurryThis(TypedArrayPrototype.set),
  TypedArrayPrototypeSlice: uncurryThis(TypedArrayPrototype.slice),
  TypedArrayPrototypeSubarray: uncurryThis(TypedArrayPrototype.subarray),
  Uint8Array,
  Uint8ArrayPrototype: Uint8Array.prototype,
  decodeURIComponent,
};
