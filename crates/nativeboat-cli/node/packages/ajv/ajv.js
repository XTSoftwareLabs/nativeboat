'use strict';
// `ajv`: Ajv's JSON Schema draft-07 class, interpreted. Exports are assigned
// as Ajv assigns them, so importers find the same named exports.
const { Ajv, ValidationError, MissingRefError } = require('./nativeboat-json-schema.js');
module.exports = exports = Ajv;
module.exports.Ajv = Ajv;
Object.defineProperty(exports, '__esModule', { value: true });
exports.default = Ajv;
exports.ValidationError = ValidationError;
exports.MissingRefError = MissingRefError;
