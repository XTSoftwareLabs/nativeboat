'use strict';

function sequence(value, name) {
  if (Array.isArray(value)) return value;
  if (typeof value === 'string') return Array.from(value);
  throw new TypeError(`${name} must be a string or an array`);
}

// Preserve common prefixes and suffixes while reporting the changed middle
// as deletes followed by inserts. This is sufficient for startup consumers;
// unlike Node's Myers implementation it does not minimize edit distance.
function diff(actual, expected) {
  if (actual === expected) return [];
  const left = sequence(actual, 'actual');
  const right = sequence(expected, 'expected');
  let prefix = 0;
  while (prefix < left.length && prefix < right.length && left[prefix] === right[prefix]) prefix++;
  let suffix = 0;
  while (suffix < left.length - prefix && suffix < right.length - prefix &&
         left[left.length - 1 - suffix] === right[right.length - 1 - suffix]) suffix++;

  const result = [];
  if (prefix) result.push([0, left.slice(0, prefix)]);
  if (left.length > prefix + suffix) result.push([-1, left.slice(prefix, left.length - suffix)]);
  if (right.length > prefix + suffix) result.push([1, right.slice(prefix, right.length - suffix)]);
  if (suffix) result.push([0, left.slice(left.length - suffix)]);
  return result;
}

module.exports = { diff };
