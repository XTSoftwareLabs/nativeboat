import query, { parse, stringify, escape as encode } from 'querystring';
import nodeQuery from 'node:querystring';
import * as namespace from 'node:querystring';
console.log(query === nodeQuery, parse === query.parse, stringify === query.stringify,
  namespace.default === query, JSON.stringify(parse('a=one+two&a=3')), encode('a b'));
