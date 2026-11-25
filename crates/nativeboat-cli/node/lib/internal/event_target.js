'use strict';

// nativeboat: the subset of Node.js v22.15.0 lib/internal/event_target.js
// that the bundled library modules use. EventTarget itself is not provided
// yet, so no object is an event target; the definitions are copied
// unchanged.

const {
  Symbol,
  SymbolFor,
} = primordials;

const kIsEventTarget = SymbolFor('nodejs.event_target');
const kEvents = Symbol('kEvents');
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
