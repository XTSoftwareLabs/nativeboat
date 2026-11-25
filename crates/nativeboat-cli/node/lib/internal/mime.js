'use strict';

const tokenPattern = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

class MIMEParams {
  constructor(serialized) {
    this._entries = [];
    if (serialized) {
      for (const raw of String(serialized).split(';')) {
        const part = raw.trim();
        if (!part) continue;
        const equals = part.indexOf('=');
        if (equals < 1) continue;
        const name = part.slice(0, equals).trim().toLowerCase();
        let value = part.slice(equals + 1).trim();
        if (value.length >= 2 && value[0] === '"' && value[value.length - 1] === '"') {
          value = value.slice(1, -1).replace(/\\(.)/g, '$1');
        }
        if (tokenPattern.test(name)) this.append(name, value);
      }
    }
  }

  append(name, value) {
    this._entries.push([String(name).toLowerCase(), String(value)]);
  }

  delete(name) {
    name = String(name).toLowerCase();
    this._entries = this._entries.filter(entry => entry[0] !== name);
  }

  get(name) {
    name = String(name).toLowerCase();
    const entry = this._entries.find(item => item[0] === name);
    return entry ? entry[1] : null;
  }

  has(name) {
    name = String(name).toLowerCase();
    return this._entries.some(entry => entry[0] === name);
  }

  set(name, value) {
    name = String(name).toLowerCase();
    value = String(value);
    const index = this._entries.findIndex(entry => entry[0] === name);
    if (index < 0) this.append(name, value);
    else {
      this._entries[index][1] = value;
      this._entries = this._entries.filter((entry, offset) => entry[0] !== name || offset === index);
    }
  }

  entries() {
    return this._entries.map(entry => entry.slice())[Symbol.iterator]();
  }

  keys() {
    return this._entries.map(entry => entry[0])[Symbol.iterator]();
  }

  values() {
    return this._entries.map(entry => entry[1])[Symbol.iterator]();
  }

  forEach(callback, thisArg) {
    for (const [name, value] of this._entries) callback.call(thisArg, value, name, this);
  }

  [Symbol.iterator]() {
    return this.entries();
  }

  toString() {
    return this._entries.map(([name, value]) => ` ${name}=${value}`).join(';');
  }
}

class MIMEType {
  constructor(input) {
    const text = String(input);
    const [essence, ...parameters] = text.split(';');
    const slash = essence.indexOf('/');
    const type = slash < 0 ? '' : essence.slice(0, slash).trim();
    const subtype = slash < 0 ? '' : essence.slice(slash + 1).trim();
    if (!tokenPattern.test(type) || !tokenPattern.test(subtype)) {
      throw new TypeError(`Invalid MIME type: ${text}`);
    }
    this._type = type.toLowerCase();
    this._subtype = subtype.toLowerCase();
    this._params = new MIMEParams(parameters.join(';'));
  }

  get type() { return this._type; }
  set type(value) {
    value = String(value);
    if (!tokenPattern.test(value)) throw new TypeError(`Invalid MIME type: ${value}`);
    this._type = value.toLowerCase();
  }

  get subtype() { return this._subtype; }
  set subtype(value) {
    value = String(value);
    if (!tokenPattern.test(value)) throw new TypeError(`Invalid MIME subtype: ${value}`);
    this._subtype = value.toLowerCase();
  }

  get essence() { return `${this._type}/${this._subtype}`; }
  get params() { return this._params; }

  toString() {
    return this.essence + this._params.toString();
  }

  toJSON() {
    return this.toString();
  }
}

module.exports = { MIMEParams, MIMEType };
