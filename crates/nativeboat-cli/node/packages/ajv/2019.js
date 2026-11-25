'use strict';
// `ajv/dist/2019.js`: Ajv's JSON Schema 2019-09 class, interpreted. Exports
// are assigned as Ajv assigns them, so importers find the same named exports.
const { Ajv2019, ValidationError, MissingRefError } = require('./nativeboat-json-schema.js');
module.exports = exports = Ajv2019;
module.exports.Ajv2019 = Ajv2019;
Object.defineProperty(exports, '__esModule', { value: true });
exports.default = Ajv2019;
exports.ValidationError = ValidationError;
exports.MissingRefError = MissingRefError;
