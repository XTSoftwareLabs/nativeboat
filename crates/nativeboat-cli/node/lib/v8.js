'use strict';

// nativeboat: Node's `v8` for the native runtime, which has no V8: heap and
// code statistics have Node's shape with zero values, flags are accepted
// without effect, and heap snapshots, serialization and the startup
// snapshot are not available.

function zeroes(names) {
  const statistics = {};
  for (const name of names) statistics[name] = 0;
  return statistics;
}

function getHeapStatistics() {
  return zeroes([
    'total_heap_size', 'total_heap_size_executable', 'total_physical_size',
    'total_available_size', 'used_heap_size', 'heap_size_limit', 'malloced_memory',
    'peak_malloced_memory', 'does_zap_garbage', 'number_of_native_contexts',
    'number_of_detached_contexts', 'total_global_handles_size', 'used_global_handles_size',
    'external_memory',
  ]);
}

function getHeapCodeStatistics() {
  return zeroes([
    'code_and_metadata_size', 'bytecode_and_metadata_size', 'external_script_source_size',
    'cpu_profiler_metadata_size',
  ]);
}

function getHeapSpaceStatistics() {
  return [];
}

function setFlagsFromString(flags) {
  if (typeof flags !== 'string') {
    const error = new TypeError('The "flags" argument must be of type string');
    error.code = 'ERR_INVALID_ARG_TYPE';
    throw error;
  }
}

function cachedDataVersionTag() {
  return 0;
}

function unavailable(name) {
  return function () {
    const error = new Error(`v8.${name}() is not available in a native build`);
    error.code = 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM';
    throw error;
  };
}

class Serializer {
  constructor() { unavailable('Serializer')(); }
}
class Deserializer {
  constructor() { unavailable('Deserializer')(); }
}

module.exports = {
  cachedDataVersionTag,
  getHeapCodeStatistics,
  getHeapSnapshot: unavailable('getHeapSnapshot'),
  getHeapSpaceStatistics,
  getHeapStatistics,
  setFlagsFromString,
  setHeapSnapshotNearHeapLimit() {},
  writeHeapSnapshot: unavailable('writeHeapSnapshot'),
  takeCoverage() {},
  stopCoverage() {},
  serialize: unavailable('serialize'),
  deserialize: unavailable('deserialize'),
  Serializer,
  Deserializer,
  DefaultSerializer: Serializer,
  DefaultDeserializer: Deserializer,
  isStringOneByteRepresentation(content) {
    return typeof content === 'string' && !/[^\u0000-ÿ]/.test(content);
  },
  promiseHooks: {
    onInit() { return () => {}; },
    onSettled() { return () => {}; },
    onBefore() { return () => {}; },
    onAfter() { return () => {}; },
    createHook() { return () => {}; },
  },
  startupSnapshot: {
    addSerializeCallback() {},
    addDeserializeCallback() {},
    setDeserializeMainFunction() {},
    isBuildingSnapshot() { return false; },
  },
  queryObjects: unavailable('queryObjects'),
};
