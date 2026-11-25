// Validates settings as conf (electron-store) does: Ajv's 2020 class and
// ajv-formats, imported from CommonJS packages.
import { Ajv2020 as Ajv } from 'ajv/dist/2020.js';
import ajvFormatsModule from 'ajv-formats';
import Draft7 from 'ajv';

const ajv = new Ajv({ allErrors: true, useDefaults: true });
ajvFormatsModule.default(ajv);
const validate = ajv.compile({
  type: 'object',
  properties: {
    theme: { type: 'string', enum: ['light', 'dark'], default: 'light' },
    fontSize: { type: 'number', minimum: 12, maximum: 32, default: 16 },
    folder: { type: 'string', pattern: '^[a-z]+$' },
    since: { type: 'string', format: 'date', formatMinimum: '2020-01-01' },
    tags: { type: 'array', items: { type: 'string' }, uniqueItems: true },
  },
});
const settings = { folder: 'notes' };
console.log(validate(settings), JSON.stringify(settings));
console.log(validate({ theme: 'blue', fontSize: 40, folder: 'A', since: '2019-05-01', tags: ['a', 1, 'a'] }));
for (const { instancePath, message } of validate.errors) console.log(`\`${instancePath.slice(1)}\` ${message}`);
console.log(ajv.errorsText(validate.errors));

const draft7 = new Draft7();
const tuple = draft7.compile({ items: [{ type: 'string' }], additionalItems: false });
console.log(tuple(['a']), tuple(['a', 'b']), JSON.stringify(tuple.errors));
try {
  draft7.compile({ type: 'object', unknownKeyword: true });
} catch (error) {
  console.log(error.message);
}
