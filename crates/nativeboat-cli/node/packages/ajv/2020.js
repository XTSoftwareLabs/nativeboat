'use strict';
// `ajv/dist/2020.js`: Ajv's JSON Schema 2020-12 class, interpreted. Exports
// are assigned as Ajv assigns them, so importers find the same named exports.
const { Ajv2020, ValidationError, MissingRefError } = require('./nativeboat-json-schema.js');
module.exports = exports = Ajv2020;
module.exports.Ajv2020 = Ajv2020;
Object.defineProperty(exports, '__esModule', { value: true });
exports.default = Ajv2020;
exports.ValidationError = ValidationError;
exports.MissingRefError = MissingRefError;
