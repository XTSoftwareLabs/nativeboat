'use strict';
// JSON Schema validation with the API and error objects of Ajv 8. Ajv
// compiles each schema to JavaScript source that it runs through
// `new Function`; a compiled program has no JavaScript engine, so this
// module interprets schemas instead. Keywords run in Ajv's order, and in
// Ajv's groups by data type, so errors arrive in the order Ajv reports them.

const JSON_TYPES = ['array', 'boolean', 'integer', 'null', 'number', 'object', 'string'];

// Ajv's keyword groups in evaluation order. Keywords without a type run
// first, then those of each data type the data has.
const ANY = ['$ref', 'const', 'enum', 'not', 'anyOf', 'oneOf', 'allOf', 'if'];
const NUMBER = ['maximum', 'minimum', 'exclusiveMaximum', 'exclusiveMinimum', 'multipleOf', 'format'];
const STRING = ['maxLength', 'minLength', 'pattern', 'format'];
const OBJECT = [
  'maxProperties', 'minProperties', 'required', 'propertyNames', 'additionalProperties',
  'dependencies', 'properties', 'patternProperties',
];
const NEXT_OBJECT = ['dependentRequired', 'dependentSchemas'];
// Keywords with no validation of their own.
const ANNOTATIONS = [
  '$schema', '$id', '$defs', '$vocabulary', '$comment', 'definitions', '$async',
  'type', 'nullable', 'then', 'else', 'title', 'description', 'default', 'deprecated',
  'readOnly', 'writeOnly', 'examples', 'contentMediaType', 'contentEncoding',
];
const NEXT_ANNOTATIONS = ['minContains', 'maxContains', 'contentSchema'];
// Keywords Ajv supports that are not interpreted here.
const UNSUPPORTED = [
  '$dynamicRef', '$dynamicAnchor', '$recursiveRef', '$recursiveAnchor',
  'unevaluatedProperties', 'unevaluatedItems', 'discriminator',
];

const DIALECTS = {
  draft7: {
    meta: 'http://json-schema.org/draft-07/schema',
    array: ['maxItems', 'minItems', 'additionalItems', 'items', 'contains', 'uniqueItems'],
    next: false,
  },
  2019: {
    meta: 'https://json-schema.org/draft/2019-09/schema',
    array: ['maxItems', 'minItems', 'additionalItems', 'items', 'contains', 'uniqueItems'],
    next: true,
  },
  2020: {
    meta: 'https://json-schema.org/draft/2020-12/schema',
    array: ['maxItems', 'minItems', 'prefixItems', 'items', 'contains', 'uniqueItems'],
    next: true,
  },
};

const LIMITS = {
  maximum: ['<=', (data, limit) => data > limit],
  minimum: ['>=', (data, limit) => data < limit],
  exclusiveMaximum: ['<', (data, limit) => data >= limit],
  exclusiveMinimum: ['>', (data, limit) => data <= limit],
};

class ValidationError extends Error {
  constructor(errors) {
    super('validation failed');
    this.errors = errors;
    this.ajv = this.validation = true;
  }
}

// The base URL of schemas without `$id`, which Ajv reports as empty.
const ANONYMOUS = 'nativeboat-schema:';
let anonymous = 0;

class MissingRefError extends Error {
  constructor(resolver, baseId, ref, msg) {
    if (baseId.startsWith(ANONYMOUS)) baseId = '#';
    super(msg || `can't resolve reference ${ref} from id ${baseId}`);
    this.missingRef = resolveUrl(baseId, ref);
    this.missingSchema = normalizeId(this.missingRef.split('#')[0]);
  }
}

function normalizeId(id) {
  return id ? id.replace(/#$/, '') : '';
}

// The URL `ref` names from the document at `base`.
function resolveUrl(base, ref) {
  base = normalizeId(base);
  if (/^[a-z][a-z0-9+.-]*:/i.test(ref)) return normalizeId(ref);
  const document = base.split('#')[0];
  if (ref.startsWith('#')) return normalizeId(document + ref);
  if (ref === '') return document;
  if (ref.startsWith('/')) {
    const origin = /^[a-z][a-z0-9+.-]*:\/\/[^/]*/i.exec(document);
    return normalizeId((origin ? origin[0] : '') + ref);
  }
  const slash = document.lastIndexOf('/');
  return normalizeId(document.slice(0, slash + 1) + ref);
}

function escapePointer(text) {
  return String(text).replace(/~/g, '~0').replace(/\//g, '~1');
}

function unescapePointer(text) {
  return text.replace(/~1/g, '/').replace(/~0/g, '~');
}

function escapeFragment(text) {
  return typeof text === 'number' ? `${text}` : encodeURIComponent(escapePointer(text));
}

// The length of a string in code points, as Ajv counts it.
function ucs2length(text) {
  let length = 0;
  for (let i = 0; i < text.length; i++, length++) {
    const code = text.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
      const next = text.charCodeAt(i + 1);
      if ((next & 0xfc00) === 0xdc00) i++;
    }
  }
  return length;
}

// fast-deep-equal, which Ajv uses for `const`, `enum` and `uniqueItems`.
function equal(a, b) {
  if (a === b) return true;
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    if (a.constructor !== b.constructor) return false;
    if (Array.isArray(a)) {
      if (a.length !== b.length) return false;
      for (let i = a.length; i-- !== 0;) if (!equal(a[i], b[i])) return false;
      return true;
    }
    if (a.constructor === RegExp) return a.source === b.source && a.flags === b.flags;
    if (a.valueOf !== Object.prototype.valueOf) return a.valueOf() === b.valueOf();
    if (a.toString !== Object.prototype.toString) return a.toString() === b.toString();
    const keys = Object.keys(a);
    if (keys.length !== Object.keys(b).length) return false;
    for (let i = keys.length; i-- !== 0;) {
      if (!Object.prototype.hasOwnProperty.call(b, keys[i])) return false;
    }
    for (let i = keys.length; i-- !== 0;) if (!equal(a[keys[i]], b[keys[i]])) return false;
    return true;
  }
  return a !== a && b !== b;
}

function isType(type, data, strictNumbers) {
  switch (type) {
    case 'null': return data === null;
    case 'array': return Array.isArray(data);
    case 'object': return data !== null && typeof data === 'object' && !Array.isArray(data);
    case 'integer':
      return typeof data === 'number' && !(data % 1) && !isNaN(data) && (!strictNumbers || isFinite(data));
    case 'number': return typeof data === 'number' && (!strictNumbers || isFinite(data));
    default: return typeof data === type;
  }
}

// The data types a schema's `type` and `nullable` allow.
function schemaTypes(schema) {
  const types = schema.type === undefined ? [] : Array.isArray(schema.type) ? schema.type.slice() : [schema.type];
  if (!types.every((type) => JSON_TYPES.includes(type))) {
    throw new Error('type must be JSONType or JSONType[]: ' + types.join(','));
  }
  if (schema.nullable === true) {
    if (schema.type === undefined) throw new Error('"nullable" cannot be used without "type"');
    if (!types.includes('null')) types.push('null');
  }
  return types;
}

// Keywords Ajv defines without validation code of their own; a schema
// with them still validates.
const DEFINED = ['type', 'nullable', 'then', 'else', '$comment', '$id', 'minContains', 'maxContains'];

// Whether a schema validates nothing, so Ajv generates no code for it.
function alwaysValid(ajv, schema) {
  if (typeof schema === 'boolean') return schema;
  for (const key in schema) if (ajv._rules.has(key) || DEFINED.includes(key)) return false;
  return true;
}

function own(object) {
  return Object.keys(object).filter((key) => key !== '__proto__');
}

function clone(value) {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

function errorObject(scope, at, keyword, params, message) {
  const error = {
    instancePath: at.path,
    schemaPath: `${at.schemaPath}/${keyword}`,
    keyword,
    params,
  };
  const { opts } = scope.ajv;
  if (opts.messages !== false) error.message = message;
  if (opts.verbose) {
    error.schema = at.schema[keyword];
    error.parentSchema = at.schema;
    error.data = at.data;
  }
  if (at.propertyName !== undefined) error.propertyName = at.propertyName;
  return error;
}

// Reports a keyword's error. Like Ajv outside `allErrors` and composite
// keywords, a failing keyword's own error replaces the errors its
// subschemas reported, unless the keyword appends to them.
function fail(scope, at, start, error, append) {
  if (!append && !at.all && !at.composite) scope.errors.length = start;
  scope.errors.push(error);
  return false;
}

function stops(at) {
  return !at.all;
}

function child(at, key, schemaPath, data) {
  return {
    ...at,
    path: `${at.path}/${escapePointer(key)}`,
    schemaPath,
    parent: data,
    key,
  };
}

function nested(at, schemaPath, composite) {
  return { ...at, schemaPath, composite: at.composite || Boolean(composite) };
}

function regExp(scope, pattern) {
  const { ajv } = scope;
  let compiled = ajv._patterns.get(pattern);
  if (!compiled) {
    compiled = new RegExp(pattern, ajv.opts.unicodeRegExp === false ? '' : 'u');
    ajv._patterns.set(pattern, compiled);
  }
  compiled.lastIndex = 0;
  return compiled;
}

function typeError(scope, at, schema, start) {
  const type = schema.type;
  const error = errorObject(scope, at, 'type', { type }, `must be ${Array.isArray(type) ? type.join(',') : type}`);
  return fail(scope, at, start, error, true);
}

// Validates `data` against `schema`; reports errors into `scope.errors`.
function check(scope, schema, data, at) {
  at = { ...at, schema, data };
  if (typeof schema === 'boolean') {
    if (schema) return true;
    const error = errorObject(scope, at, 'false schema', {}, 'boolean schema is false');
    return fail(scope, at, scope.errors.length, error, true);
  }
  const { ajv } = scope;
  const start = scope.errors.length;
  const types = schemaTypes(schema);
  const strictNumbers = ajv.opts.strictNumbers;
  const plan = keywordPlan(ajv, schema);
  const typesUpfront = types.length > 0 && !(types.length === 1 && plan.some(({ group }) => group.type === types[0]));
  if (typesUpfront && !types.some((type) => isType(type, data, strictNumbers))) {
    typeError(scope, at, schema, start);
    if (stops(at)) return false;
  }
  if (schema.$ref !== undefined && !Object.keys(schema).some((key) => key !== '$ref' && ajv._rules.has(key))) {
    return ref(scope, schema, data, at, start) && scope.errors.length === start;
  }
  for (const { group, used } of plan) {
    if (group.type !== undefined) {
      if (isType(group.type, data, strictNumbers)) {
        if (ajv.opts.useDefaults && !at.composite) assignDefaults(scope, schema, data, group.type, at);
        if (!keywords(scope, group, used, schema, data, at, start) && stops(at)) return false;
      } else if (!typesUpfront && types.length === 1 && types[0] === group.type) {
        typeError(scope, at, schema, start);
      }
    } else if (!keywords(scope, group, used, schema, data, at, start) && stops(at)) {
      return false;
    }
    if (stops(at) && scope.errors.length > start) return false;
  }
  return scope.errors.length === start;
}

// The groups a schema uses, each with the keywords it has, in rule order.
// Like Ajv's compiled validators, the plan is made once per schema object.
function keywordPlan(ajv, schema) {
  let plan = ajv._plans.get(schema);
  if (plan) return plan;
  plan = [];
  for (const group of ajv._groups) {
    const used = group.rules.filter((keyword) => schema[keyword] !== undefined);
    if (used.length > 0) plan.push({ group, used });
  }
  if (typeof schema === 'object' && schema !== null) ajv._plans.set(schema, plan);
  return plan;
}

function keywords(scope, group, used, schema, data, at, start) {
  let valid = true;
  for (const keyword of used) {
    const before = scope.errors.length;
    const rule = scope.ajv._rules.get(keyword);
    const ok = typeof rule === 'function'
      ? rule(scope, schema, data, at, before, group.type)
      : custom(scope, rule, keyword, schema, data, at, before);
    if (!ok || scope.errors.length > before) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

function assignDefaults(scope, schema, data, type, at) {
  const empty = scope.ajv.opts.useDefaults === 'empty';
  const assign = (key, value) => {
    if (value === undefined) return;
    const current = data[key];
    if (current === undefined || (empty && (current === null || current === ''))) data[key] = clone(value);
  };
  if (type === 'object' && schema.properties) {
    for (const key in schema.properties) {
      const property = schema.properties[key];
      if (property && typeof property === 'object') assign(key, property.default);
    }
  } else if (type === 'array' && Array.isArray(schema.items)) {
    schema.items.forEach((item, index) => {
      if (item && typeof item === 'object') assign(index, item.default);
    });
  }
}

function ref(scope, schema, data, at) {
  const target = scope.ajv._resolve(at.base, schema.$ref);
  const ok = check(scope, target.schema, data, {
    ...at,
    schemaPath: target.schemaPath,
    base: target.base,
  });
  return ok;
}

function limitNumber(keyword) {
  const [comparison, fails] = LIMITS[keyword];
  return (scope, schema, data, at, start) => {
    const limit = schema[keyword];
    if (!fails(data, limit) && !isNaN(data)) return true;
    return fail(scope, at, start, errorObject(scope, at, keyword, { comparison, limit }, `must be ${comparison} ${limit}`));
  };
}

function limitCount(keyword, count, noun) {
  const more = keyword.startsWith('max');
  return (scope, schema, data, at, start) => {
    const limit = schema[keyword];
    const length = count(scope, data);
    if (more ? length <= limit : length >= limit) return true;
    const message = `must NOT have ${more ? 'more' : 'fewer'} than ${limit} ${noun}`;
    return fail(scope, at, start, errorObject(scope, at, keyword, { limit }, message));
  };
}

const stringLength = (scope, data) => (scope.ajv.opts.unicode === false ? data.length : ucs2length(data));
const itemCount = (scope, data) => data.length;
const propertyCount = (scope, data) => Object.keys(data).length;

function missing(scope, at, start, keyword, data, properties, params, message) {
  let valid = true;
  for (const property of properties) {
    if (data[property] !== undefined) continue;
    valid = false;
    fail(scope, at, start, errorObject(scope, at, keyword, params(property), message(property)), true);
    if (stops(at)) return false;
  }
  return valid;
}

function propertyDependencies(scope, keyword, dependencies, data, at, start) {
  let valid = true;
  for (const property in dependencies) {
    const deps = dependencies[property];
    if (!Array.isArray(deps) || deps.length === 0 || data[property] === undefined) continue;
    const depsCount = deps.length;
    const text = deps.join(', ');
    const message = `must have ${depsCount === 1 ? 'property' : 'properties'} ${text} when property ${property} is present`;
    const ok = missing(scope, at, start, keyword, data, deps,
      (missingProperty) => ({ property, missingProperty, depsCount, deps: text }), () => message);
    if (!ok) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

function schemaDependencies(scope, keyword, dependencies, data, at) {
  let valid = true;
  for (const property in dependencies) {
    const dependency = dependencies[property];
    if (Array.isArray(dependency) || alwaysValid(scope.ajv, dependency) || data[property] === undefined) continue;
    const path = `${at.schemaPath}/${keyword}/${escapeFragment(property)}`;
    if (!check(scope, dependency, data, nested(at, path))) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

// Items from `from` on, against the `keyword` schema, as `items` and
// `additionalItems` apply.
function restItems(scope, keyword, schema, data, at, start, from) {
  if (schema === false) {
    if (data.length <= from) return true;
    return fail(scope, at, start, errorObject(scope, at, keyword, { limit: from }, `must NOT have more than ${from} items`));
  }
  if (alwaysValid(scope.ajv, schema)) return true;
  let valid = true;
  for (let i = from; i < data.length; i++) {
    if (!check(scope, schema, data[i], child(at, i, `${at.schemaPath}/${keyword}`, data))) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

function tuple(scope, keyword, schemas, data, at) {
  let valid = true;
  for (let i = 0; i < schemas.length; i++) {
    if (alwaysValid(scope.ajv, schemas[i]) || data.length <= i) continue;
    if (!check(scope, schemas[i], data[i], child(at, i, `${at.schemaPath}/${keyword}/${i}`, data))) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

function additionalProperty(scope, schema, key) {
  const properties = schema.properties ? own(schema.properties) : [];
  if (properties.includes(key)) return false;
  const patterns = schema.patternProperties ? own(schema.patternProperties) : [];
  return !patterns.some((pattern) => regExp(scope, pattern).test(key));
}

function additionalProperties(scope, schema, data, at, start, rule) {
  const { opts } = scope.ajv;
  const remove = opts.removeAdditional;
  if (remove !== 'all' && alwaysValid(scope.ajv, rule)) return true;
  let valid = true;
  for (const key in data) {
    if (!additionalProperty(scope, schema, key)) continue;
    if (remove === 'all' || (remove && rule === false)) {
      delete data[key];
      continue;
    }
    if (rule === false) {
      valid = false;
      fail(scope, at, start, errorObject(scope, at, 'additionalProperties', { additionalProperty: key },
        'must NOT have additional properties'));
      if (stops(at)) return false;
      continue;
    }
    if (typeof rule !== 'object' || alwaysValid(scope.ajv, rule)) continue;
    const path = `${at.schemaPath}/additionalProperties`;
    if (remove === 'failing') {
      const before = scope.errors.length;
      if (!check(scope, rule, data[key], { ...child(at, key, path, data), composite: true, all: false })) {
        scope.errors.length = before;
        delete data[key];
      }
    } else if (!check(scope, rule, data[key], child(at, key, path, data))) {
      valid = false;
      if (stops(at)) return false;
    }
  }
  return valid;
}

// The built-in keywords: each validates `data` against `schema[keyword]`.
const RULES = {
  $ref: ref,
  const(scope, schema, data, at, start) {
    if (equal(data, schema.const)) return true;
    return fail(scope, at, start, errorObject(scope, at, 'const', { allowedValue: schema.const }, 'must be equal to constant'));
  },
  enum(scope, schema, data, at, start) {
    if (schema.enum.some((value) => equal(data, value))) return true;
    return fail(scope, at, start, errorObject(scope, at, 'enum', { allowedValues: schema.enum },
      'must be equal to one of the allowed values'));
  },
  not(scope, schema, data, at, start) {
    const valid = check(scope, schema.not, data, nested(at, `${at.schemaPath}/not`, true));
    scope.errors.length = start;
    if (!valid) return true;
    return fail(scope, at, start, errorObject(scope, at, 'not', {}, 'must NOT be valid'));
  },
  anyOf(scope, schema, data, at, start) {
    if (schema.anyOf.some((branch) => alwaysValid(scope.ajv, branch))) return true;
    for (let i = 0; i < schema.anyOf.length; i++) {
      if (check(scope, schema.anyOf[i], data, nested(at, `${at.schemaPath}/anyOf/${i}`, true))) {
        scope.errors.length = start;
        return true;
      }
    }
    return fail(scope, at, start, errorObject(scope, at, 'anyOf', {}, 'must match a schema in anyOf'), true);
  },
  oneOf(scope, schema, data, at, start) {
    let valid = false;
    let passing = null;
    // A second passing schema ends the search, as Ajv's nested code does.
    for (let i = 0; i < schema.oneOf.length; i++) {
      const branch = schema.oneOf[i];
      const ok = alwaysValid(scope.ajv, branch)
        || check(scope, branch, data, nested(at, `${at.schemaPath}/oneOf/${i}`, true));
      if (ok && valid) {
        valid = false;
        passing = [passing, i];
        break;
      }
      if (ok) {
        valid = true;
        passing = i;
      }
    }
    if (valid) {
      scope.errors.length = start;
      return true;
    }
    return fail(scope, at, start, errorObject(scope, at, 'oneOf', { passingSchemas: passing },
      'must match exactly one schema in oneOf'), true);
  },
  allOf(scope, schema, data, at) {
    let valid = true;
    for (let i = 0; i < schema.allOf.length; i++) {
      if (alwaysValid(scope.ajv, schema.allOf[i])) continue;
      if (!check(scope, schema.allOf[i], data, nested(at, `${at.schemaPath}/allOf/${i}`))) {
        valid = false;
        if (stops(at)) return false;
      }
    }
    return valid;
  },
  if(scope, schema, data, at, start) {
    const has = (keyword) => schema[keyword] !== undefined && !alwaysValid(scope.ajv, schema[keyword]);
    if (!has('then') && !has('else')) return true;
    const matches = check(scope, schema.if, data, { ...nested(at, `${at.schemaPath}/if`, true), all: false });
    scope.errors.length = start;
    const clause = matches ? 'then' : 'else';
    if (!has(clause)) return true;
    if (check(scope, schema[clause], data, nested(at, `${at.schemaPath}/${clause}`))) return true;
    if (!at.all && !at.composite) return false;
    return fail(scope, at, start, errorObject(scope, at, 'if', { failingKeyword: clause },
      `must match "${clause}" schema`), true);
  },
  maximum: limitNumber('maximum'),
  minimum: limitNumber('minimum'),
  exclusiveMaximum: limitNumber('exclusiveMaximum'),
  exclusiveMinimum: limitNumber('exclusiveMinimum'),
  multipleOf(scope, schema, data, at, start) {
    const multipleOf = schema.multipleOf;
    const result = data / multipleOf;
    const precision = scope.ajv.opts.multipleOfPrecision;
    const invalid = precision
      ? Math.abs(Math.round(result) - result) > Math.pow(10, -precision)
      : result !== parseInt(result);
    if (!invalid) return true;
    return fail(scope, at, start, errorObject(scope, at, 'multipleOf', { multipleOf }, `must be multiple of ${multipleOf}`));
  },
  maxLength: limitCount('maxLength', stringLength, 'characters'),
  minLength: limitCount('minLength', stringLength, 'characters'),
  pattern(scope, schema, data, at, start) {
    const pattern = schema.pattern;
    if (regExp(scope, pattern).test(data)) return true;
    return fail(scope, at, start, errorObject(scope, at, 'pattern', { pattern }, `must match pattern "${pattern}"`));
  },
  format(scope, schema, data, at, start, type) {
    const { ajv } = scope;
    if (ajv.opts.validateFormats === false) return true;
    const name = schema.format;
    const format = ajv.formats[name];
    if (format === undefined || format === true) return true;
    const definition = typeof format === 'object' && !(format instanceof RegExp) ? format : { validate: format };
    if ((definition.type || 'string') !== type) return true;
    const validate = definition.validate;
    const valid = validate instanceof RegExp ? (validate.lastIndex = 0, validate.test(data)) : validate(data);
    if (valid) return true;
    return fail(scope, at, start, errorObject(scope, at, 'format', { format: name }, `must match format "${name}"`));
  },
  maxItems: limitCount('maxItems', itemCount, 'items'),
  minItems: limitCount('minItems', itemCount, 'items'),
  prefixItems(scope, schema, data, at) {
    return tuple(scope, 'prefixItems', schema.prefixItems, data, at);
  },
  items(scope, schema, data, at, start) {
    if (Array.isArray(schema.items)) return tuple(scope, 'items', schema.items, data, at);
    if (alwaysValid(scope.ajv, schema.items)) return true;
    const from = Array.isArray(schema.prefixItems) ? schema.prefixItems.length : 0;
    if (from > 0) return restItems(scope, 'items', schema.items, data, at, start, from);
    return restItems(scope, 'items', schema.items, data, at, start, 0);
  },
  additionalItems(scope, schema, data, at, start) {
    if (!Array.isArray(schema.items)) return true;
    return restItems(scope, 'additionalItems', schema.additionalItems, data, at, start, schema.items.length);
  },
  contains(scope, schema, data, at, start) {
    const next = scope.ajv._dialect.next || scope.ajv.opts.next;
    const min = next && schema.minContains !== undefined ? schema.minContains : 1;
    const max = next ? schema.maxContains : undefined;
    const params = max === undefined ? { minContains: min } : { minContains: min, maxContains: max };
    const message = max === undefined
      ? `must contain at least ${min} valid item(s)`
      : `must contain at least ${min} and no more than ${max} valid item(s)`;
    const failure = () => fail(scope, at, start, errorObject(scope, at, 'contains', params, message));
    if (max === undefined && min === 0) return true;
    if (max !== undefined && min > max) return failure();
    if (alwaysValid(scope.ajv, schema.contains)) {
      if (data.length >= min && (max === undefined || data.length <= max)) return true;
      return failure();
    }
    let valid = min === 0;
    let count = 0;
    for (let i = 0; i < data.length; i++) {
      const item = { ...child(at, i, `${at.schemaPath}/contains`, data), composite: true };
      if (!check(scope, schema.contains, data[i], item)) continue;
      count++;
      if (max === undefined) {
        if (count >= min) {
          valid = true;
          break;
        }
      } else {
        if (count > max) {
          valid = false;
          break;
        }
        if (count >= min) valid = true;
      }
    }
    if (valid) {
      scope.errors.length = start;
      return true;
    }
    return failure();
  },
  uniqueItems(scope, schema, data, at, start) {
    if (schema.uniqueItems !== true || data.length <= 1) return true;
    const items = schema.items && typeof schema.items === 'object' && !Array.isArray(schema.items)
      ? schemaTypes(schema.items) : [];
    const report = (i, j) => fail(scope, at, start, errorObject(scope, at, 'uniqueItems', { i, j },
      `must NOT have duplicate items (items ## ${j} and ${i} are identical)`));
    if (items.length > 0 && !items.some((type) => type === 'object' || type === 'array')) {
      const indices = {};
      for (let i = data.length; i--;) {
        let item = data[i];
        if (!items.some((type) => isType(type, item, scope.ajv.opts.strictNumbers))) continue;
        if (items.length > 1 && typeof item === 'string') item += '_';
        if (typeof indices[item] === 'number') return report(i, indices[item]);
        indices[item] = i;
      }
      return true;
    }
    for (let i = data.length; i--;) {
      for (let j = i; j--;) if (equal(data[i], data[j])) return report(i, j);
    }
    return true;
  },
  maxProperties: limitCount('maxProperties', propertyCount, 'properties'),
  minProperties: limitCount('minProperties', propertyCount, 'properties'),
  required(scope, schema, data, at, start) {
    return missing(scope, at, start, 'required', data, schema.required,
      (missingProperty) => ({ missingProperty }),
      (missingProperty) => `must have required property '${missingProperty}'`);
  },
  propertyNames(scope, schema, data, at, start) {
    if (alwaysValid(scope.ajv, schema.propertyNames)) return true;
    let valid = true;
    for (const key in data) {
      const name = { ...nested(at, `${at.schemaPath}/propertyNames`, true), propertyName: key };
      if (check(scope, schema.propertyNames, key, name)) continue;
      valid = false;
      fail(scope, at, start, errorObject(scope, at, 'propertyNames', { propertyName: key }, 'property name must be valid'), true);
      if (stops(at)) return false;
    }
    return valid;
  },
  additionalProperties(scope, schema, data, at, start) {
    return additionalProperties(scope, schema, data, at, start, schema.additionalProperties);
  },
  dependencies(scope, schema, data, at, start) {
    return propertyDependencies(scope, 'dependencies', schema.dependencies, data, at, start)
      && schemaDependencies(scope, 'dependencies', schema.dependencies, data, at);
  },
  dependentRequired(scope, schema, data, at, start) {
    return propertyDependencies(scope, 'dependentRequired', schema.dependentRequired, data, at, start);
  },
  dependentSchemas(scope, schema, data, at) {
    return schemaDependencies(scope, 'dependentSchemas', schema.dependentSchemas, data, at);
  },
  properties(scope, schema, data, at, start) {
    const { opts } = scope.ajv;
    let valid = true;
    if (opts.removeAdditional === 'all' && schema.additionalProperties === undefined) {
      additionalProperties(scope, schema, data, at, start, undefined);
    }
    for (const key of own(schema.properties)) {
      const property = schema.properties[key];
      if (alwaysValid(scope.ajv, property)) continue;
      const hasDefault = opts.useDefaults && !at.composite && property.default !== undefined;
      if (!hasDefault && data[key] === undefined) continue;
      const path = `${at.schemaPath}/properties/${escapeFragment(key)}`;
      if (!check(scope, property, data[key], child(at, key, path, data))) {
        valid = false;
        if (stops(at)) return false;
      }
    }
    return valid;
  },
  patternProperties(scope, schema, data, at) {
    let valid = true;
    for (const pattern of own(schema.patternProperties)) {
      const property = schema.patternProperties[pattern];
      if (alwaysValid(scope.ajv, property)) continue;
      const path = `${at.schemaPath}/patternProperties/${escapeFragment(pattern)}`;
      for (const key in data) {
        if (!regExp(scope, pattern).test(key)) continue;
        if (!check(scope, property, data[key], child(at, key, path, data))) {
          valid = false;
          if (stops(at)) return false;
        }
      }
    }
    return valid;
  },
};

// A user-defined keyword: Ajv's `validate`, `compile` and `macro` forms.
function custom(scope, definition, keyword, schema, data, at, start) {
  const value = schema[keyword];
  if (definition.valid === true) return true;
  const keywordError = () => fail(scope, at, start, errorObject(scope, at, keyword, {},
    `must pass "${keyword}" keyword validation`), true);
  if (definition.macro) {
    const expanded = scope.ajv._macro(definition, keyword, value, schema);
    if (check(scope, expanded, data, nested(at, `${at.schemaPath}/${keyword}`, true))) return true;
    return keywordError();
  }
  const dataContext = {
    instancePath: at.path,
    parentData: at.parent,
    parentDataProperty: at.key,
    rootData: scope.rootData,
    dynamicAnchors: {},
  };
  let validate;
  let valid;
  if (definition.compile) {
    validate = scope.ajv._compiledKeyword(definition, keyword, value, schema);
    valid = validate.call(scope.context, data, dataContext);
  } else {
    validate = definition.validate;
    valid = definition.schema === false
      ? validate.call(scope.context, data, dataContext)
      : validate.call(scope.context, value, data, schema, dataContext);
  }
  if (valid) return true;
  if (definition.errors !== false && Array.isArray(validate.errors) && validate.errors.length) {
    for (const reported of validate.errors) {
      const error = { ...reported };
      if (error.instancePath === undefined) error.instancePath = at.path;
      if (error.schemaPath === undefined) error.schemaPath = `${at.schemaPath}/${keyword}`;
      if (scope.ajv.opts.verbose) {
        error.schema = value;
        error.data = data;
      }
      scope.errors.push(error);
    }
    return false;
  }
  return keywordError();
}

const SINGLE = [
  'not', 'if', 'then', 'else', 'additionalProperties', 'additionalItems', 'contains',
  'propertyNames', 'contentSchema',
];
const LISTS = ['allOf', 'anyOf', 'oneOf', 'prefixItems'];
const MAPS = ['properties', 'patternProperties', '$defs', 'definitions', 'dependentSchemas', 'dependencies'];

// Calls `visit(schema, path, base)` for a schema and each of its subschemas.
function walk(schema, path, base, visit) {
  if (!schema || typeof schema !== 'object') return;
  if (typeof schema.$id === 'string' && !schema.$id.startsWith('#')) base = resolveUrl(base, schema.$id);
  visit(schema, path, base);
  for (const keyword of SINGLE) {
    if (schema[keyword] !== undefined) walk(schema[keyword], `${path}/${keyword}`, base, visit);
  }
  if (schema.items !== undefined) {
    if (Array.isArray(schema.items)) {
      schema.items.forEach((item, i) => walk(item, `${path}/items/${i}`, base, visit));
    } else {
      walk(schema.items, `${path}/items`, base, visit);
    }
  }
  for (const keyword of LISTS) {
    if (Array.isArray(schema[keyword])) {
      schema[keyword].forEach((item, i) => walk(item, `${path}/${keyword}/${i}`, base, visit));
    }
  }
  for (const keyword of MAPS) {
    const map = schema[keyword];
    if (!map || typeof map !== 'object') continue;
    for (const key of own(map)) {
      if (!Array.isArray(map[key])) walk(map[key], `${path}/${keyword}/${escapeFragment(key)}`, base, visit);
    }
  }
}

const DEFAULT_OPTIONS = {
  strict: undefined,
  allErrors: false,
  useDefaults: false,
  removeAdditional: false,
  validateFormats: true,
  validateSchema: true,
  messages: true,
  verbose: false,
  unicodeRegExp: true,
  inlineRefs: true,
  addUsedSchema: true,
};

const UNSUPPORTED_OPTIONS = ['coerceTypes', '$data', 'discriminator', 'jtd'];

class Ajv {
  constructor(opts = {}) {
    const strict = opts.strict;
    this.opts = {
      ...DEFAULT_OPTIONS,
      ...opts,
      strictSchema: opts.strictSchema ?? strict ?? true,
      strictNumbers: opts.strictNumbers ?? strict ?? true,
      strictTypes: opts.strictTypes ?? strict ?? 'log',
      strictTuples: opts.strictTuples ?? strict ?? 'log',
      strictRequired: opts.strictRequired ?? strict ?? false,
      code: { ...opts.code },
    };
    for (const option of UNSUPPORTED_OPTIONS) {
      if (opts[option]) {
        throw new Error(`The Ajv option "${option}" is not supported in compiled applications`);
      }
    }
    this.errors = null;
    this.logger = opts.logger === false
      ? { log() {}, warn() {}, error() {} }
      : opts.logger || console;
    this.schemas = {};
    this.refs = {};
    this.formats = {};
    this._cache = new Map();
    this._patterns = new Map();
    this._documents = new Map();
    this._keywordCache = new Map();
    this._plans = new WeakMap();
    this._metas = new Set([this._dialect.meta]);
    this._rules = new Map();
    this._known = new Set([...ANNOTATIONS, ...UNSUPPORTED]);
    if (this._dialect.next || opts.next) for (const keyword of NEXT_ANNOTATIONS) this._known.add(keyword);
    const object = this._dialect.next || opts.next ? [...OBJECT, ...NEXT_OBJECT] : OBJECT;
    this._groups = [
      { type: undefined, rules: ANY.slice() },
      { type: 'number', rules: NUMBER.slice() },
      { type: 'string', rules: STRING.slice() },
      { type: 'array', rules: this._dialect.array.slice() },
      { type: 'object', rules: object.slice() },
    ];
    for (const group of this._groups) {
      for (const keyword of group.rules) {
        this._rules.set(keyword, RULES[keyword]);
        this._known.add(keyword);
      }
    }
    if (opts.formats) for (const name in opts.formats) this.addFormat(name, opts.formats[name]);
    if (opts.keywords) this.addVocabulary(opts.keywords);
    if (opts.schemas) {
      if (Array.isArray(opts.schemas)) this.addSchema(opts.schemas);
      else for (const key in opts.schemas) this.addSchema(opts.schemas[key], key);
    }
  }

  get _dialect() {
    return DIALECTS.draft7;
  }

  validate(schemaKeyRef, data) {
    let validate;
    if (typeof schemaKeyRef === 'string') {
      validate = this.getSchema(schemaKeyRef);
      if (!validate) throw new Error(`no schema with key or ref "${schemaKeyRef}"`);
    } else {
      validate = this.compile(schemaKeyRef);
    }
    const valid = validate(data);
    if (!('$async' in validate)) this.errors = validate.errors;
    return valid;
  }

  compile(schema, _meta) {
    return this._add(schema, undefined, '', true).validate;
  }

  compileAsync(schema, meta) {
    try {
      return Promise.resolve(this.compile(schema, meta));
    } catch (error) {
      return Promise.reject(error);
    }
  }

  addSchema(schema, key, _meta, _validateSchema) {
    if (Array.isArray(schema)) {
      for (const item of schema) this.addSchema(item, undefined, _meta, _validateSchema);
      return this;
    }
    let id;
    if (typeof schema === 'object' && schema !== null) {
      id = schema.$id;
      if (id !== undefined && typeof id !== 'string') throw new Error('schema $id must be string');
    }
    key = normalizeId(key || id);
    this._checkUnique(key);
    this.schemas[key] = this._add(schema, _validateSchema, key, true);
    return this;
  }

  addMetaSchema(schema, key, _validateSchema) {
    this.addSchema(schema, key, true, _validateSchema);
    const id = normalizeId(key || (schema && schema.$id));
    if (id) this._metas.add(id);
    return this;
  }

  validateSchema(schema, throwOrLogError) {
    if (typeof schema === 'boolean') return true;
    const errors = this._schemaErrors(schema);
    this.errors = errors.length ? errors : null;
    if (errors.length && throwOrLogError) {
      const message = 'schema is invalid: ' + this.errorsText(errors);
      if (this.opts.validateSchema === 'log') this.logger.error(message);
      else throw new Error(message);
    }
    return errors.length === 0;
  }

  getSchema(keyRef) {
    const key = normalizeId(keyRef);
    let entry = this.schemas[key] || this.refs[key];
    if (typeof entry === 'string') entry = this.schemas[entry] || this.refs[entry];
    if (entry) return entry.validate;
    try {
      const target = this._resolve('', keyRef);
      return this._add(target.schema, false, target.base, false).validate;
    } catch {
      return undefined;
    }
  }

  removeSchema(schemaKeyRef) {
    if (schemaKeyRef === undefined) {
      this.schemas = {};
      this.refs = {};
      this._cache.clear();
      this._documents.clear();
      return this;
    }
    if (typeof schemaKeyRef === 'string') {
      const key = normalizeId(schemaKeyRef);
      const entry = this.schemas[key] || this.refs[key];
      if (entry && typeof entry === 'object') this._cache.delete(entry.schema);
      delete this.schemas[key];
      delete this.refs[key];
      this._documents.delete(key);
      return this;
    }
    if (typeof schemaKeyRef === 'object' && schemaKeyRef !== null) {
      this._cache.delete(schemaKeyRef);
      const id = normalizeId(schemaKeyRef.$id);
      if (id) {
        delete this.schemas[id];
        delete this.refs[id];
        this._documents.delete(id);
      }
      return this;
    }
    throw new Error('ajv.removeSchema: invalid parameter');
  }

  addVocabulary(definitions) {
    for (const definition of definitions) this.addKeyword(definition);
    return this;
  }

  addKeyword(kwdOrDef, def) {
    let keyword;
    if (typeof kwdOrDef === 'string') {
      keyword = kwdOrDef;
      if (typeof def === 'object') def.keyword = keyword;
    } else if (typeof kwdOrDef === 'object' && def === undefined) {
      def = kwdOrDef;
      keyword = def.keyword;
      if (Array.isArray(keyword) && !keyword.length) throw new Error('addKeywords: keyword must be string or non-empty array');
    } else {
      throw new Error('invalid addKeywords parameters');
    }
    const keywords = Array.isArray(keyword) ? keyword : [keyword];
    for (const name of keywords) {
      if (!/^[a-z_$][a-z0-9_$:-]*$/i.test(name)) throw new Error(`Keyword ${name} has invalid name`);
      if (this._known.has(name) || this._rules.has(name)) throw new Error(`Keyword ${name} is already defined`);
    }
    if (!def) {
      for (const name of keywords) this._known.add(name);
      return this;
    }
    if (def.code) {
      throw new Error(`Keyword ${keywords[0]} generates code, which is not supported in compiled applications`);
    }
    if (def.$data || def.async) {
      throw new Error(`Keyword ${keywords[0]} uses $data or async validation, which is not supported in compiled applications`);
    }
    const types = def.type === undefined ? [undefined] : Array.isArray(def.type) ? def.type : [def.type];
    for (const name of keywords) {
      this._known.add(name);
      if (!def.validate && !def.compile && !def.macro) continue;
      this._rules.set(name, def);
      for (let type of types) {
        if (type === 'integer') type = 'number';
        let group = this._groups.find((item) => item.type === type);
        if (!group) {
          group = { type, rules: [] };
          this._groups.push(group);
        }
        if (def.before && group.rules.includes(def.before)) {
          group.rules.splice(group.rules.indexOf(def.before), 0, name);
        } else {
          group.rules.push(name);
        }
      }
    }
    this._plans = new WeakMap();
    return this;
  }

  getKeyword(keyword) {
    const rule = this._rules.get(keyword);
    if (rule) return typeof rule === 'function' ? true : rule;
    return this._known.has(keyword);
  }

  removeKeyword(keyword) {
    this._rules.delete(keyword);
    this._known.delete(keyword);
    for (const group of this._groups) {
      const index = group.rules.indexOf(keyword);
      if (index >= 0) group.rules.splice(index, 1);
    }
    this._plans = new WeakMap();
    return this;
  }

  addFormat(name, format) {
    if (typeof format === 'string') format = new RegExp(format);
    if (format && typeof format === 'object' && !(format instanceof RegExp) && format.async) {
      throw new Error(`Format "${name}" is asynchronous, which is not supported in compiled applications`);
    }
    this.formats[name] = format;
    return this;
  }

  errorsText(errors = this.errors, { separator = ', ', dataVar = 'data' } = {}) {
    if (!errors || errors.length === 0) return 'No errors';
    return errors.map((error) => `${dataVar}${error.instancePath} ${error.message}`).reduce((text, message) => text + separator + message);
  }

  _checkUnique(id) {
    if (this.schemas[id] || this.refs[id]) throw new Error(`schema with key or id "${id}" already exists`);
  }

  // Checks a schema and makes its validation function; `register` records
  // the schema under its id, as Ajv does for compiled and added schemas.
  _add(schema, validateSchema, base, register) {
    if (typeof schema !== 'object' && typeof schema !== 'boolean') {
      throw new Error('schema must be object or boolean');
    }
    const cached = this._cache.get(schema);
    if (cached) return cached;
    let id = '';
    if (typeof schema === 'object' && schema !== null) {
      if (schema.$id !== undefined && typeof schema.$id !== 'string') throw new Error('schema $id must be string');
      id = normalizeId(schema.$id || base || '');
    }
    if (validateSchema !== false && this.opts.validateSchema !== false) this.validateSchema(schema, true);
    // A schema without an id still resolves `#...` references inside itself.
    const entry = { schema, baseId: id || `${ANONYMOUS}${++anonymous}`, validate: undefined };
    if (register && id && !id.startsWith('#') && this.opts.addUsedSchema !== false) this._checkUnique(id);
    this._cache.set(schema, entry);
    try {
      this._index(schema, entry.baseId);
      this._check(schema, entry.baseId);
    } catch (error) {
      this._cache.delete(schema);
      throw error;
    }
    if (register && id && !id.startsWith('#') && this.opts.addUsedSchema !== false) this.refs[id] = entry;
    entry.validate = this._validator(entry);
    return entry;
  }

  // Records the documents and anchors a schema defines, by URL.
  _index(schema, base) {
    if (!this._documents.has(base)) this._documents.set(base, { schema, base });
    walk(schema, '#', base, (node, path, nodeBase) => {
      if (typeof node.$id === 'string') {
        if (node.$id.startsWith('#')) {
          this._documents.set(resolveUrl(nodeBase, node.$id), { schema: node, base: nodeBase, path });
        } else if (!this._documents.has(nodeBase)) {
          this._documents.set(nodeBase, { schema: node, base: nodeBase, path });
        }
      }
      if (typeof node.$anchor === 'string') {
        this._documents.set(resolveUrl(nodeBase, '#' + node.$anchor), { schema: node, base: nodeBase, path });
      }
    });
  }

  // Ajv's compile-time checks: strict mode, formats and references.
  _check(root, base) {
    const strict = this.opts.strictSchema;
    walk(root, '#', base, (schema, path, nodeBase) => {
      if (schema.$schema !== undefined && !this._metas.has(normalizeId(schema.$schema))) {
        throw new Error(`no schema with key or ref "${schema.$schema}"`);
      }
      schemaTypes(schema);
      for (const keyword in schema) {
        if (UNSUPPORTED.includes(keyword)) {
          throw new Error(`The JSON Schema keyword "${keyword}" is not supported in compiled applications`);
        }
        if (!this._known.has(keyword) && strict) {
          const message = `strict mode: unknown keyword: "${keyword}"`;
          if (strict === true) throw new Error(message);
          if (strict === 'log') this.logger.warn(message);
        }
      }
      if (typeof schema.pattern === 'string') regExp({ ajv: this }, schema.pattern);
      if (schema.patternProperties && typeof schema.patternProperties === 'object') {
        for (const pattern of own(schema.patternProperties)) regExp({ ajv: this }, pattern);
      }
      if (typeof schema.format === 'string' && this.opts.validateFormats !== false
          && this.formats[schema.format] === undefined) {
        const message = `unknown format "${schema.format}" ignored in schema at path "${path}"`;
        if (strict === false) this.logger.warn(message);
        else throw new Error(message);
      }
      if (typeof schema.$ref === 'string') this._resolve(nodeBase, schema.$ref);
      for (const keyword in schema) {
        const definition = this._rules.get(keyword);
        if (definition && typeof definition === 'object') {
          if (definition.macro) this._macro(definition, keyword, schema[keyword], schema);
          else if (definition.compile) this._compiledKeyword(definition, keyword, schema[keyword], schema);
        }
      }
    });
  }

  // The schema a reference names, with the base URL and error path its
  // keywords report under.
  _resolve(base, reference) {
    const url = resolveUrl(base, reference);
    const hash = url.indexOf('#');
    const documentUrl = hash < 0 ? url : url.slice(0, hash);
    const fragment = hash < 0 ? '' : url.slice(hash + 1);
    const named = this._documents.get(url);
    if (named && !fragment.startsWith('/')) {
      return { schema: named.schema, base: named.base, schemaPath: reference };
    }
    let document = this._documents.get(documentUrl);
    if (!document) {
      const added = this.schemas[documentUrl] || this.refs[documentUrl];
      if (added && typeof added === 'object') document = { schema: added.schema, base: added.baseId };
    }
    if (!document) throw new MissingRefError(this, base, reference);
    let schema = document.schema;
    let schemaBase = document.base;
    if (fragment) {
      for (const part of fragment.slice(1).split('/')) {
        const key = unescapePointer(decodeURIComponent(part));
        if (!schema || typeof schema !== 'object' || !(key in schema)) {
          throw new MissingRefError(this, base, reference);
        }
        schema = schema[key];
        if (schema && typeof schema === 'object' && typeof schema.$id === 'string' && !schema.$id.startsWith('#')) {
          schemaBase = resolveUrl(schemaBase, schema.$id);
        }
      }
    }
    // Ajv reports a referenced schema's errors under the reference itself.
    return { schema, base: schemaBase, schemaPath: reference };
  }

  // What a user keyword's `macro` or `compile` function returns for one
  // schema, made once.
  _keywordResult(kind, definition, keyword, value, schema) {
    let bySchema = this._keywordCache.get(schema);
    if (!bySchema) {
      bySchema = new Map();
      this._keywordCache.set(schema, bySchema);
    }
    if (!bySchema.has(keyword)) {
      const it = { self: this, schema, opts: this.opts, keyword };
      bySchema.set(keyword, definition[kind].call(this, value, schema, it));
    }
    return bySchema.get(keyword);
  }

  _macro(definition, keyword, value, schema) {
    return this._keywordResult('macro', definition, keyword, value, schema);
  }

  _compiledKeyword(definition, keyword, value, schema) {
    return this._keywordResult('compile', definition, keyword, value, schema);
  }

  // The validation function for a compiled schema.
  _validator(entry) {
    const ajv = this;
    const asynchronous = entry.schema && typeof entry.schema === 'object' && entry.schema.$async === true;
    function validate(data, dataContext = {}) {
      const scope = {
        ajv,
        errors: [],
        rootData: dataContext.rootData === undefined ? data : dataContext.rootData,
        context: this,
      };
      const valid = check(scope, entry.schema, data, {
        path: dataContext.instancePath || '',
        schemaPath: '#',
        parent: dataContext.parentData,
        key: dataContext.parentDataProperty,
        composite: false,
        all: Boolean(ajv.opts.allErrors),
        base: entry.baseId,
        propertyName: undefined,
      });
      validate.errors = valid ? null : scope.errors;
      if (asynchronous) return valid ? Promise.resolve(data) : Promise.reject(new ValidationError(scope.errors));
      return valid;
    }
    validate.errors = null;
    validate.schema = entry.schema;
    validate.schemaEnv = entry;
    if (asynchronous) validate.$async = true;
    return validate;
  }

  // The structural errors Ajv's meta-schema reports most often.
  _schemaErrors(root) {
    const errors = [];
    const report = (path, keyword, params, message) => errors.push({
      instancePath: path, schemaPath: `#/${keyword}`, keyword, params, message,
    });
    const visit = (schema, path) => {
      if (typeof schema === 'boolean') return;
      if (!schema || typeof schema !== 'object' || Array.isArray(schema)) {
        report(path, 'type', { type: ['object', 'boolean'] }, 'must be object,boolean');
        return;
      }
      if (schema.type !== undefined) {
        const types = Array.isArray(schema.type) ? schema.type : [schema.type];
        if (!types.every((type) => JSON_TYPES.includes(type))) {
          report(`${path}/type`, 'enum', { allowedValues: JSON_TYPES }, 'must be equal to one of the allowed values');
          if (!Array.isArray(schema.type)) report(`${path}/type`, 'type', { type: 'array' }, 'must be array');
          report(`${path}/type`, 'anyOf', {}, 'must match a schema in anyOf');
        }
      }
      if (schema.required !== undefined && !(Array.isArray(schema.required) && schema.required.every((item) => typeof item === 'string'))) {
        report(`${path}/required`, 'type', { type: 'array' }, 'must be array');
      }
      if (schema.enum !== undefined && !Array.isArray(schema.enum)) {
        report(`${path}/enum`, 'type', { type: 'array' }, 'must be array');
      }
      for (const keyword of ['maximum', 'minimum', 'exclusiveMaximum', 'exclusiveMinimum', 'multipleOf']) {
        if (schema[keyword] !== undefined && typeof schema[keyword] !== 'number') {
          report(`${path}/${keyword}`, 'type', { type: 'number' }, 'must be number');
        }
      }
      for (const keyword of ['maxLength', 'minLength', 'maxItems', 'minItems', 'maxProperties', 'minProperties']) {
        const value = schema[keyword];
        if (value !== undefined && !(Number.isInteger(value) && value >= 0)) {
          report(`${path}/${keyword}`, 'type', { type: 'integer' }, 'must be integer');
        }
      }
      if (schema.pattern !== undefined && typeof schema.pattern !== 'string') {
        report(`${path}/pattern`, 'type', { type: 'string' }, 'must be string');
      }
      for (const keyword of ['allOf', 'anyOf', 'oneOf']) {
        const list = schema[keyword];
        if (list === undefined) continue;
        if (!Array.isArray(list)) report(`${path}/${keyword}`, 'type', { type: 'array' }, 'must be array');
        else list.forEach((item, i) => visit(item, `${path}/${keyword}/${i}`));
      }
      for (const keyword of ['properties', 'patternProperties', '$defs', 'definitions', 'dependentSchemas']) {
        const map = schema[keyword];
        if (map === undefined) continue;
        if (!map || typeof map !== 'object' || Array.isArray(map)) {
          report(`${path}/${keyword}`, 'type', { type: 'object' }, 'must be object');
        } else {
          for (const key of own(map)) visit(map[key], `${path}/${keyword}/${escapePointer(key)}`);
        }
      }
      for (const keyword of SINGLE) {
        if (schema[keyword] !== undefined) visit(schema[keyword], `${path}/${keyword}`);
      }
      if (Array.isArray(schema.items)) schema.items.forEach((item, i) => visit(item, `${path}/items/${i}`));
      else if (schema.items !== undefined) visit(schema.items, `${path}/items`);
    };
    visit(root, '');
    return errors;
  }
}

class Ajv2019 extends Ajv {
  get _dialect() {
    return DIALECTS[2019];
  }
}

class Ajv2020 extends Ajv {
  get _dialect() {
    return DIALECTS[2020];
  }
}

module.exports = { Ajv, Ajv2019, Ajv2020, ValidationError, MissingRefError };
