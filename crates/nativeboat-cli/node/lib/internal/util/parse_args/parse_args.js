'use strict';

// A focused implementation of Node's util.parseArgs object form. It covers
// named boolean/string options and positional arguments used by packaged apps.
function parseArgs(config) {
  config = config || {};
  const options = config.options || {};
  const args = config.args || process.argv.slice(2);
  const strict = config.strict !== false;
  const allowPositionals = config.allowPositionals === true;
  const values = {};
  const positionals = [];
  let positionalOnly = false;

  for (let index = 0; index < args.length; index++) {
    const token = args[index];
    if (positionalOnly) {
      positionals.push(token);
      continue;
    }
    if (token === '--') {
      positionalOnly = true;
      continue;
    }
    if (!token.startsWith('-') || token === '-') {
      if (!allowPositionals && strict) {
        throw new TypeError(`Unexpected positional argument: ${token}`);
      }
      positionals.push(token);
      continue;
    }

    const equals = token.indexOf('=');
    const name = token.startsWith('--')
      ? token.slice(2, equals < 0 ? undefined : equals)
      : token.slice(1);
    const definition = options[name];
    if (!definition) {
      if (strict) throw new TypeError(`Unknown option: ${token}`);
      continue;
    }
    const type = definition.type;
    if (type === 'boolean') {
      values[name] = equals >= 0 ? token.slice(equals + 1) !== 'false' : true;
    } else if (type === 'string') {
      if (equals >= 0) {
        values[name] = token.slice(equals + 1);
      } else if (index + 1 < args.length) {
        values[name] = args[++index];
      } else if (strict) {
        throw new TypeError(`Option requires an argument: ${token}`);
      }
    } else if (strict) {
      throw new TypeError(`Invalid option configuration for ${name}`);
    }
  }

  for (const name of Object.keys(options)) {
    const definition = options[name];
    if (values[name] === undefined && definition.default !== undefined) {
      values[name] = definition.default;
    }
    if (values[name] === undefined && definition.type === 'boolean') {
      values[name] = false;
    }
  }

  return config.tokens ? { values, positionals, tokens: [] } : { values, positionals };
}

module.exports = { parseArgs };
