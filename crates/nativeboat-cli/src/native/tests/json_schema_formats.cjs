'use strict';
// A small stand-in for ajv-formats' format definitions.
const date = /^(\d\d\d\d)-(\d\d)-(\d\d)$/;
const compare = (a, b) => (a > b ? 1 : a < b ? -1 : 0);
exports.fullFormats = { date: { validate: date, compare }, email: /^[^@\s]+@[^@\s]+$/ };
exports.fastFormats = exports.fullFormats;
exports.formatNames = Object.keys(exports.fullFormats);
