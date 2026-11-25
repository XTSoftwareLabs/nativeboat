'use strict';
// `ajv-formats` without Ajv's code generator: the package's own format
// definitions, and its `formatMaximum`-style keywords as validators.
const formats = require('./formats');

const LIMITS = {
  formatMaximum: ['<=', (order) => order > 0],
  formatMinimum: ['>=', (order) => order < 0],
  formatExclusiveMaximum: ['<', (order) => order >= 0],
  formatExclusiveMinimum: ['>', (order) => order <= 0],
};

const formatLimitDefinition = {
  keyword: Object.keys(LIMITS),
  type: 'string',
  schemaType: 'string',
  compile(limit, parentSchema, it) {
    const keyword = it.keyword;
    const [comparison, fails] = LIMITS[keyword];
    const ajv = it.self;
    const valid = () => true;
    if (ajv.opts.validateFormats === false) return valid;
    const format = ajv.formats[parentSchema.format];
    if (!format || format === true) return valid;
    if (typeof format !== 'object' || format instanceof RegExp || typeof format.compare !== 'function') {
      throw new Error(`"${keyword}": format "${parentSchema.format}" does not define "compare" function`);
    }
    function validate(data) {
      if (!fails(format.compare(data, limit))) return true;
      validate.errors = [{ keyword, params: { comparison, limit }, message: `should be ${comparison} ${limit}` }];
      return false;
    }
    return validate;
  },
};

function addFormats(ajv, list, definitions) {
  for (const name of list) ajv.addFormat(name, definitions[name]);
}

const formatsPlugin = (ajv, opts = { keywords: true }) => {
  if (Array.isArray(opts)) {
    addFormats(ajv, opts, formats.fullFormats);
    return ajv;
  }
  const definitions = opts.mode === 'fast' ? formats.fastFormats : formats.fullFormats;
  addFormats(ajv, opts.formats || formats.formatNames, definitions);
  if (opts.keywords) ajv.addKeyword(formatLimitDefinition);
  return ajv;
};

formatsPlugin.get = (name, mode = 'full') => {
  const definitions = mode === 'fast' ? formats.fastFormats : formats.fullFormats;
  const format = definitions[name];
  if (!format) throw new Error(`Unknown format "${name}"`);
  return format;
};

module.exports = exports = formatsPlugin;
Object.defineProperty(exports, '__esModule', { value: true });
exports.default = formatsPlugin;
