'use strict';

// nativeboat: the subset of Node.js v22.15.0 lib/internal/event_target.js
// that the bundled library modules use. The runtime's own EventTarget class
// carries the `nodejs.event_target` mark and shows its listeners under the
// registered `nativeboat.event_target.events` symbol, which stands for
// Node's private kEvents.

const {
  Symbol,
  SymbolFor,
} = primordials;

const kIsEventTarget = SymbolFor('nodejs.event_target');
const kEvents = SymbolFor('nativeboat.event_target.events');
const kResistStopPropagation = Symbol('kResistStopPropagation');

function isEventTarget(obj) {
  return obj?.constructor?.[kIsEventTarget];
}

module.exports = {
  isEventTarget,
  kEvents,
  kIsEventTarget,
  kResistStopPropagation,
};
