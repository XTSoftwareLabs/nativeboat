'use strict';

// Node's `X509Certificate` for PEM and DER certificates, read with a DER
// parser: names and alternative names as OpenSSL prints them, validity,
// fingerprints, serial number, CA status and extended key usage. Checks
// against hosts, emails, IPs and issuers, verification and public keys
// need public key cryptography and are not available.

const { Buffer } = require('buffer');
const {
  codes: { ERR_FEATURE_UNAVAILABLE_ON_PLATFORM, ERR_INVALID_ARG_TYPE },
} = require('internal/errors');

const binding = internalBinding('crypto');
const kParsed = Symbol('kParsed');

function codedError(message, code) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function malformed() {
  return codedError('error:0680007B:asn1 encoding routines::header too long', 'ERR_OSSL_ASN1_HEADER_TOO_LONG');
}

// One DER element: its tag, its contents and where it ends.
function element(bytes, offset) {
  if (offset + 2 > bytes.length) throw malformed();
  const tag = bytes[offset];
  let length = bytes[offset + 1];
  let start = offset + 2;
  if (length & 0x80) {
    const count = length & 0x7f;
    if (count === 0 || count > 4 || start + count > bytes.length) throw malformed();
    length = 0;
    for (let i = 0; i < count; i++) length = length * 256 + bytes[start + i];
    start += count;
  }
  const end = start + length;
  if (end > bytes.length) throw malformed();
  return { tag, start, end, bytes };
}

function children(parent) {
  const result = [];
  for (let offset = parent.start; offset < parent.end;) {
    const child = element(parent.bytes, offset);
    result.push(child);
    offset = child.end;
  }
  return result;
}

const contents = (node) => node.bytes.subarray(node.start, node.end);

function oid(node) {
  const bytes = contents(node);
  const parts = [Math.floor(bytes[0] / 40), bytes[0] % 40];
  let value = 0;
  for (let i = 1; i < bytes.length; i++) {
    value = value * 128 + (bytes[i] & 0x7f);
    if (!(bytes[i] & 0x80)) {
      parts.push(value);
      value = 0;
    }
  }
  return parts.join('.');
}

// A string value of a name attribute, decoded by its ASN.1 type.
function text(node) {
  const bytes = contents(node);
  if (node.tag === 0x1e) {
    let result = '';
    for (let i = 0; i + 1 < bytes.length; i += 2) result += String.fromCharCode(bytes[i] * 256 + bytes[i + 1]);
    return result;
  }
  if (node.tag === 0x0c) return Buffer.from(bytes).toString('utf8');
  return Buffer.from(bytes).toString('latin1');
}

const attributeNames = {
  '2.5.4.3': 'CN',
  '2.5.4.4': 'SN',
  '2.5.4.5': 'serialNumber',
  '2.5.4.6': 'C',
  '2.5.4.7': 'L',
  '2.5.4.8': 'ST',
  '2.5.4.9': 'street',
  '2.5.4.10': 'O',
  '2.5.4.11': 'OU',
  '2.5.4.12': 'title',
  '2.5.4.17': 'postalCode',
  '2.5.4.42': 'GN',
  '2.5.4.43': 'initials',
  '2.5.4.46': 'dnQualifier',
  '2.5.4.65': 'pseudonym',
  '2.5.4.97': 'organizationIdentifier',
  '0.9.2342.19200300.100.1.1': 'UID',
  '0.9.2342.19200300.100.1.25': 'DC',
  '1.2.840.113549.1.9.1': 'emailAddress',
  '1.3.6.1.4.1.311.60.2.1.1': 'jurisdictionL',
  '1.3.6.1.4.1.311.60.2.1.2': 'jurisdictionST',
  '1.3.6.1.4.1.311.60.2.1.3': 'jurisdictionC',
  '2.5.4.15': 'businessCategory',
};

// RFC 2253 escaping, as OpenSSL's multiline name printing applies it.
function escape(value) {
  let result = '';
  for (let i = 0; i < value.length; i++) {
    const char = value[i];
    const code = char.charCodeAt(0);
    if (',+"\\<>;'.includes(char) || (i === 0 && (char === '#' || char === ' ')) ||
        (i === value.length - 1 && char === ' ')) {
      result += `\\${char}`;
    } else if (code < 0x20 || code === 0x7f) {
      result += `\\${code.toString(16).toUpperCase().padStart(2, '0')}`;
    } else {
      result += char;
    }
  }
  return result;
}

function name(node) {
  const lines = [];
  for (const set of children(node)) {
    for (const attribute of children(set)) {
      const [type, value] = children(attribute);
      const id = oid(type);
      lines.push(`${attributeNames[id] ?? id}=${escape(text(value))}`);
    }
  }
  return lines.join('\n');
}

const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function time(node) {
  const value = Buffer.from(contents(node)).toString('latin1');
  const utc = node.tag === 0x17;
  let year = Number(value.slice(0, utc ? 2 : 4));
  if (utc) year += year < 50 ? 2000 : 1900;
  const rest = value.slice(utc ? 2 : 4);
  const [month, day, hour, minute, second] = [0, 2, 4, 6, 8].map((at) => Number(rest.slice(at, at + 2)));
  const date = new Date(Date.UTC(year, month - 1, day, hour, minute, second || 0));
  const pad = (n) => `${n}`.padStart(2, '0');
  const printed = `${months[month - 1]} ${`${day}`.padStart(2, ' ')} ${pad(hour)}:${pad(minute)}:${pad(second || 0)} ${year} GMT`;
  return { date, printed };
}

function alternativeNames(value) {
  const entries = [];
  for (const general of children(element(value, 0))) {
    const bytes = contents(general);
    switch (general.tag) {
      case 0x81: entries.push(`email:${Buffer.from(bytes).toString('latin1')}`); break;
      case 0x82: entries.push(`DNS:${Buffer.from(bytes).toString('latin1')}`); break;
      case 0x86: entries.push(`URI:${Buffer.from(bytes).toString('latin1')}`); break;
      case 0x87:
        if (bytes.length === 4) {
          entries.push(`IP Address:${[...bytes].join('.')}`);
        } else {
          const groups = [];
          for (let i = 0; i + 1 < bytes.length; i += 2) groups.push(((bytes[i] << 8) | bytes[i + 1]).toString(16).toUpperCase());
          entries.push(`IP Address:${groups.join(':')}`);
        }
        break;
      case 0x88: entries.push(`Registered ID:${oid({ ...general, tag: 6 })}`); break;
      default: entries.push('othername:<unsupported>');
    }
  }
  return entries.join(', ');
}

function parse(der) {
  const certificate = element(der, 0);
  if (certificate.tag !== 0x30 || certificate.end !== der.length) throw malformed();
  const [tbs] = children(certificate);
  const fields = children(tbs);
  let index = fields[0].tag === 0xa0 ? 1 : 0;
  const serial = contents(fields[index++]);
  index++; // signature algorithm
  const issuer = name(fields[index++]);
  const [notBefore, notAfter] = children(fields[index++]);
  const subject = name(fields[index++]);
  const parsed = {
    raw: der,
    serial,
    issuer,
    subject,
    validFrom: time(notBefore),
    validTo: time(notAfter),
    subjectAltName: undefined,
    ca: false,
    keyUsage: undefined,
  };
  let keyUsageBits;
  for (const field of fields.slice(index)) {
    if (field.tag !== 0xa3) continue;
    for (const extension of children(children(field)[0])) {
      const parts = children(extension);
      const id = oid(parts[0]);
      const value = contents(parts[parts.length - 1]);
      if (id === '2.5.29.17') {
        parsed.subjectAltName = alternativeNames(value);
      } else if (id === '2.5.29.19') {
        const constraints = children(element(value, 0));
        parsed.ca = constraints.length > 0 && constraints[0].tag === 0x01 && contents(constraints[0])[0] !== 0;
      } else if (id === '2.5.29.37') {
        parsed.keyUsage = children(element(value, 0)).map(oid);
      } else if (id === '2.5.29.15') {
        const bits = contents(element(value, 0));
        keyUsageBits = bits.length > 1 ? bits[1] : 0;
      }
    }
  }
  // A certificate whose key usage leaves out certificate signing is not a
  // CA, as OpenSSL's X509_check_ca decides.
  if (keyUsageBits !== undefined && !(keyUsageBits & 0x04)) parsed.ca = false;
  return parsed;
}

const PEM = /-----BEGIN CERTIFICATE-----([\s\S]*?)-----END CERTIFICATE-----/;

function fingerprint(der, digest) {
  const hex = binding.Hash(digest).update(der).digest().toString('hex').toUpperCase();
  return hex.match(/../g).join(':');
}

class X509Certificate {
  constructor(buffer) {
    if (typeof buffer === 'string') buffer = Buffer.from(buffer);
    if (!ArrayBuffer.isView(buffer)) {
      throw new ERR_INVALID_ARG_TYPE('buffer', ['string', 'Buffer', 'TypedArray', 'DataView'], buffer);
    }
    const bytes = Buffer.from(buffer.buffer, buffer.byteOffset, buffer.byteLength);
    let der;
    if (bytes.length > 0 && bytes[0] === 0x30) {
      der = Buffer.from(bytes);
    } else {
      const match = PEM.exec(bytes.toString('latin1'));
      if (!match) throw codedError('error:0480006C:PEM routines::no start line', 'ERR_OSSL_PEM_NO_START_LINE');
      der = Buffer.from(match[1].replace(/\s+/g, ''), 'base64');
    }
    this[kParsed] = parse(der);
  }

  get subject() { return this[kParsed].subject; }
  get issuer() { return this[kParsed].issuer; }
  get subjectAltName() { return this[kParsed].subjectAltName; }
  get infoAccess() { return undefined; }
  get validFrom() { return this[kParsed].validFrom.printed; }
  get validTo() { return this[kParsed].validTo.printed; }
  get validFromDate() { return new Date(this[kParsed].validFrom.date); }
  get validToDate() { return new Date(this[kParsed].validTo.date); }
  get fingerprint() { return fingerprint(this[kParsed].raw, 'sha1'); }
  get fingerprint256() { return fingerprint(this[kParsed].raw, 'sha256'); }
  get fingerprint512() { return fingerprint(this[kParsed].raw, 'sha512'); }
  get keyUsage() { return this[kParsed].keyUsage; }
  get ca() { return this[kParsed].ca; }
  get raw() { return Buffer.from(this[kParsed].raw); }

  get serialNumber() {
    const serial = this[kParsed].serial;
    let start = 0;
    while (start < serial.length - 1 && serial[start] === 0) start++;
    const hex = Buffer.from(serial.subarray(start)).toString('hex').toUpperCase();
    return hex === '00' ? '0' : hex;
  }

  get publicKey() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.publicKey');
  }

  checkHost() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.checkHost');
  }

  checkEmail() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.checkEmail');
  }

  checkIP() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.checkIP');
  }

  checkIssued() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.checkIssued');
  }

  checkPrivateKey() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.checkPrivateKey');
  }

  verify() {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('X509Certificate.verify');
  }

  toString() {
    const base64 = this[kParsed].raw.toString('base64');
    return `-----BEGIN CERTIFICATE-----\n${base64.match(/.{1,64}/g).join('\n')}\n-----END CERTIFICATE-----\n`;
  }

  toJSON() {
    return this.toString();
  }

  toLegacyObject() {
    return {
      subject: this.subject,
      issuer: this.issuer,
      subjectaltname: this.subjectAltName,
      valid_from: this.validFrom,
      valid_to: this.validTo,
      fingerprint: this.fingerprint,
      fingerprint256: this.fingerprint256,
      fingerprint512: this.fingerprint512,
      ext_key_usage: this.keyUsage,
      serialNumber: this.serialNumber,
      raw: this.raw,
    };
  }
}

module.exports = { X509Certificate };
