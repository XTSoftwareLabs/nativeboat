'use strict';

// nativeboat: Node v22.15.0 transfer marking subset; workers and cloning
// are not provided by this source subset. The definition is unchanged. The
// messaging symbols that transferable classes key their hooks on are this
// module's own, and structuredClone is the runtime's.
const {
  privateSymbols: { transfer_mode_private_symbol },
  constants: { kDisallowCloneAndTransfer, kTransferable, kCloneable },
} = internalBinding('util');

function markTransferMode(obj, cloneable = false, transferable = false) {
  if ((typeof obj !== 'object' && typeof obj !== 'function') || obj === null)
    return;  // This object is a primitive and therefore already untransferable.
  let mode = kDisallowCloneAndTransfer;
  if (cloneable) mode |= kCloneable;
  if (transferable) mode |= kTransferable;
  obj[transfer_mode_private_symbol] = mode;
}

module.exports = {
  markTransferMode,
  structuredClone: globalThis.structuredClone,
  kClone: Symbol('messaging_clone_symbol'),
  kDeserialize: Symbol('messaging_deserialize_symbol'),
  kTransfer: Symbol('messaging_transfer_symbol'),
  kTransferList: Symbol('messaging_transfer_list_symbol'),
};
