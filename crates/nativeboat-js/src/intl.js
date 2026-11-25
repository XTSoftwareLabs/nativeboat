// Intl: locale-sensitive formatting of dates, numbers, lists, plurals and
// relative times, and comparison and segmentation of text, as Chromium's ICU
// provides them. American English follows CLDR; other locales take their
// names, patterns and separators from Windows (host.localeData). Evaluates to
// a function of the host object:
//
//   host.locale()              the default locale: the application's choice
//                              (Chromium's --lang) or the Windows user's
//   host.localeData(tag)       Windows' data for a locale, or null
//   host.timeZone()            the Windows key of the local time zone
//   host.zoneOffset(key, ms)   minutes east of UTC in a Windows time zone
(function (host) {
  'use strict';
  const global = globalThis;
  const define = (target, name, value) =>
    Object.defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });
  const defineAll = (target, members) => {
    for (const [name, value] of Object.entries(members)) define(target, name, value);
  };
  const tag = (prototype, name) =>
    Object.defineProperty(prototype, Symbol.toStringTag, { value: name, configurable: true });

  // Options. -----------------------------------------------------------------

  const toOptions = (options) => {
    if (options === undefined) return Object.create(null);
    if (options === null) throw new TypeError('Cannot convert undefined or null to object');
    return Object(options);
  };
  const getOption = (options, name, values, fallback) => {
    const value = options[name];
    if (value === undefined) return fallback;
    const text = String(value);
    if (values && !values.includes(text)) throw new RangeError(`Value ${text} out of range for Intl options property ${name}`);
    return text;
  };
  const getBoolean = (options, name, fallback) => (options[name] === undefined ? fallback : Boolean(options[name]));
  const getNumber = (options, name, minimum, maximum, fallback) => {
    const value = options[name];
    if (value === undefined) return fallback;
    const number = Number(value);
    if (!Number.isFinite(number) || number < minimum || number > maximum) {
      throw new RangeError(`${name} value is out of range.`);
    }
    return Math.floor(number);
  };

  // Locales. -----------------------------------------------------------------

  const LANGUAGE_TAG = /^([a-z]{2,3}|[a-z]{5,8})(?:-([a-z]{4}))?(?:-([a-z]{2}|\d{3}))?((?:-(?:[a-z\d]{5,8}|\d[a-z\d]{3}))*)((?:-[a-wyz\d](?:-[a-z\d]{2,8})+)*)(?:-x(?:-[a-z\d]{1,8})+)?$/i;

  const canonicalTag = (value) => {
    if (typeof value !== 'string' && !(value instanceof Locale)) {
      throw new TypeError('Language ID should be string or object.');
    }
    const text = String(value).replace(/_/g, '-');
    const match = LANGUAGE_TAG.exec(text);
    if (!match) throw new RangeError(`Incorrect locale information provided`);
    const [, language, script, region, variants, extensions] = match;
    let result = language.toLowerCase();
    if (script) result += `-${script[0].toUpperCase()}${script.slice(1).toLowerCase()}`;
    if (region) result += `-${region.toUpperCase()}`;
    return result + variants.toLowerCase() + extensions.toLowerCase();
  };

  const canonicalLocales = (locales) => {
    if (locales === undefined) return [];
    const list = typeof locales === 'string' || locales instanceof Locale ? [locales] : Array.from(Object(locales));
    const result = [];
    for (const item of list) {
      const canonical = canonicalTag(item);
      if (!result.includes(canonical)) result.push(canonical);
    }
    return result;
  };

  // The base tag and the Unicode extension keywords, such as hc and nu.
  const splitTag = (canonical) => {
    const index = canonical.indexOf('-u-');
    if (index < 0) return { base: canonical, keywords: {} };
    const keywords = {};
    let key = null;
    for (const part of canonical.slice(index + 3).split('-')) {
      if (part.length === 1) break;
      if (part.length === 2) {
        key = part;
        keywords[key] = 'true';
      } else if (key) keywords[key] = keywords[key] === 'true' ? part : `${keywords[key]}-${part}`;
    }
    return { base: canonical.slice(0, index), keywords };
  };

  const defaultLocale = () => {
    try {
      return splitTag(canonicalTag(String(host.locale()))).base;
    } catch {
      return 'en-US';
    }
  };

  // Locale data. ---------------------------------------------------------------

  const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
  const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  const ENGLISH = {
    english: true,
    months: MONTHS,
    monthsGenitive: MONTHS,
    monthsShort: MONTHS.map((month) => month.slice(0, 3)),
    weekdays: WEEKDAYS,
    weekdaysShort: WEEKDAYS.map((day) => day.slice(0, 3)),
    weekdaysNarrow: WEEKDAYS.map((day) => day[0]),
    am: 'AM',
    pm: 'PM',
    decimal: '.',
    group: ',',
    grouping: [3],
    minus: '-',
    percent: '%',
    hourCycle: 'h12',
  };

  const groupingOf = (text) => {
    const sizes = String(text || '3;0').split(';').map(Number).filter((size) => size > 0);
    return sizes.length ? sizes : [3];
  };

  const fromWindows = (windows) => {
    const time = windows.time || 'HH:mm:ss';
    return {
      english: false,
      months: windows.months,
      monthsGenitive: windows.monthsGenitive.map((name, index) => name || windows.months[index]),
      monthsShort: windows.monthsShort,
      weekdays: windows.weekdays,
      weekdaysShort: windows.weekdaysShort,
      weekdaysNarrow: windows.weekdaysNarrow,
      am: windows.am || 'AM',
      pm: windows.pm || 'PM',
      decimal: windows.decimal || '.',
      group: windows.group === undefined ? ',' : windows.group,
      grouping: groupingOf(windows.grouping),
      minus: windows.minus || '-',
      percent: windows.percent || '%',
      percentPattern: windows.percentPattern,
      negativePercentPattern: windows.negativePercentPattern,
      currency: windows.currency,
      currencySymbol: windows.currencySymbol,
      currencyPattern: windows.currencyPattern,
      shortDate: windows.shortDate,
      longDate: windows.longDate,
      yearMonth: windows.yearMonth,
      monthDay: windows.monthDay,
      time,
      shortTime: windows.shortTime || time.replace(/[:.]?s+/, ''),
      hourCycle: /H/.test(time) ? 'h23' : 'h12',
    };
  };

  // French groups digits with a narrow no-break space in CLDR, as Chromium
  // shows them, where older Windows versions use a no-break space; Canadian
  // French keeps the wider one.
  const cldrGroup = (base, group) => (group === ' ' && /^fr(-|$)/.test(base) && !/^fr-CA(-|$)/.test(base) ? ' ' : group);

  const localeCache = new Map();
  // The data of the first requested locale Windows knows, else of the
  // default locale; American English is built in.
  const localeData = (base) => {
    if (localeCache.has(base)) return localeCache.get(base);
    let data = null;
    if (base === 'en' || base === 'en-US') data = { ...ENGLISH, locale: base };
    else {
      const windows = typeof host.localeData === 'function' ? host.localeData(base) : null;
      if (windows) {
        data = { ...fromWindows(windows), locale: base };
        data.group = cldrGroup(base, data.group);
      }
    }
    localeCache.set(base, data);
    return data;
  };

  const resolveLocale = (locales) => {
    for (const requested of canonicalLocales(locales)) {
      const { base, keywords } = splitTag(requested);
      const data = localeData(base) || localeData(base.split('-')[0]);
      if (data) return { locale: data.locale, keywords, data };
    }
    const base = defaultLocale();
    const data = localeData(base) || localeData('en-US');
    return { locale: data.locale, keywords: {}, data };
  };

  const language = (locale) => locale.split('-')[0];

  // Numbers. -------------------------------------------------------------------

  // The decimal digits of a non-negative finite number as a string, with the
  // number of integer digits: the shortest representation, as ICU rounds it.
  const decimalDigits = (value) => {
    if (typeof value === 'bigint') {
      const text = String(value < 0n ? -value : value);
      return { digits: text, point: text.length };
    }
    let text = String(value);
    let exponent = 0;
    const e = text.indexOf('e');
    if (e >= 0) {
      exponent = Number(text.slice(e + 1));
      text = text.slice(0, e);
    }
    const dot = text.indexOf('.');
    const integer = dot < 0 ? text : text.slice(0, dot);
    const fraction = dot < 0 ? '' : text.slice(dot + 1);
    let digits = integer + fraction;
    let point = integer.length + exponent;
    const leading = digits.match(/^0*/)[0].length;
    digits = digits.slice(leading);
    point -= leading;
    digits = digits.replace(/0+$/, '');
    if (!digits) return { digits: '', point: 1 };
    return { digits, point };
  };

  // Keeps `count` digits, rounding half away from zero.
  const roundDigits = ({ digits, point }, count) => {
    if (count >= digits.length) return { digits, point };
    if (count < 0) return { digits: '', point: 1 };
    let kept = digits.slice(0, count).split('').map(Number);
    if (Number(digits[count]) >= 5) {
      let index = kept.length - 1;
      while (index >= 0 && kept[index] === 9) {
        kept[index] = 0;
        index--;
      }
      if (index < 0) {
        kept.unshift(1);
        point++;
      } else kept[index]++;
    }
    const text = kept.join('').replace(/0+$/, '');
    return text ? { digits: text, point } : { digits: '', point: 1 };
  };

  const roundNumber = (number, rounding) => {
    if (rounding.significant) {
      const rounded = roundDigits(number, rounding.maximumSignificantDigits);
      return { ...rounded, minimumSignificant: rounding.minimumSignificantDigits };
    }
    const rounded = roundDigits(number, number.point + rounding.maximumFractionDigits);
    return { ...rounded, minimumFraction: rounding.minimumFractionDigits };
  };

  // Integer and fraction digit strings of a rounded number.
  const numberStrings = (rounded, minimumIntegerDigits) => {
    const { digits, point } = rounded;
    let integer;
    let fraction;
    if (point <= 0) {
      integer = '0';
      fraction = '0'.repeat(-point) + digits;
    } else {
      integer = digits.slice(0, point).padEnd(point, '0');
      fraction = digits.slice(point);
    }
    integer = integer.replace(/^0+(?=\d)/, '');
    if (!digits) {
      integer = '0';
      fraction = '';
    }
    if (rounded.minimumFraction !== undefined) fraction = fraction.padEnd(rounded.minimumFraction, '0');
    if (rounded.minimumSignificant !== undefined) {
      const significant = (integer === '0' ? '' : integer).length + (integer === '0' ? fraction.replace(/^0*/, '') : fraction).length;
      if (significant < rounded.minimumSignificant) fraction += '0'.repeat(rounded.minimumSignificant - Math.max(significant, digits ? 0 : 1));
    }
    integer = integer.padStart(minimumIntegerDigits, '0');
    return { integer, fraction };
  };

  const groupInteger = (integer, data, useGrouping) => {
    if (useGrouping === false || !data.group) return [integer];
    const [primary, secondary = primary] = data.grouping;
    if (useGrouping === 'min2' && integer.length < primary + 2) return [integer];
    if (integer.length <= primary) return [integer];
    const groups = [integer.slice(-primary)];
    let rest = integer.slice(0, -primary);
    while (rest.length > secondary) {
      groups.unshift(rest.slice(-secondary));
      rest = rest.slice(0, -secondary);
    }
    groups.unshift(rest);
    return groups;
  };

  const CURRENCIES = {
    USD: ['$', '$', 'US dollar', 'US dollars'], EUR: ['€', '€', 'euro', 'euros'], GBP: ['£', '£', 'British pound', 'British pounds'],
    JPY: ['¥', '¥', 'Japanese yen', 'Japanese yen'], CNY: ['CN¥', '¥', 'Chinese yuan', 'Chinese yuan'], INR: ['₹', '₹', 'Indian rupee', 'Indian rupees'],
    CAD: ['CA$', '$', 'Canadian dollar', 'Canadian dollars'], AUD: ['A$', '$', 'Australian dollar', 'Australian dollars'], NZD: ['NZ$', '$', 'New Zealand dollar', 'New Zealand dollars'],
    HKD: ['HK$', '$', 'Hong Kong dollar', 'Hong Kong dollars'], TWD: ['NT$', '$', 'New Taiwan dollar', 'New Taiwan dollars'], SGD: ['SGD', '$', 'Singapore dollar', 'Singapore dollars'],
    MXN: ['MX$', '$', 'Mexican peso', 'Mexican pesos'], BRL: ['R$', 'R$', 'Brazilian real', 'Brazilian reals'], KRW: ['₩', '₩', 'South Korean won', 'South Korean won'],
    CHF: ['CHF', 'CHF', 'Swiss franc', 'Swiss francs'], SEK: ['SEK', 'kr', 'Swedish krona', 'Swedish kronor'], NOK: ['NOK', 'kr', 'Norwegian krone', 'Norwegian kroner'],
    DKK: ['DKK', 'kr', 'Danish krone', 'Danish kroner'], PLN: ['PLN', 'zł', 'Polish zloty', 'Polish zlotys'], RUB: ['RUB', '₽', 'Russian ruble', 'Russian rubles'],
    TRY: ['TRY', '₺', 'Turkish lira', 'Turkish Lira'], ILS: ['₪', '₪', 'Israeli new shekel', 'Israeli new shekels'], ZAR: ['ZAR', 'R', 'South African rand', 'South African rand'],
    VND: ['₫', '₫', 'Vietnamese dong', 'Vietnamese dong'], THB: ['THB', '฿', 'Thai baht', 'Thai baht'], PHP: ['₱', '₱', 'Philippine peso', 'Philippine pesos'],
    BTC: ['BTC', 'BTC', 'BTC', 'BTC'],
  };
  const ZERO_DECIMAL = ['JPY', 'KRW', 'VND', 'CLP', 'ISK', 'UGX', 'PYG', 'XAF', 'XOF', 'IDR'];
  const THREE_DECIMAL = ['BHD', 'IQD', 'JOD', 'KWD', 'LYD', 'OMR', 'TND'];
  const currencyDigits = (code) => (ZERO_DECIMAL.includes(code) ? (code === 'IDR' ? 2 : 0) : THREE_DECIMAL.includes(code) ? 3 : 2);

  const UNITS = {
    // unit: [short singular, short plural, long singular, long plural, narrow]
    acre: ['ac', 'ac', 'acre', 'acres', 'ac'], bit: ['bit', 'bit', 'bit', 'bits', 'bit'], byte: ['byte', 'byte', 'byte', 'bytes', 'B'],
    celsius: ['°C', '°C', 'degree Celsius', 'degrees Celsius', '°C'], centimeter: ['cm', 'cm', 'centimeter', 'centimeters', 'cm'],
    day: ['day', 'days', 'day', 'days', 'd'], degree: ['deg', 'deg', 'degree', 'degrees', '°'], fahrenheit: ['°F', '°F', 'degree Fahrenheit', 'degrees Fahrenheit', '°'],
    'fluid-ounce': ['fl oz', 'fl oz', 'fluid ounce', 'fluid ounces', 'fl oz'], foot: ['ft', 'ft', 'foot', 'feet', '′'], gallon: ['gal', 'gal', 'gallon', 'gallons', 'gal'],
    gigabit: ['Gb', 'Gb', 'gigabit', 'gigabits', 'Gb'], gigabyte: ['GB', 'GB', 'gigabyte', 'gigabytes', 'GB'], gram: ['g', 'g', 'gram', 'grams', 'g'],
    hectare: ['ha', 'ha', 'hectare', 'hectares', 'ha'], hour: ['hr', 'hr', 'hour', 'hours', 'h'], inch: ['in', 'in', 'inch', 'inches', '″'],
    kilobit: ['kb', 'kb', 'kilobit', 'kilobits', 'kb'], kilobyte: ['kB', 'kB', 'kilobyte', 'kilobytes', 'kB'], kilogram: ['kg', 'kg', 'kilogram', 'kilograms', 'kg'],
    kilometer: ['km', 'km', 'kilometer', 'kilometers', 'km'], liter: ['L', 'L', 'liter', 'liters', 'L'], megabit: ['Mb', 'Mb', 'megabit', 'megabits', 'Mb'],
    megabyte: ['MB', 'MB', 'megabyte', 'megabytes', 'MB'], meter: ['m', 'm', 'meter', 'meters', 'm'], microsecond: ['μs', 'μs', 'microsecond', 'microseconds', 'μs'],
    mile: ['mi', 'mi', 'mile', 'miles', 'mi'], 'mile-scandinavian': ['smi', 'smi', 'mile-scandinavian', 'miles-scandinavian', 'smi'], milliliter: ['mL', 'mL', 'milliliter', 'milliliters', 'mL'],
    millimeter: ['mm', 'mm', 'millimeter', 'millimeters', 'mm'], millisecond: ['ms', 'ms', 'millisecond', 'milliseconds', 'ms'], minute: ['min', 'min', 'minute', 'minutes', 'm'],
    month: ['mth', 'mths', 'month', 'months', 'm'], nanosecond: ['ns', 'ns', 'nanosecond', 'nanoseconds', 'ns'], ounce: ['oz', 'oz', 'ounce', 'ounces', 'oz'],
    percent: ['%', '%', 'percent', 'percent', '%'], petabyte: ['PB', 'PB', 'petabyte', 'petabytes', 'PB'], pound: ['lb', 'lb', 'pound', 'pounds', 'lb'],
    second: ['sec', 'sec', 'second', 'seconds', 's'], stone: ['st', 'st', 'stone', 'stones', 'st'], terabit: ['Tb', 'Tb', 'terabit', 'terabits', 'Tb'],
    terabyte: ['TB', 'TB', 'terabyte', 'terabytes', 'TB'], week: ['wk', 'wks', 'week', 'weeks', 'w'], yard: ['yd', 'yd', 'yard', 'yards', 'yd'], year: ['yr', 'yrs', 'year', 'years', 'y'],
  };
  const NARROW_TIGHT = ['percent', 'celsius', 'fahrenheit', 'degree', 'foot', 'inch'];
  const LONG_COMPACT = ['', 'thousand', 'million', 'billion', 'trillion'];
  const SHORT_COMPACT = ['', 'K', 'M', 'B', 'T'];

  // Parts of a formatted number, before any sign, currency or unit.
  function digitParts(nf, number) {
    const { data, options } = nf;
    const parts = [];
    if (options.notation === 'scientific' || options.notation === 'engineering') {
      const exponent = number.digits ? number.point - 1 : 0;
      const shift = options.notation === 'engineering' ? exponent - ((exponent % 3) + 3) % 3 : exponent;
      const mantissa = { digits: number.digits, point: number.point - shift };
      const rounded = roundNumber(mantissa, options.rounding);
      let power = shift;
      if (rounded.point > mantissa.point) {
        power += options.notation === 'engineering' ? (rounded.point - 1 >= 3 ? 3 : 0) : 1;
        rounded.point -= power - shift;
      }
      pushDigits(parts, numberStrings(rounded, options.minimumIntegerDigits), data, options.useGrouping);
      parts.push({ type: 'exponentSeparator', value: 'E' });
      if (power < 0) parts.push({ type: 'exponentMinusSign', value: data.minus });
      parts.push({ type: 'exponentInteger', value: String(Math.abs(power)) });
      return parts;
    }
    if (options.notation === 'compact' && number.digits) {
      let magnitude = Math.min(Math.floor((number.point - 1) / 3), 4);
      if (magnitude < 0) magnitude = 0;
      let scaled = { digits: number.digits, point: number.point - magnitude * 3 };
      let rounded = compactRound(scaled, options);
      if (rounded.point > 3 && magnitude < 4) {
        magnitude++;
        scaled = { digits: number.digits, point: number.point - magnitude * 3 };
        rounded = compactRound(scaled, options);
      }
      pushDigits(parts, numberStrings(rounded, options.minimumIntegerDigits), data, options.useGrouping);
      if (magnitude > 0) {
        if (options.compactDisplay === 'long') {
          parts.push({ type: 'literal', value: ' ' });
          parts.push({ type: 'compact', value: LONG_COMPACT[magnitude] });
        } else parts.push({ type: 'compact', value: SHORT_COMPACT[magnitude] });
      }
      return parts;
    }
    const rounded = roundNumber(number, options.rounding);
    pushDigits(parts, numberStrings(rounded, options.minimumIntegerDigits), data, options.useGrouping);
    return parts;
  }

  const compactRound = (scaled, options) => {
    if (options.roundingExplicit) return roundNumber(scaled, options.rounding);
    // Two significant digits below 100, integers from then on.
    if (scaled.point >= 2) return { ...roundDigits(scaled, scaled.point), minimumFraction: 0 };
    return { ...roundDigits(scaled, 2), minimumFraction: 0 };
  };

  function pushDigits(parts, { integer, fraction }, data, useGrouping) {
    const groups = groupInteger(integer, data, useGrouping);
    groups.forEach((group, index) => {
      if (index > 0) parts.push({ type: 'group', value: data.group });
      parts.push({ type: 'integer', value: group });
    });
    if (fraction) {
      parts.push({ type: 'decimal', value: data.decimal });
      parts.push({ type: 'fraction', value: fraction });
    }
  }

  const isZeroParts = (parts) => parts.every((part) => !['integer', 'fraction'].includes(part.type) || /^0*$/.test(part.value));

  function numberParts(nf, value) {
    const { data, options } = nf;
    const negative = typeof value === 'bigint' ? value < 0n : value < 0 || Object.is(value, -0);
    let body;
    if (typeof value === 'number' && Number.isNaN(value)) body = [{ type: 'nan', value: 'NaN' }];
    else if (typeof value === 'number' && !Number.isFinite(value)) body = [{ type: 'infinity', value: '∞' }];
    else {
      let number = decimalDigits(typeof value === 'bigint' ? value : Math.abs(value));
      if (options.style === 'percent') number = { ...number, point: number.point + 2 };
      body = digitParts(nf, number);
    }
    const zero = isZeroParts(body) && !body.some((part) => part.type === 'nan' || part.type === 'infinity');
    let sign = '';
    switch (options.signDisplay) {
      case 'auto':
        sign = negative && !Number.isNaN(value) ? '-' : '';
        break;
      case 'always':
        sign = negative ? '-' : '+';
        break;
      case 'exceptZero':
        sign = zero ? '' : negative ? '-' : '+';
        break;
      case 'negative':
        sign = negative && !zero ? '-' : '';
        break;
      default:
        sign = '';
    }
    return decorate(nf, body, sign, negative);
  }

  const signPart = (sign, data) => (sign === '-' ? { type: 'minusSign', value: data.minus } : { type: 'plusSign', value: '+' });

  function decorate(nf, body, sign, negative) {
    const { data, options } = nf;
    const parts = [];
    const pushSign = () => {
      if (sign) parts.push(signPart(sign, data));
    };
    if (options.style === 'currency') {
      const code = options.currency;
      const names = CURRENCIES[code];
      let symbol;
      if (options.currencyDisplay === 'code') symbol = code;
      else if (options.currencyDisplay === 'name') symbol = null;
      else if (code === 'JPY' && language(data.locale) === 'ja') symbol = '￥';
      else if (!data.english && data.currency === code && data.currencySymbol) symbol = data.currencySymbol;
      else symbol = names ? names[options.currencyDisplay === 'narrowSymbol' ? 1 : 0] : code;
      if (symbol === null) {
        pushSign();
        parts.push(...body);
        const one = body.length === 1 && body[0].value === '1';
        parts.push({ type: 'literal', value: ' ' });
        parts.push({ type: 'currency', value: names ? names[one ? 2 : 3] : code });
        return parts;
      }
      const spaced = options.currencyDisplay === 'code' || /^[A-Z]{3}$/.test(symbol);
      const accounting = options.currencySign === 'accounting' && sign === '-';
      const pattern = data.english ? 0 : data.currencyPattern || 0;
      if (accounting) parts.push({ type: 'literal', value: '(' });
      else pushSign();
      if (pattern === 0 || pattern === 2) {
        parts.push({ type: 'currency', value: symbol });
        if (pattern === 2 || spaced) parts.push({ type: 'literal', value: ' ' });
        parts.push(...body);
      } else {
        parts.push(...body);
        parts.push({ type: 'literal', value: ' ' });
        parts.push({ type: 'currency', value: symbol });
      }
      if (accounting) parts.push({ type: 'literal', value: ')' });
      return parts;
    }
    pushSign();
    parts.push(...body);
    if (options.style === 'percent') {
      const spaced = !data.english && (data.percentPattern === 0 || data.percentPattern === 3);
      if (spaced) parts.push({ type: 'literal', value: ' ' });
      parts.push({ type: 'percentSign', value: data.percent });
    } else if (options.style === 'unit') {
      const [first, per] = options.unit.split('-per-');
      const names = UNITS[first];
      const one = body.length === 1 && body[0].value === '1' && !negative;
      const display = options.unitDisplay;
      let unit = display === 'long' ? names[one ? 2 : 3] : display === 'narrow' ? names[4] : names[one ? 0 : 1];
      if (per) {
        const other = UNITS[per];
        unit = display === 'long' ? `${unit} per ${other[2]}` : `${unit}/${per === 'hour' ? 'h' : other[0]}`;
      }
      const tight = display === 'narrow' || (display === 'short' && options.unit === 'percent');
      if (!tight || (display === 'narrow' && !NARROW_TIGHT.includes(options.unit) && unit.length > 2 && !/^[a-z]{1,2}$/i.test(unit))) {
        parts.push({ type: 'literal', value: ' ' });
      }
      parts.push({ type: 'unit', value: unit });
    }
    return parts;
  }

  const SANCTIONED_UNITS = Object.keys(UNITS);

  class NumberFormat {
    constructor(locales, options) {
      const { locale, keywords, data } = resolveLocale(locales);
      options = toOptions(options);
      const style = getOption(options, 'style', ['decimal', 'percent', 'currency', 'unit'], 'decimal');
      let currency = options.currency === undefined ? undefined : String(options.currency).toUpperCase();
      if (currency !== undefined && !/^[A-Z]{3}$/.test(currency)) throw new RangeError(`Invalid currency code : ${options.currency}`);
      if (style === 'currency' && currency === undefined) throw new TypeError('Currency code is required with currency style.');
      const currencyDisplay = getOption(options, 'currencyDisplay', ['code', 'symbol', 'narrowSymbol', 'name'], 'symbol');
      const currencySign = getOption(options, 'currencySign', ['standard', 'accounting'], 'standard');
      const unit = options.unit === undefined ? undefined : String(options.unit);
      if (style === 'unit') {
        if (unit === undefined) throw new TypeError('Unit is required with unit style.');
        const valid = unit.split('-per-').every((part) => SANCTIONED_UNITS.includes(part));
        if (!valid) throw new RangeError(`Invalid unit argument for Intl.NumberFormat() '${unit}'`);
      }
      const unitDisplay = getOption(options, 'unitDisplay', ['short', 'narrow', 'long'], 'short');
      const notation = getOption(options, 'notation', ['standard', 'scientific', 'engineering', 'compact'], 'standard');
      const digits = style === 'currency' ? currencyDigits(currency) : 0;
      const defaultMinimum = style === 'currency' && notation !== 'compact' ? digits : 0;
      const defaultMaximum = style === 'currency' && notation !== 'compact' ? digits : style === 'percent' ? 0 : notation === 'compact' ? 0 : 3;
      const minimumIntegerDigits = getNumber(options, 'minimumIntegerDigits', 1, 21, 1);
      const minimumSignificantDigits = options.minimumSignificantDigits;
      const maximumSignificantDigits = options.maximumSignificantDigits;
      const minimumFractionDigits = options.minimumFractionDigits;
      const maximumFractionDigits = options.maximumFractionDigits;
      const significant = minimumSignificantDigits !== undefined || maximumSignificantDigits !== undefined;
      let rounding;
      if (significant) {
        const minimum = getNumber(options, 'minimumSignificantDigits', 1, 21, 1);
        const maximum = getNumber(options, 'maximumSignificantDigits', minimum, 21, 21);
        rounding = { significant: true, minimumSignificantDigits: minimum, maximumSignificantDigits: maximum };
      } else {
        let minimum = getNumber(options, 'minimumFractionDigits', 0, 100, undefined);
        let maximum = getNumber(options, 'maximumFractionDigits', 0, 100, undefined);
        if (minimum === undefined) minimum = Math.min(defaultMinimum, maximum === undefined ? defaultMinimum : maximum);
        if (maximum === undefined) maximum = Math.max(defaultMaximum, minimum);
        else if (minimum > maximum) throw new RangeError('maximumFractionDigits value is out of range.');
        rounding = { significant: false, minimumFractionDigits: minimum, maximumFractionDigits: maximum };
      }
      const roundingExplicit = significant || minimumFractionDigits !== undefined || maximumFractionDigits !== undefined;
      const compactDisplay = getOption(options, 'compactDisplay', ['short', 'long'], 'short');
      let useGrouping = options.useGrouping;
      if (useGrouping === undefined) useGrouping = notation === 'compact' ? 'min2' : 'auto';
      else if (useGrouping === true || useGrouping === 'true') useGrouping = 'always';
      else if (useGrouping === false || useGrouping === 'false' || useGrouping === null || useGrouping === '') useGrouping = false;
      else useGrouping = getOption({ useGrouping }, 'useGrouping', ['min2', 'auto', 'always'], 'auto');
      const signDisplay = getOption(options, 'signDisplay', ['auto', 'never', 'always', 'exceptZero', 'negative'], 'auto');
      define(this, '_state', {
        data,
        locale,
        options: {
          locale,
          numberingSystem: keywords.nu || 'latn',
          style,
          currency,
          currencyDisplay,
          currencySign,
          unit,
          unitDisplay,
          minimumIntegerDigits,
          rounding,
          roundingExplicit,
          notation,
          compactDisplay,
          useGrouping,
          signDisplay,
        },
      });
      define(this, 'format', (value) => this.formatToParts(value).map((part) => part.value).join(''));
    }

    formatToParts(value) {
      const number = typeof value === 'bigint' ? value : typeof value === 'string' && /^-?\d+$/.test(value.trim()) && Math.abs(Number(value)) > Number.MAX_SAFE_INTEGER ? BigInt(value.trim()) : Number(value);
      return numberParts(this._state, number).map((part) => ({ type: part.type, value: part.value }));
    }

    formatRange(start, end) {
      if (start === undefined || end === undefined) throw new TypeError('start or end is undefined');
      const first = this.format(start);
      const second = this.format(end);
      return first === second ? `~${first}` : `${first}–${second}`;
    }

    resolvedOptions() {
      const options = this._state.options;
      const result = { locale: options.locale, numberingSystem: options.numberingSystem, style: options.style };
      if (options.style === 'currency') Object.assign(result, { currency: options.currency, currencyDisplay: options.currencyDisplay, currencySign: options.currencySign });
      if (options.style === 'unit') Object.assign(result, { unit: options.unit, unitDisplay: options.unitDisplay });
      result.minimumIntegerDigits = options.minimumIntegerDigits;
      if (options.rounding.significant) {
        result.minimumSignificantDigits = options.rounding.minimumSignificantDigits;
        result.maximumSignificantDigits = options.rounding.maximumSignificantDigits;
      } else {
        result.minimumFractionDigits = options.rounding.minimumFractionDigits;
        result.maximumFractionDigits = options.rounding.maximumFractionDigits;
      }
      result.useGrouping = options.useGrouping;
      result.notation = options.notation;
      if (options.notation === 'compact') result.compactDisplay = options.compactDisplay;
      result.signDisplay = options.signDisplay;
      result.roundingIncrement = 1;
      result.roundingMode = 'halfExpand';
      result.roundingPriority = 'auto';
      result.trailingZeroDisplay = 'auto';
      return result;
    }
  }
  tag(NumberFormat.prototype, 'Intl.NumberFormat');

  // Plural rules. --------------------------------------------------------------

  // CLDR cardinal rules by language, over the operands of a formatted number:
  // n absolute value, i integer digits, v fraction digit count, f fraction.
  const between = (value, low, high) => value >= low && value <= high;
  const CARDINAL = {
    en: (o) => (o.i === 1 && o.v === 0 ? 'one' : 'other'),
    fr: (o) => (o.i === 0 || o.i === 1 ? 'one' : 'other'),
    pt: (o) => (between(o.i, 0, 1) ? 'one' : 'other'),
    es: (o) => (o.n === 1 ? 'one' : 'other'),
    ru: (o) => {
      if (o.v !== 0) return 'other';
      if (o.i % 10 === 1 && o.i % 100 !== 11) return 'one';
      if (between(o.i % 10, 2, 4) && !between(o.i % 100, 12, 14)) return 'few';
      return 'many';
    },
    pl: (o) => {
      if (o.v !== 0) return 'other';
      if (o.i === 1) return 'one';
      if (between(o.i % 10, 2, 4) && !between(o.i % 100, 12, 14)) return 'few';
      return 'many';
    },
    cs: (o) => (o.v !== 0 ? 'many' : o.i === 1 ? 'one' : between(o.i, 2, 4) ? 'few' : 'other'),
    ar: (o) => {
      if (o.n === 0) return 'zero';
      if (o.n === 1) return 'one';
      if (o.n === 2) return 'two';
      if (o.v === 0 && between(o.i % 100, 3, 10)) return 'few';
      if (o.v === 0 && between(o.i % 100, 11, 99)) return 'many';
      return 'other';
    },
    he: (o) => (o.v === 0 && o.i === 1 ? 'one' : o.v === 0 && o.i === 2 ? 'two' : 'other'),
    lt: (o) => {
      if (o.f !== 0) return 'many';
      if (o.i % 10 === 1 && !between(o.i % 100, 11, 19)) return 'one';
      if (between(o.i % 10, 2, 9) && !between(o.i % 100, 11, 19)) return 'few';
      return 'other';
    },
    other: () => 'other',
  };
  const PLURAL_FAMILY = {
    de: 'en', nl: 'en', sv: 'en', da: 'en', nb: 'en', no: 'en', fi: 'en', et: 'en', it: 'en', ca: 'en', el: 'en', hu: 'es', tr: 'es', bg: 'es', ka: 'es',
    uk: 'ru', be: 'ru', sk: 'cs', hi: 'fr', bn: 'fr', fa: 'fr', ja: 'other', zh: 'other', ko: 'other', th: 'other', vi: 'other', id: 'other', ms: 'other', yue: 'other',
  };
  const cardinalRule = (locale) => {
    const code = language(locale);
    if (code === 'pt' && locale === 'pt-PT') return CARDINAL.en;
    return CARDINAL[code] || CARDINAL[PLURAL_FAMILY[code]] || CARDINAL.en;
  };
  const englishOrdinal = (o) => {
    if (o.v !== 0) return 'other';
    if (o.i % 10 === 1 && o.i % 100 !== 11) return 'one';
    if (o.i % 10 === 2 && o.i % 100 !== 12) return 'two';
    if (o.i % 10 === 3 && o.i % 100 !== 13) return 'few';
    return 'other';
  };

  const pluralOperands = (text) => {
    const [integer, fraction = ''] = text.replace(/^[-+]/, '').split('.');
    return {
      n: Math.abs(Number(text)),
      i: Number(integer),
      v: fraction.length,
      f: fraction ? Number(fraction) : 0,
    };
  };

  class PluralRules {
    constructor(locales, options) {
      const { locale } = resolveLocale(locales);
      options = toOptions(options);
      const type = getOption(options, 'type', ['cardinal', 'ordinal'], 'cardinal');
      const format = new NumberFormat('en-US', { ...pick(options, DIGIT_OPTIONS), useGrouping: false });
      define(this, '_state', { locale, type, format });
    }

    select(value) {
      const { locale, type, format } = this._state;
      const number = Number(value);
      if (!Number.isFinite(number)) return 'other';
      const operands = pluralOperands(format.format(number));
      if (type === 'ordinal') return language(locale) === 'en' ? englishOrdinal(operands) : 'other';
      return cardinalRule(locale)(operands);
    }

    selectRange(start, end) {
      return this.select(end);
    }

    resolvedOptions() {
      const { locale, type, format } = this._state;
      const digits = format.resolvedOptions();
      const categories = type === 'ordinal' ? (language(locale) === 'en' ? ['few', 'one', 'two', 'other'] : ['other']) : pluralCategories(locale);
      return {
        locale,
        type,
        minimumIntegerDigits: digits.minimumIntegerDigits,
        minimumFractionDigits: digits.minimumFractionDigits,
        maximumFractionDigits: digits.maximumFractionDigits,
        ...(digits.minimumSignificantDigits === undefined ? {} : { minimumSignificantDigits: digits.minimumSignificantDigits, maximumSignificantDigits: digits.maximumSignificantDigits }),
        pluralCategories: categories,
        roundingIncrement: 1,
        roundingMode: 'halfExpand',
        roundingPriority: 'auto',
        trailingZeroDisplay: 'auto',
      };
    }
  }
  tag(PluralRules.prototype, 'Intl.PluralRules');

  const DIGIT_OPTIONS = ['minimumIntegerDigits', 'minimumFractionDigits', 'maximumFractionDigits', 'minimumSignificantDigits', 'maximumSignificantDigits'];
  const pick = (options, names) => {
    const result = {};
    for (const name of names) if (options[name] !== undefined) result[name] = options[name];
    return result;
  };
  const pluralCategories = (locale) => {
    const rule = cardinalRule(locale);
    const found = new Set();
    for (const sample of ['0', '1', '2', '3', '5', '11', '22', '100', '0.5', '1.5', '21', '25']) found.add(rule(pluralOperands(sample)));
    const order = ['zero', 'one', 'two', 'few', 'many', 'other'];
    return order.filter((category) => found.has(category));
  };

  // Relative times. --------------------------------------------------------------

  const RELATIVE_UNITS = ['year', 'quarter', 'month', 'week', 'day', 'hour', 'minute', 'second'];
  const RELATIVE_NAMES = {
    long: { year: ['year', 'years'], quarter: ['quarter', 'quarters'], month: ['month', 'months'], week: ['week', 'weeks'], day: ['day', 'days'], hour: ['hour', 'hours'], minute: ['minute', 'minutes'], second: ['second', 'seconds'] },
    short: { year: ['yr.', 'yr.'], quarter: ['qtr.', 'qtrs.'], month: ['mo.', 'mo.'], week: ['wk.', 'wk.'], day: ['day', 'days'], hour: ['hr.', 'hr.'], minute: ['min.', 'min.'], second: ['sec.', 'sec.'] },
    narrow: { year: ['y', 'y'], quarter: ['q', 'q'], month: ['mo', 'mo'], week: ['w', 'w'], day: [' day', ' days'], hour: ['h', 'h'], minute: ['m', 'm'], second: ['s', 's'] },
  };
  const RELATIVE_WORDS = {
    day: ['yesterday', 'today', 'tomorrow'],
    year: ['last year', 'this year', 'next year'],
    quarter: ['last quarter', 'this quarter', 'next quarter'],
    month: ['last month', 'this month', 'next month'],
    week: ['last week', 'this week', 'next week'],
    hour: [null, 'this hour', null],
    minute: [null, 'this minute', null],
    second: [null, 'now', null],
  };
  const SHORT_WORDS = { year: 'yr.', quarter: 'qtr.', month: 'mo.', week: 'wk.' };

  class RelativeTimeFormat {
    constructor(locales, options) {
      const { locale } = resolveLocale(locales);
      options = toOptions(options);
      const style = getOption(options, 'style', ['long', 'short', 'narrow'], 'long');
      const numeric = getOption(options, 'numeric', ['always', 'auto'], 'always');
      define(this, '_state', { locale, style, numeric, number: new NumberFormat(locale), plural: new PluralRules('en') });
    }

    formatToParts(value, unit) {
      const { style, numeric, number, plural } = this._state;
      const name = String(unit).replace(/s$/, '');
      if (!RELATIVE_UNITS.includes(name)) throw new RangeError(`Invalid unit argument for format() '${unit}'`);
      value = Number(value);
      if (!Number.isFinite(value)) throw new RangeError('Invalid time value');
      if (numeric === 'auto' && [-1, 0, 1].includes(value) && !(Object.is(value, -0))) {
        let word = RELATIVE_WORDS[name][value + 1];
        if (word && style !== 'long' && SHORT_WORDS[name]) word = word.replace(name, SHORT_WORDS[name]);
        if (word) return [{ type: 'literal', value: word }];
      }
      const past = value < 0 || Object.is(value, -0);
      const parts = number.formatToParts(Math.abs(value)).map((part) => ({ ...part, unit: name }));
      const names = RELATIVE_NAMES[style][name];
      const label = names[plural.select(Math.abs(value)) === 'one' ? 0 : 1];
      const tight = style === 'narrow' && !label.startsWith(' ');
      const unitText = tight ? label : ` ${label.trim()}`;
      return past ? [...parts, { type: 'literal', value: `${unitText} ago` }] : [{ type: 'literal', value: 'in ' }, ...parts, { type: 'literal', value: unitText }];
    }

    format(value, unit) {
      return this.formatToParts(value, unit).map((part) => part.value).join('');
    }

    resolvedOptions() {
      const { locale, style, numeric } = this._state;
      return { locale, style, numeric, numberingSystem: 'latn' };
    }
  }
  tag(RelativeTimeFormat.prototype, 'Intl.RelativeTimeFormat');

  // Lists. ------------------------------------------------------------------------

  class ListFormat {
    constructor(locales, options) {
      const { locale } = resolveLocale(locales);
      options = toOptions(options);
      const type = getOption(options, 'type', ['conjunction', 'disjunction', 'unit'], 'conjunction');
      const style = getOption(options, 'style', ['long', 'short', 'narrow'], 'long');
      define(this, '_state', { locale, type, style });
    }

    formatToParts(list) {
      const { type, style } = this._state;
      const items = [];
      for (const item of list) {
        if (typeof item !== 'string') throw new TypeError(`Iterable yielded ${item} which is not a string`);
        items.push(item);
      }
      const word = type === 'disjunction' ? 'or' : type === 'unit' ? '' : style === 'long' ? 'and' : style === 'short' ? '&' : '';
      const separator = type === 'unit' && style === 'narrow' ? ' ' : ', ';
      const parts = [];
      items.forEach((item, index) => {
        if (index > 0) {
          const last = index === items.length - 1;
          let literal = separator;
          if (last && word) literal = items.length === 2 ? ` ${word} ` : `, ${word} `;
          parts.push({ type: 'literal', value: literal });
        }
        parts.push({ type: 'element', value: item });
      });
      return parts;
    }

    format(list) {
      return this.formatToParts(list).map((part) => part.value).join('');
    }

    resolvedOptions() {
      const { locale, type, style } = this._state;
      return { locale, type, style };
    }
  }
  tag(ListFormat.prototype, 'Intl.ListFormat');

  // Collation: the root collation's levels, approximated. Spaces and
  // punctuation sort before digits, digits before letters; letters compare
  // without accents and case first, then by accents, then lower case first.

  const MARK = /^\p{M}$/u;
  const classOf = (character) => (/\s/.test(character) ? 0 : /[\p{P}\p{S}]/u.test(character) ? 1 : /\p{N}/u.test(character) ? 2 : 3);

  const collationElements = (text, numeric, ignorePunctuation) => {
    const elements = [];
    // Marks before any character weigh on the first one.
    let leading = '';
    const characters = Array.from(String(text).normalize('NFD'));
    for (let index = 0; index < characters.length; index++) {
      const character = characters[index];
      if (MARK.test(character)) {
        if (elements.length) elements[elements.length - 1].accents += character;
        else leading += character;
        continue;
      }
      const kind = classOf(character);
      if (ignorePunctuation && kind <= 1) continue;
      if (numeric && /\d/.test(character)) {
        let digits = character;
        while (index + 1 < characters.length && /\d/.test(characters[index + 1])) digits += characters[++index];
        const value = digits.replace(/^0+(?=\d)/, '');
        elements.push({ kind: 2, primary: value.length.toString().padStart(4, '0') + value, accents: '', upper: false, original: digits });
        continue;
      }
      const lower = character.toLowerCase();
      elements.push({ kind, primary: lower, accents: leading, upper: lower !== character, original: character });
      leading = '';
    }
    return elements;
  };

  const compareStrings = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

  class Collator {
    constructor(locales, options) {
      const { locale, keywords } = resolveLocale(locales);
      options = toOptions(options);
      const usage = getOption(options, 'usage', ['sort', 'search'], 'sort');
      const sensitivity = getOption(options, 'sensitivity', ['base', 'accent', 'case', 'variant'], 'variant');
      const ignorePunctuation = getBoolean(options, 'ignorePunctuation', false);
      const numeric = getBoolean(options, 'numeric', keywords.kn === 'true');
      const caseFirst = getOption(options, 'caseFirst', ['upper', 'lower', 'false'], keywords.kf || 'false');
      define(this, '_state', { locale, usage, sensitivity, ignorePunctuation, numeric, caseFirst });
      define(this, 'compare', (left, right) => this._compare(String(left), String(right)));
    }

    _compare(left, right) {
      const { sensitivity, ignorePunctuation, numeric, caseFirst } = this._state;
      const a = collationElements(left, numeric, ignorePunctuation);
      const b = collationElements(right, numeric, ignorePunctuation);
      const length = Math.min(a.length, b.length);
      for (let index = 0; index < length; index++) {
        if (a[index].kind !== b[index].kind) return a[index].kind < b[index].kind ? -1 : 1;
        const order = compareStrings(a[index].primary, b[index].primary);
        if (order) return order;
      }
      if (a.length !== b.length) return a.length < b.length ? -1 : 1;
      if (sensitivity === 'accent' || sensitivity === 'variant') {
        for (let index = 0; index < length; index++) {
          const order = compareStrings(a[index].accents, b[index].accents);
          if (order) return order;
        }
      }
      if (sensitivity === 'case' || sensitivity === 'variant') {
        for (let index = 0; index < length; index++) {
          if (a[index].upper !== b[index].upper) return (a[index].upper ? 1 : -1) * (caseFirst === 'upper' ? -1 : 1);
        }
      }
      return 0;
    }

    resolvedOptions() {
      const { locale, usage, sensitivity, ignorePunctuation, numeric, caseFirst } = this._state;
      return { locale, usage, sensitivity, ignorePunctuation, collation: 'default', numeric, caseFirst };
    }
  }
  tag(Collator.prototype, 'Intl.Collator');

  // Segmentation. ------------------------------------------------------------------

  const EXTENDS = /^[\p{M}‍︎️\u{1f3fb}-\u{1f3ff}\u{e0020}-\u{e007f}]$/u;
  const REGIONAL = /^[\u{1f1e6}-\u{1f1ff}]$/u;

  const graphemes = (text) => {
    const result = [];
    const characters = Array.from(text);
    let index = 0;
    for (let position = 0; position < characters.length; position++) {
      let segment = characters[position];
      if (segment === '\r' && characters[position + 1] === '\n') segment += characters[++position];
      else if (REGIONAL.test(segment) && REGIONAL.test(characters[position + 1] || '')) segment += characters[++position];
      while (position + 1 < characters.length && EXTENDS.test(characters[position + 1])) {
        segment += characters[++position];
        if (segment.endsWith('‍') && position + 1 < characters.length) segment += characters[++position];
      }
      result.push({ segment, index });
      index += segment.length;
    }
    return result;
  };

  const words = (text) => {
    const result = [];
    const pattern = /[\p{L}\p{N}\p{M}_'’]+|\s+|[^\s\p{L}\p{N}]/gu;
    let match;
    while ((match = pattern.exec(text))) {
      const isWordLike = /[\p{L}\p{N}]/u.test(match[0]);
      result.push({ segment: match[0], index: match.index, isWordLike });
    }
    return result;
  };

  const sentences = (text) => {
    const result = [];
    const pattern = /[^.!?。！？]*(?:[.!?。！？]+["')\]]*\s*|$)/gu;
    let match;
    while ((match = pattern.exec(text)) && match[0]) result.push({ segment: match[0], index: match.index });
    return result;
  };

  class Segmenter {
    constructor(locales, options) {
      const { locale } = resolveLocale(locales);
      options = toOptions(options);
      const granularity = getOption(options, 'granularity', ['grapheme', 'word', 'sentence'], 'grapheme');
      define(this, '_state', { locale, granularity });
    }

    segment(input) {
      const text = String(input);
      const { granularity } = this._state;
      const list = (granularity === 'word' ? words : granularity === 'sentence' ? sentences : graphemes)(text).map((item) => ({ ...item, input: text }));
      return {
        [Symbol.iterator]: () => list[Symbol.iterator](),
        containing(index = 0) {
          const position = Number(index);
          return list.find((item) => position >= item.index && position < item.index + item.segment.length);
        },
      };
    }

    resolvedOptions() {
      const { locale, granularity } = this._state;
      return { locale, granularity };
    }
  }
  tag(Segmenter.prototype, 'Intl.Segmenter');

  // Locale objects. --------------------------------------------------------------

  const LIKELY = { en: 'en-Latn-US', fr: 'fr-Latn-FR', de: 'de-Latn-DE', es: 'es-Latn-ES', it: 'it-Latn-IT', pt: 'pt-Latn-BR', ja: 'ja-Jpan-JP', zh: 'zh-Hans-CN', ko: 'ko-Kore-KR', ru: 'ru-Cyrl-RU', ar: 'ar-Arab-EG', hi: 'hi-Deva-IN', nl: 'nl-Latn-NL', sv: 'sv-Latn-SE', pl: 'pl-Latn-PL', tr: 'tr-Latn-TR', uk: 'uk-Cyrl-UA', he: 'he-Hebr-IL', th: 'th-Thai-TH', vi: 'vi-Latn-VN' };
  const KEYWORDS = { calendar: 'ca', collation: 'co', hourCycle: 'hc', caseFirst: 'kf', numeric: 'kn', numberingSystem: 'nu' };

  class Locale {
    constructor(tagName, options) {
      if (tagName === undefined || (typeof tagName !== 'string' && !(tagName instanceof Locale) && (typeof tagName !== 'object' || tagName === null))) {
        throw new TypeError('First argument to Intl.Locale constructor can\'t be empty or missing');
      }
      const canonical = canonicalTag(tagName instanceof Locale ? tagName.toString() : String(tagName));
      const { base, keywords } = splitTag(canonical);
      options = toOptions(options);
      const [languagePart, ...rest] = base.split('-');
      let script = rest.find((part) => part.length === 4);
      let region = rest.find((part) => part.length === 2 || /^\d{3}$/.test(part));
      const languageName = options.language === undefined ? languagePart : String(options.language);
      if (options.script !== undefined) script = String(options.script);
      if (options.region !== undefined) region = String(options.region).toUpperCase();
      for (const [name, key] of Object.entries(KEYWORDS)) {
        if (options[name] !== undefined) keywords[key] = name === 'numeric' ? String(Boolean(options[name])) : String(options[name]);
      }
      define(this, '_state', { language: languageName, script, region, keywords });
    }

    get language() {
      return this._state.language;
    }

    get script() {
      return this._state.script;
    }

    get region() {
      return this._state.region;
    }

    get baseName() {
      const { language: code, script, region } = this._state;
      return [code, script, region].filter(Boolean).join('-');
    }

    get calendar() {
      return this._state.keywords.ca;
    }

    get collation() {
      return this._state.keywords.co;
    }

    get hourCycle() {
      return this._state.keywords.hc;
    }

    get caseFirst() {
      return this._state.keywords.kf;
    }

    get numeric() {
      return this._state.keywords.kn === 'true';
    }

    get numberingSystem() {
      return this._state.keywords.nu;
    }

    maximize() {
      const likely = LIKELY[this.language];
      if (!likely) return new Locale(this.toString());
      const [, script, region] = likely.split('-');
      return new Locale(this.toString(), { script: this.script || script, region: this.region || region });
    }

    minimize() {
      return new Locale(this.toString(), {}).withBase(this.language);
    }

    withBase(base) {
      const keywords = Object.entries(this._state.keywords).map(([key, value]) => (value === 'true' ? key : `${key}-${value}`));
      return new Locale(keywords.length ? `${base}-u-${keywords.join('-')}` : base);
    }

    toString() {
      const keywords = Object.entries(this._state.keywords)
        .sort(([a], [b]) => (a < b ? -1 : 1))
        .map(([key, value]) => (value === 'true' ? key : `${key}-${value}`));
      return keywords.length ? `${this.baseName}-u-${keywords.join('-')}` : this.baseName;
    }
  }
  tag(Locale.prototype, 'Intl.Locale');

  // Time zones: Windows zones and their IANA names, as CLDR maps them. -------------

  const WINDOWS_ZONES = {
    'Dateline Standard Time': 'Etc/GMT+12', 'UTC-11': 'Etc/GMT+11', 'Aleutian Standard Time': 'America/Adak', 'Hawaiian Standard Time': 'Pacific/Honolulu',
    'Marquesas Standard Time': 'Pacific/Marquesas', 'Alaskan Standard Time': 'America/Anchorage', 'UTC-09': 'Etc/GMT+9', 'Pacific Standard Time (Mexico)': 'America/Tijuana',
    'UTC-08': 'Etc/GMT+8', 'Pacific Standard Time': 'America/Los_Angeles', 'US Mountain Standard Time': 'America/Phoenix', 'Mountain Standard Time (Mexico)': 'America/Mazatlan',
    'Mountain Standard Time': 'America/Denver', 'Yukon Standard Time': 'America/Whitehorse', 'Central America Standard Time': 'America/Guatemala', 'Central Standard Time': 'America/Chicago',
    'Easter Island Standard Time': 'Pacific/Easter', 'Central Standard Time (Mexico)': 'America/Mexico_City', 'Canada Central Standard Time': 'America/Regina', 'SA Pacific Standard Time': 'America/Bogota',
    'Eastern Standard Time (Mexico)': 'America/Cancun', 'Eastern Standard Time': 'America/New_York', 'Haiti Standard Time': 'America/Port-au-Prince', 'Cuba Standard Time': 'America/Havana',
    'US Eastern Standard Time': 'America/Indianapolis', 'Turks And Caicos Standard Time': 'America/Grand_Turk', 'Paraguay Standard Time': 'America/Asuncion', 'Atlantic Standard Time': 'America/Halifax',
    'Venezuela Standard Time': 'America/Caracas', 'Central Brazilian Standard Time': 'America/Cuiaba', 'SA Western Standard Time': 'America/La_Paz', 'Pacific SA Standard Time': 'America/Santiago',
    'Newfoundland Standard Time': 'America/St_Johns', 'Tocantins Standard Time': 'America/Araguaina', 'E. South America Standard Time': 'America/Sao_Paulo', 'SA Eastern Standard Time': 'America/Cayenne',
    'Argentina Standard Time': 'America/Buenos_Aires', 'Greenland Standard Time': 'America/Godthab', 'Montevideo Standard Time': 'America/Montevideo', 'Magallanes Standard Time': 'America/Punta_Arenas',
    'Saint Pierre Standard Time': 'America/Miquelon', 'Bahia Standard Time': 'America/Bahia', 'UTC-02': 'Etc/GMT+2', 'Azores Standard Time': 'Atlantic/Azores',
    'Cape Verde Standard Time': 'Atlantic/Cape_Verde', UTC: 'UTC', 'GMT Standard Time': 'Europe/London', 'Greenwich Standard Time': 'Atlantic/Reykjavik',
    'Sao Tome Standard Time': 'Africa/Sao_Tome', 'Morocco Standard Time': 'Africa/Casablanca', 'W. Europe Standard Time': 'Europe/Berlin', 'Central Europe Standard Time': 'Europe/Budapest',
    'Romance Standard Time': 'Europe/Paris', 'Central European Standard Time': 'Europe/Warsaw', 'W. Central Africa Standard Time': 'Africa/Lagos', 'Jordan Standard Time': 'Asia/Amman',
    'GTB Standard Time': 'Europe/Bucharest', 'Middle East Standard Time': 'Asia/Beirut', 'Egypt Standard Time': 'Africa/Cairo', 'E. Europe Standard Time': 'Europe/Chisinau',
    'Syria Standard Time': 'Asia/Damascus', 'West Bank Standard Time': 'Asia/Hebron', 'South Africa Standard Time': 'Africa/Johannesburg', 'FLE Standard Time': 'Europe/Kiev',
    'Israel Standard Time': 'Asia/Jerusalem', 'South Sudan Standard Time': 'Africa/Juba', 'Kaliningrad Standard Time': 'Europe/Kaliningrad', 'Sudan Standard Time': 'Africa/Khartoum',
    'Libya Standard Time': 'Africa/Tripoli', 'Namibia Standard Time': 'Africa/Windhoek', 'Arabic Standard Time': 'Asia/Baghdad', 'Turkey Standard Time': 'Europe/Istanbul',
    'Arab Standard Time': 'Asia/Riyadh', 'Belarus Standard Time': 'Europe/Minsk', 'Russian Standard Time': 'Europe/Moscow', 'E. Africa Standard Time': 'Africa/Nairobi',
    'Volgograd Standard Time': 'Europe/Volgograd', 'Iran Standard Time': 'Asia/Tehran', 'Arabian Standard Time': 'Asia/Dubai', 'Astrakhan Standard Time': 'Europe/Astrakhan',
    'Azerbaijan Standard Time': 'Asia/Baku', 'Russia Time Zone 3': 'Europe/Samara', 'Mauritius Standard Time': 'Indian/Mauritius', 'Saratov Standard Time': 'Europe/Saratov',
    'Georgian Standard Time': 'Asia/Tbilisi', 'Caucasus Standard Time': 'Asia/Yerevan', 'Afghanistan Standard Time': 'Asia/Kabul', 'West Asia Standard Time': 'Asia/Tashkent',
    'Ekaterinburg Standard Time': 'Asia/Yekaterinburg', 'Pakistan Standard Time': 'Asia/Karachi', 'Qyzylorda Standard Time': 'Asia/Qyzylorda', 'India Standard Time': 'Asia/Calcutta',
    'Sri Lanka Standard Time': 'Asia/Colombo', 'Nepal Standard Time': 'Asia/Katmandu', 'Central Asia Standard Time': 'Asia/Almaty', 'Bangladesh Standard Time': 'Asia/Dhaka',
    'Omsk Standard Time': 'Asia/Omsk', 'Myanmar Standard Time': 'Asia/Rangoon', 'SE Asia Standard Time': 'Asia/Bangkok', 'Altai Standard Time': 'Asia/Barnaul',
    'W. Mongolia Standard Time': 'Asia/Hovd', 'North Asia Standard Time': 'Asia/Krasnoyarsk', 'N. Central Asia Standard Time': 'Asia/Novosibirsk', 'Tomsk Standard Time': 'Asia/Tomsk',
    'China Standard Time': 'Asia/Shanghai', 'North Asia East Standard Time': 'Asia/Irkutsk', 'Singapore Standard Time': 'Asia/Singapore', 'W. Australia Standard Time': 'Australia/Perth',
    'Taipei Standard Time': 'Asia/Taipei', 'Ulaanbaatar Standard Time': 'Asia/Ulaanbaatar', 'Aus Central W. Standard Time': 'Australia/Eucla', 'Transbaikal Standard Time': 'Asia/Chita',
    'Tokyo Standard Time': 'Asia/Tokyo', 'North Korea Standard Time': 'Asia/Pyongyang', 'Korea Standard Time': 'Asia/Seoul', 'Yakutsk Standard Time': 'Asia/Yakutsk',
    'Cen. Australia Standard Time': 'Australia/Adelaide', 'AUS Central Standard Time': 'Australia/Darwin', 'E. Australia Standard Time': 'Australia/Brisbane', 'AUS Eastern Standard Time': 'Australia/Sydney',
    'West Pacific Standard Time': 'Pacific/Port_Moresby', 'Tasmania Standard Time': 'Australia/Hobart', 'Vladivostok Standard Time': 'Asia/Vladivostok', 'Lord Howe Standard Time': 'Australia/Lord_Howe',
    'Bougainville Standard Time': 'Pacific/Bougainville', 'Russia Time Zone 10': 'Asia/Srednekolymsk', 'Magadan Standard Time': 'Asia/Magadan', 'Norfolk Standard Time': 'Pacific/Norfolk',
    'Sakhalin Standard Time': 'Asia/Sakhalin', 'Central Pacific Standard Time': 'Pacific/Guadalcanal', 'Russia Time Zone 11': 'Asia/Kamchatka', 'New Zealand Standard Time': 'Pacific/Auckland',
    'UTC+12': 'Etc/GMT-12', 'Fiji Standard Time': 'Pacific/Fiji', 'Chatham Islands Standard Time': 'Pacific/Chatham', 'UTC+13': 'Etc/GMT-13',
    'Tonga Standard Time': 'Pacific/Tongatapu', 'Samoa Standard Time': 'Pacific/Apia', 'Line Islands Standard Time': 'Pacific/Kiritimati',
  };
  // Other names of the same zones, by the Windows zone that keeps their time.
  const ZONE_ALIASES = {
    'Eastern Standard Time': 'America/Toronto America/Detroit America/Montreal America/Nassau America/Indiana/Indianapolis',
    'Central Standard Time': 'America/Winnipeg America/Indiana/Knox',
    'Mountain Standard Time': 'America/Edmonton America/Boise',
    'Pacific Standard Time': 'America/Vancouver',
    'SA Pacific Standard Time': 'America/Lima America/Panama America/Jamaica',
    'Argentina Standard Time': 'America/Argentina/Buenos_Aires',
    'Greenland Standard Time': 'America/Nuuk',
    'GMT Standard Time': 'Europe/Dublin Europe/Lisbon Atlantic/Canary Atlantic/Faroe',
    'Greenwich Standard Time': 'Africa/Abidjan Africa/Accra Africa/Dakar',
    'W. Europe Standard Time': 'Europe/Amsterdam Europe/Andorra Europe/Rome Europe/Stockholm Europe/Oslo Europe/Vienna Europe/Zurich Europe/Luxembourg Europe/Monaco Europe/Malta Europe/Gibraltar',
    'Central Europe Standard Time': 'Europe/Prague Europe/Belgrade Europe/Bratislava Europe/Ljubljana Europe/Tirane',
    'Romance Standard Time': 'Europe/Brussels Europe/Copenhagen Europe/Madrid',
    'Central European Standard Time': 'Europe/Zagreb Europe/Sarajevo Europe/Skopje',
    'GTB Standard Time': 'Europe/Athens Asia/Nicosia',
    'FLE Standard Time': 'Europe/Kyiv Europe/Helsinki Europe/Riga Europe/Sofia Europe/Tallinn Europe/Vilnius',
    'Arab Standard Time': 'Asia/Kuwait Asia/Qatar Asia/Bahrain Asia/Aden',
    'Arabian Standard Time': 'Asia/Muscat',
    'India Standard Time': 'Asia/Kolkata',
    'Nepal Standard Time': 'Asia/Kathmandu',
    'Myanmar Standard Time': 'Asia/Yangon',
    'SE Asia Standard Time': 'Asia/Jakarta Asia/Ho_Chi_Minh Asia/Saigon Asia/Phnom_Penh Asia/Vientiane',
    'China Standard Time': 'Asia/Hong_Kong Asia/Macau Asia/Chongqing',
    'Singapore Standard Time': 'Asia/Kuala_Lumpur Asia/Manila',
    'Central Asia Standard Time': 'Asia/Bishkek',
    'AUS Eastern Standard Time': 'Australia/Melbourne Australia/Canberra',
    'Hawaiian Standard Time': 'US/Hawaii',
    'E. Africa Standard Time': 'Africa/Addis_Ababa Africa/Dar_es_Salaam Africa/Kampala',
    'South Africa Standard Time': 'Africa/Harare Africa/Maputo',
    'W. Central Africa Standard Time': 'Africa/Algiers Africa/Kinshasa Africa/Tunis',
  };
  const IANA_ZONES = new Map();
  for (const [key, name] of Object.entries(WINDOWS_ZONES)) IANA_ZONES.set(name.toLowerCase(), { name, key });
  for (const [key, names] of Object.entries(ZONE_ALIASES)) {
    for (const name of names.split(' ')) IANA_ZONES.set(name.toLowerCase(), { name, key });
  }
  // American zones have English abbreviations: name, standard offset and
  // the standard and daylight abbreviations.
  const AMERICAN_ZONES = {
    'America/Los_Angeles': [-480, 'P', 'Pacific'], 'America/Denver': [-420, 'M', 'Mountain'], 'America/Phoenix': [-420, 'M', 'Mountain'],
    'America/Chicago': [-360, 'C', 'Central'], 'America/New_York': [-300, 'E', 'Eastern'], 'America/Anchorage': [-540, 'AK', 'Alaska'],
    'Pacific/Honolulu': [-600, 'H', 'Hawaii-Aleutian'],
  };

  const localZone = () => {
    const key = typeof host.timeZone === 'function' ? String(host.timeZone()) : '';
    if (WINDOWS_ZONES[key]) return { name: WINDOWS_ZONES[key], local: true };
    const offset = -new Date().getTimezoneOffset();
    if (offset === 0) return { name: 'UTC', local: true };
    const hours = -offset / 60;
    return { name: Number.isInteger(hours) ? `Etc/GMT${hours > 0 ? '+' : ''}${hours}` : 'UTC', local: true };
  };

  const fixedZone = (name, minutes) => ({ name, offset: () => minutes });

  const resolveZone = (value) => {
    if (value === undefined) return localZone();
    const text = String(value);
    const upper = text.toUpperCase();
    if (['UTC', 'GMT', 'ETC/UTC', 'ETC/GMT', 'ETC/UCT', 'UCT', 'ETC/ZULU', 'ZULU', 'ETC/UNIVERSAL', 'UNIVERSAL', 'ETC/GREENWICH', 'GREENWICH'].includes(upper)) {
      return fixedZone('UTC', 0);
    }
    const fixed = /^([+-])(\d{2}):?(\d{2})$/.exec(text);
    if (fixed) {
      const minutes = (Number(fixed[2]) * 60 + Number(fixed[3])) * (fixed[1] === '-' ? -1 : 1);
      if (Math.abs(minutes) < 24 * 60) return fixedZone(`${fixed[1]}${fixed[2]}:${fixed[3]}`, minutes);
    }
    const etc = /^Etc\/GMT([+-])(\d{1,2})$/i.exec(text);
    if (etc && Number(etc[2]) <= 14) return fixedZone(`Etc/GMT${etc[1]}${Number(etc[2])}`, Number(etc[2]) * 60 * (etc[1] === '+' ? -1 : 1));
    const known = IANA_ZONES.get(text.toLowerCase());
    if (!known) throw new RangeError(`Invalid time zone specified: ${text}`);
    const local = localZone();
    if (local.name === WINDOWS_ZONES[known.key] && known.name === local.name) return local;
    return {
      name: known.name,
      offset: (ms) => {
        const minutes = typeof host.zoneOffset === 'function' ? host.zoneOffset(known.key, ms) : null;
        return typeof minutes === 'number' ? minutes : 0;
      },
    };
  };

  const fieldsOf = (date, zone) => {
    const ms = date.getTime();
    if (zone.local) {
      return {
        year: date.getFullYear(), month: date.getMonth(), day: date.getDate(), weekday: date.getDay(),
        hour: date.getHours(), minute: date.getMinutes(), second: date.getSeconds(), millisecond: date.getMilliseconds(),
        offset: -date.getTimezoneOffset(),
      };
    }
    const offset = zone.offset(ms);
    const shifted = new Date(ms + offset * 60000);
    return {
      year: shifted.getUTCFullYear(), month: shifted.getUTCMonth(), day: shifted.getUTCDate(), weekday: shifted.getUTCDay(),
      hour: shifted.getUTCHours(), minute: shifted.getUTCMinutes(), second: shifted.getUTCSeconds(), millisecond: shifted.getUTCMilliseconds(),
      offset,
    };
  };

  const offsetText = (minutes, long) => {
    if (minutes === 0) return 'GMT';
    const sign = minutes < 0 ? '-' : '+';
    const hours = Math.floor(Math.abs(minutes) / 60);
    const rest = Math.abs(minutes) % 60;
    if (long) return `GMT${sign}${String(hours).padStart(2, '0')}:${String(rest).padStart(2, '0')}`;
    return `GMT${sign}${hours}${rest ? `:${String(rest).padStart(2, '0')}` : ''}`;
  };

  const zoneName = (zone, offset, width) => {
    if (zone.name === 'UTC' && (width === 'short' || width === 'long')) return width === 'long' ? 'Coordinated Universal Time' : 'UTC';
    const american = AMERICAN_ZONES[zone.name];
    if (american && width !== 'shortOffset' && width !== 'longOffset') {
      const [standard, letter, name] = american;
      const daylight = offset !== standard;
      if (width === 'short') return `${letter}${daylight ? 'D' : 'S'}T`;
      if (width === 'long') return `${name} ${daylight ? 'Daylight' : 'Standard'} Time`;
      if (width === 'shortGeneric') return `${letter}T`;
      return `${name} Time`;
    }
    return offsetText(offset, width === 'long' || width === 'longOffset' || width === 'longGeneric');
  };

  // Date and time formatting. ------------------------------------------------------

  // Patterns are lists of tokens: field tokens { field, width } and literals.
  // CLDR pattern letters are used for English, Windows pictures otherwise.
  const CLDR_FIELDS = { G: 'era', y: 'year', M: 'month', L: 'month', d: 'day', E: 'weekday', c: 'weekday', a: 'dayPeriod', h: 'hour', H: 'hour', K: 'hour', k: 'hour', m: 'minute', s: 'second', S: 'fractionalSecond', z: 'timeZoneName', v: 'timeZoneName', O: 'timeZoneName' };

  const cldrWidth = (letter, count) => {
    switch (letter) {
      case 'y':
        return count === 2 ? '2-digit' : 'numeric';
      case 'M':
      case 'L':
        return ['numeric', '2-digit', 'short', 'long', 'narrow'][Math.min(count, 5) - 1];
      case 'E':
      case 'c':
        return count === 5 ? 'narrow' : count === 4 ? 'long' : 'short';
      case 'G':
        return count === 5 ? 'narrow' : count === 4 ? 'long' : 'short';
      case 'z':
        return count === 4 ? 'long' : 'short';
      case 'v':
        return count === 4 ? 'longGeneric' : 'shortGeneric';
      case 'O':
        return count === 4 ? 'longOffset' : 'shortOffset';
      case 'S':
        return count;
      default:
        return count === 2 ? '2-digit' : 'numeric';
    }
  };

  const tokenizeCldr = (pattern) => {
    const tokens = [];
    let index = 0;
    while (index < pattern.length) {
      const character = pattern[index];
      if (CLDR_FIELDS[character]) {
        let count = 1;
        while (pattern[index + count] === character) count++;
        tokens.push({
          field: CLDR_FIELDS[character],
          width: cldrWidth(character, count),
          standalone: character === 'L' || character === 'c',
          cycle: { h: 'h12', H: 'h23', K: 'h11', k: 'h24' }[character],
        });
        index += count;
      } else {
        tokens.push({ literal: character });
        index++;
      }
    }
    return tokens;
  };

  const WINDOWS_FIELDS = { d: 'day', M: 'month', y: 'year', g: 'era', h: 'hour', H: 'hour', m: 'minute', s: 'second', t: 'dayPeriod' };

  const tokenizeWindows = (pattern) => {
    const tokens = [];
    let index = 0;
    while (index < pattern.length) {
      const character = pattern[index];
      if (character === "'") {
        const end = pattern.indexOf("'", index + 1);
        const text = pattern.slice(index + 1, end < 0 ? pattern.length : end);
        tokens.push({ literal: text || "'" });
        index = end < 0 ? pattern.length : end + 1;
        continue;
      }
      if (!WINDOWS_FIELDS[character]) {
        tokens.push({ literal: character });
        index++;
        continue;
      }
      let count = 1;
      while (pattern[index + count] === character) count++;
      index += count;
      let field = WINDOWS_FIELDS[character];
      let width;
      if (character === 'd' && count >= 3) {
        field = 'weekday';
        width = count === 3 ? 'short' : 'long';
      } else if (character === 'M') width = ['numeric', '2-digit', 'short', 'long'][Math.min(count, 4) - 1];
      else if (character === 'y') width = count === 2 ? '2-digit' : 'numeric';
      else if (character === 'g' || character === 't') width = 'short';
      else width = count >= 2 ? '2-digit' : 'numeric';
      tokens.push({ field, width, cycle: character === 'h' ? 'h12' : character === 'H' ? 'h23' : undefined });
    }
    return tokens;
  };

  const pad = (value, width) => (width === '2-digit' ? String(value % 100).padStart(2, '0') : String(value));

  const formatToken = (token, fields, data, state, pattern) => {
    switch (token.field) {
      case 'era':
        return fields.year > 0 ? (token.width === 'long' ? 'Anno Domini' : token.width === 'narrow' ? 'A' : 'AD') : token.width === 'long' ? 'Before Christ' : token.width === 'narrow' ? 'B' : 'BC';
      case 'year':
        return token.width === '2-digit' ? String(Math.abs(fields.year) % 100).padStart(2, '0') : String(fields.year > 0 ? fields.year : 1 - fields.year);
      case 'month': {
        if (token.width === 'numeric' || token.width === '2-digit') return pad(fields.month + 1, token.width);
        if (token.width === 'narrow') return data.months[fields.month][0];
        const withDay = !token.standalone && pattern.some((other) => other.field === 'day');
        if (token.width === 'long') return withDay ? data.monthsGenitive[fields.month] : data.months[fields.month];
        return data.monthsShort[fields.month];
      }
      case 'day':
        return pad(fields.day, token.width);
      case 'weekday':
        return (token.width === 'long' ? data.weekdays : token.width === 'narrow' ? data.weekdaysNarrow : data.weekdaysShort)[fields.weekday];
      case 'dayPeriod':
        return fields.hour < 12 ? data.am : data.pm;
      case 'hour': {
        const cycle = state.hourCycle || token.cycle;
        let hour = fields.hour;
        if (cycle === 'h12') hour = hour % 12 || 12;
        else if (cycle === 'h11') hour %= 12;
        else if (cycle === 'h24') hour = hour || 24;
        return pad(hour, token.width);
      }
      case 'minute':
        return pad(fields.minute, token.width);
      case 'second':
        return pad(fields.second, token.width);
      case 'fractionalSecond':
        return String(fields.millisecond).padStart(3, '0').slice(0, token.width);
      default:
        return zoneName(state.zone, fields.offset, token.width);
    }
  };

  const englishDate = (o) => {
    const textMonth = ['long', 'short', 'narrow'].includes(o.month);
    const M = { numeric: 'M', '2-digit': 'MM', short: 'MMM', long: 'MMMM', narrow: 'MMMMM' }[o.month] || '';
    const d = o.day === '2-digit' ? 'dd' : o.day ? 'd' : '';
    const y = o.year === '2-digit' ? 'yy' : o.year ? 'y' : '';
    let pattern;
    if (textMonth) {
      if (d && y) pattern = `${M} ${d}, ${y}`;
      else if (y) pattern = `${M} ${y}`;
      else if (d) pattern = `${M} ${d}`;
      else pattern = M.replace(/M/g, 'L');
    } else if (M) pattern = [M, d, y].filter(Boolean).join('/');
    else pattern = d && y ? `${d}, ${y}` : d || y;
    const E = { long: 'EEEE', short: 'EEE', narrow: 'EEEEE' }[o.weekday];
    if (E) pattern = pattern ? `${E}, ${pattern}` : E.replace(/E/g, 'c');
    const G = { long: 'GGGG', short: 'G', narrow: 'GGGGG' }[o.era];
    if (G) pattern = pattern ? `${pattern} ${G}` : G;
    return pattern;
  };

  const englishTime = (o, cycle) => {
    const twelve = cycle === 'h12' || cycle === 'h11';
    const letter = { h12: 'h', h11: 'K', h23: 'H', h24: 'k' }[cycle];
    const parts = [];
    if (o.hour) parts.push(twelve && o.hour === 'numeric' ? letter : letter + letter);
    if (o.minute) parts.push(o.hour || o.second || o.minute === '2-digit' ? 'mm' : 'm');
    if (o.second) parts.push(o.hour || o.minute || o.second === '2-digit' ? 'ss' : 's');
    let pattern = parts.join(':');
    if (o.fractionalSecondDigits) pattern += `${pattern ? '.' : ''}${'S'.repeat(o.fractionalSecondDigits)}`;
    if (o.hour && twelve) pattern += ' a';
    const zoneLetters = { short: 'z', long: 'zzzz', shortGeneric: 'v', longGeneric: 'vvvv', shortOffset: 'O', longOffset: 'OOOO' };
    if (o.timeZoneName) pattern += ` ${zoneLetters[o.timeZoneName]}`;
    return pattern;
  };

  const STYLE_DATES = { full: 'EEEE, MMMM d, y', long: 'MMMM d, y', medium: 'MMM d, y', short: 'M/d/yy' };
  const STYLE_TIMES = { full: 'h:mm:ss a zzzz', long: 'h:mm:ss a z', medium: 'h:mm:ss a', short: 'h:mm a' };

  // Removes a field and a separator next to it from a Windows picture.
  const withoutField = (tokens, field) => {
    const index = tokens.findIndex((token) => token.field === field);
    if (index < 0) return tokens;
    const result = tokens.slice();
    const after = result[index + 1] && result[index + 1].literal !== undefined;
    const before = result[index - 1] && result[index - 1].literal !== undefined;
    if (after && (index === 0 || !before)) {
      let end = index + 1;
      while (result[end] && result[end].literal !== undefined) end++;
      result.splice(index, end - index);
    } else if (before) {
      let start = index - 1;
      while (start > 0 && result[start - 1].literal !== undefined) start--;
      result.splice(start, index - start + 1);
    } else result.splice(index, 1);
    return result;
  };

  // CLDR's numeric year-month-day pictures (as Windows pictures) for locales
  // whose Windows short date differs.
  const NUMERIC_DATES = {
    de: 'd.M.yyyy', fr: 'dd/MM/yyyy', es: 'd/M/yyyy', it: 'd/M/yyyy', pt: 'dd/MM/yyyy', nl: 'd-M-yyyy', ru: 'dd.MM.yyyy',
    ja: 'yyyy/M/d', zh: 'yyyy/M/d', 'zh-TW': 'yyyy/M/d', ko: "yyyy'. 'M'. 'd'.'", pl: 'd.MM.yyyy', sv: 'yyyy-MM-dd', tr: 'dd.MM.yyyy',
    'en-GB': 'dd/MM/yyyy', 'en-AU': 'dd/MM/yyyy', 'en-NZ': 'd/MM/yyyy', 'en-IE': 'd/M/yyyy', 'en-CA': 'yyyy-MM-dd', 'en-IN': 'd/M/yyyy', 'en-ZA': 'yyyy/MM/dd',
    fi: 'd.M.yyyy', da: 'd.M.yyyy', nb: 'd.M.yyyy', cs: "d'. 'M'. 'yyyy", uk: 'dd.MM.yyyy', he: 'd.M.yyyy', vi: 'd/M/yyyy', id: 'd/M/yyyy',
  };
  const numericDate = (data) => NUMERIC_DATES[data.locale] || NUMERIC_DATES[language(data.locale)] || data.shortDate || 'M/d/yyyy';

  const windowsDate = (o, data) => {
    const textMonth = ['long', 'short', 'narrow'].includes(o.month);
    let tokens;
    if (textMonth || (o.weekday && o.month)) {
      const picture = o.year && o.day ? data.longDate : o.year ? data.yearMonth : o.day ? data.monthDay : 'MMMM';
      tokens = tokenizeWindows(picture || 'MMMM d, yyyy');
      if (!o.weekday) tokens = withoutField(tokens, 'weekday');
      if (!o.year) tokens = withoutField(tokens, 'year');
      if (!o.day) tokens = withoutField(tokens, 'day');
    } else if (o.month || o.day || o.year) {
      tokens = tokenizeWindows(numericDate(data));
      for (const field of ['year', 'month', 'day']) if (!o[field]) tokens = withoutField(tokens, field);
      if (o.weekday) tokens = [{ field: 'weekday', width: o.weekday }, { literal: ', ' }, ...tokens];
    } else tokens = o.weekday ? [{ field: 'weekday', width: o.weekday }] : [];
    return tokens.map((token) => {
      if (!token.field) return token;
      if (token.field === 'weekday' && o.weekday) return { ...token, width: o.weekday };
      if (token.field === 'month' && o.month) {
        // A numeric month in a long date, as in "yyyy'年'M'月'd'日'", stays numeric.
        const numeric = token.width === 'numeric' || token.width === '2-digit';
        if (textMonth) return numeric ? token : { ...token, width: o.month };
        return { ...token, width: o.month === '2-digit' ? '2-digit' : numeric ? token.width : o.month };
      }
      if (token.field === 'year' && o.year) return { ...token, width: o.year };
      if (token.field === 'day' && o.day === '2-digit') return { ...token, width: '2-digit' };
      return token;
    });
  };

  const windowsTime = (o, data, cycle) => {
    let tokens = tokenizeWindows(o.second ? data.time : data.shortTime);
    if (!o.second) tokens = withoutField(tokens, 'second');
    if (!o.minute) tokens = withoutField(tokens, 'minute');
    if (!o.hour) tokens = withoutField(withoutField(tokens, 'hour'), 'dayPeriod');
    const twelve = cycle === 'h12' || cycle === 'h11';
    const hasPeriod = tokens.some((token) => token.field === 'dayPeriod');
    tokens = tokens.map((token) => (token.field === 'hour' ? { ...token, cycle } : token));
    if (twelve && o.hour && !hasPeriod) tokens.push({ literal: ' ' }, { field: 'dayPeriod', width: 'short' });
    if (!twelve && hasPeriod) tokens = withoutField(tokens, 'dayPeriod');
    if (o.fractionalSecondDigits) {
      const index = tokens.findIndex((token) => token.field === 'second');
      tokens.splice(index + 1, 0, { literal: data.decimal }, { field: 'fractionalSecond', width: o.fractionalSecondDigits });
    }
    if (o.timeZoneName) tokens.push({ literal: ' ' }, { field: 'timeZoneName', width: o.timeZoneName });
    return tokens;
  };

  const DATE_FIELDS = ['weekday', 'era', 'year', 'month', 'day'];
  const TIME_FIELDS = ['dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits'];
  const FIELD_VALUES = {
    weekday: ['narrow', 'short', 'long'], era: ['narrow', 'short', 'long'], year: ['2-digit', 'numeric'],
    month: ['2-digit', 'numeric', 'narrow', 'short', 'long'], day: ['2-digit', 'numeric'], dayPeriod: ['narrow', 'short', 'long'],
    hour: ['2-digit', 'numeric'], minute: ['2-digit', 'numeric'], second: ['2-digit', 'numeric'],
    timeZoneName: ['short', 'long', 'shortOffset', 'longOffset', 'shortGeneric', 'longGeneric'],
  };
  const STYLES = ['full', 'long', 'medium', 'short'];

  function createDateTimeFormat(target, locales, options, required, defaults) {
    const { locale, keywords, data } = resolveLocale(locales);
    options = toOptions(options);
    const hour12 = options.hour12 === undefined ? undefined : Boolean(options.hour12);
    const hourCycleOption = getOption(options, 'hourCycle', ['h11', 'h12', 'h23', 'h24'], keywords.hc);
    const zone = resolveZone(options.timeZone);
    const fields = {};
    for (const name of [...DATE_FIELDS, 'dayPeriod', 'hour', 'minute', 'second', 'timeZoneName']) {
      fields[name] = getOption(options, name, FIELD_VALUES[name], undefined);
    }
    fields.fractionalSecondDigits = getNumber(options, 'fractionalSecondDigits', 1, 3, undefined);
    const dateStyle = getOption(options, 'dateStyle', STYLES, undefined);
    const timeStyle = getOption(options, 'timeStyle', STYLES, undefined);
    const explicit = Object.keys(fields).some((name) => fields[name] !== undefined && name !== 'timeZoneName');
    if ((dateStyle || timeStyle) && (explicit || fields.timeZoneName)) {
      throw new TypeError(`Can't set option ${Object.keys(fields).find((name) => fields[name] !== undefined)} when ${dateStyle ? 'dateStyle' : 'timeStyle'} is used`);
    }
    if (required === 'date' && timeStyle && !dateStyle) throw new TypeError('Invalid option : timeStyle');
    if (required === 'time' && dateStyle && !timeStyle) throw new TypeError('Invalid option : dateStyle');
    if (!dateStyle && !timeStyle) {
      const needDate = required !== 'time' && !DATE_FIELDS.some((name) => fields[name]);
      const needTime = required !== 'date' && !TIME_FIELDS.some((name) => fields[name]);
      const nothing = needDate && needTime;
      if (nothing || (required === 'date' && needDate)) {
        if (defaults === 'date' || defaults === 'all') Object.assign(fields, { year: 'numeric', month: 'numeric', day: 'numeric' });
      }
      if (nothing || (required === 'time' && needTime)) {
        if (defaults === 'time' || defaults === 'all') Object.assign(fields, { hour: 'numeric', minute: 'numeric', second: 'numeric' });
      }
    }
    let hourCycle = data.hourCycle;
    if (hour12 !== undefined) hourCycle = hour12 ? (data.hourCycle === 'h11' ? 'h11' : 'h12') : 'h23';
    else if (hourCycleOption) hourCycle = hourCycleOption;
    const usesHour = Boolean(fields.hour || timeStyle);
    let tokens;
    if (data.english) {
      let pattern;
      if (dateStyle || timeStyle) {
        let time = timeStyle && STYLE_TIMES[timeStyle];
        if (time && (hourCycle === 'h23' || hourCycle === 'h24')) time = time.replace('h', 'HH').replace(' a', '');
        const date = dateStyle && STYLE_DATES[dateStyle];
        pattern = date && time ? `${date}${dateStyle === 'full' || dateStyle === 'long' ? ' \'at\' ' : ', '}${time}` : date || time;
      } else {
        const date = englishDate(fields);
        const time = englishTime(fields, hourCycle);
        pattern = date && time ? `${date}, ${time}` : date || time;
      }
      tokens = tokenizeCldrQuoted(pattern);
    } else {
      const styleFields = (style) => ({
        full: { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' },
        long: { year: 'numeric', month: 'long', day: 'numeric' },
        medium: { year: 'numeric', month: 'short', day: 'numeric' },
        short: { year: '2-digit', month: '2-digit', day: '2-digit' },
      })[style];
      const dateFields = dateStyle ? styleFields(dateStyle) : fields;
      const timeFields = timeStyle
        ? { hour: 'numeric', minute: 'numeric', second: timeStyle === 'short' ? undefined : 'numeric', timeZoneName: timeStyle === 'full' ? 'long' : timeStyle === 'long' ? 'short' : undefined }
        : fields;
      const date = DATE_FIELDS.some((name) => dateFields[name]) ? windowsDate(dateFields, data) : [];
      const time = TIME_FIELDS.some((name) => timeFields[name]) ? windowsTime(timeFields, data, hourCycle) : [];
      // CLDR joins dates and times with a comma in these languages.
      const comma = ['en', 'de', 'es', 'it', 'ru', 'pl', 'nl', 'pt', 'uk', 'cs', 'tr'].includes(language(locale));
      const glue = comma && dateStyle !== 'full' && dateStyle !== 'long' ? ', ' : ' ';
      tokens = date.length && time.length ? [...date, { literal: glue }, ...time] : [...date, ...time];
    }
    define(target, '_state', {
      locale,
      data,
      zone,
      tokens,
      hourCycle: usesHour ? hourCycle : undefined,
      fields,
      dateStyle,
      timeStyle,
      calendar: 'gregory',
      numberingSystem: keywords.nu || 'latn',
    });
  }

  // CLDR patterns with quoted literals such as 'at'.
  const tokenizeCldrQuoted = (pattern) => {
    const tokens = [];
    pattern.split(/('[^']*')/).forEach((piece) => {
      if (piece.startsWith("'")) tokens.push({ literal: piece.slice(1, -1) });
      else tokens.push(...tokenizeCldr(piece));
    });
    return tokens;
  };

  const toDate = (value) => {
    const date = value === undefined ? new Date() : value instanceof Date ? value : new Date(Number(value));
    if (Number.isNaN(date.getTime())) throw new RangeError('Invalid time value');
    return date;
  };

  function dateParts(state, value) {
    const date = toDate(value);
    const fields = fieldsOf(date, state.zone);
    const parts = [];
    for (const token of state.tokens) {
      if (token.literal !== undefined) {
        const last = parts[parts.length - 1];
        if (last && last.type === 'literal') last.value += token.literal;
        else parts.push({ type: 'literal', value: token.literal });
      } else {
        parts.push({ type: token.field === 'era' ? 'era' : token.field, value: formatToken(token, fields, state.data, state, state.tokens) });
      }
    }
    return parts;
  }

  class DateTimeFormat {
    constructor(locales, options) {
      createDateTimeFormat(this, locales, options, 'any', 'date');
      define(this, 'format', (value) => this.formatToParts(value).map((part) => part.value).join(''));
    }

    formatToParts(value) {
      return dateParts(this._state, value);
    }

    formatRange(start, end) {
      if (start === undefined || end === undefined) throw new TypeError('startDate or endDate is undefined');
      const first = this.format(start);
      const second = this.format(end);
      return first === second ? first : `${first} – ${second}`;
    }

    formatRangeToParts(start, end) {
      const first = this.formatToParts(start).map((part) => ({ ...part, source: 'startRange' }));
      const second = this.formatToParts(end).map((part) => ({ ...part, source: 'endRange' }));
      if (first.map((part) => part.value).join('') === second.map((part) => part.value).join('')) return first.map((part) => ({ ...part, source: 'shared' }));
      return [...first, { type: 'literal', value: ' – ', source: 'shared' }, ...second];
    }

    resolvedOptions() {
      const state = this._state;
      const result = { locale: state.locale, calendar: state.calendar, numberingSystem: state.numberingSystem, timeZone: state.zone.name };
      if (state.hourCycle) {
        result.hourCycle = state.hourCycle;
        result.hour12 = state.hourCycle === 'h11' || state.hourCycle === 'h12';
      }
      for (const name of ['weekday', 'era', 'year', 'month', 'day', 'dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits', 'timeZoneName']) {
        let value = state.fields[name];
        if (value === undefined) continue;
        if ((name === 'minute' || name === 'second') && state.fields.hour) value = '2-digit';
        if (name === 'hour' && (state.hourCycle === 'h23' || state.hourCycle === 'h24') && state.data.english) value = '2-digit';
        result[name] = value;
      }
      if (state.dateStyle) result.dateStyle = state.dateStyle;
      if (state.timeStyle) result.timeStyle = state.timeStyle;
      return result;
    }
  }
  tag(DateTimeFormat.prototype, 'Intl.DateTimeFormat');

  // Display names: English names of common languages, regions and currencies.
  const LANGUAGE_NAMES = { ar: 'Arabic', de: 'German', en: 'English', es: 'Spanish', fr: 'French', he: 'Hebrew', hi: 'Hindi', it: 'Italian', ja: 'Japanese', ko: 'Korean', nl: 'Dutch', pl: 'Polish', pt: 'Portuguese', ru: 'Russian', sv: 'Swedish', th: 'Thai', tr: 'Turkish', uk: 'Ukrainian', vi: 'Vietnamese', zh: 'Chinese', 'en-US': 'American English', 'en-GB': 'British English', 'zh-Hans': 'Simplified Chinese', 'zh-Hant': 'Traditional Chinese', 'pt-BR': 'Brazilian Portuguese' };
  const REGION_NAMES = { AU: 'Australia', BR: 'Brazil', CA: 'Canada', CH: 'Switzerland', CN: 'China', DE: 'Germany', ES: 'Spain', FR: 'France', GB: 'United Kingdom', HK: 'Hong Kong SAR China', IN: 'India', IT: 'Italy', JP: 'Japan', KR: 'South Korea', MX: 'Mexico', NL: 'Netherlands', NZ: 'New Zealand', RU: 'Russia', SE: 'Sweden', SG: 'Singapore', TW: 'Taiwan', US: 'United States' };

  class DisplayNames {
    constructor(locales, options) {
      const { locale } = resolveLocale(locales);
      options = toOptions(options);
      const type = getOption(options, 'type', ['language', 'region', 'script', 'currency', 'calendar', 'dateTimeField'], undefined);
      if (type === undefined) throw new TypeError('Required option \'type\' is missing');
      const style = getOption(options, 'style', ['narrow', 'short', 'long'], 'long');
      const fallback = getOption(options, 'fallback', ['code', 'none'], 'code');
      define(this, '_state', { locale, type, style, fallback });
    }

    of(code) {
      const { type, fallback } = this._state;
      const text = String(code);
      let name;
      if (type === 'language') name = LANGUAGE_NAMES[text] || LANGUAGE_NAMES[text.split('-')[0]];
      else if (type === 'region') name = REGION_NAMES[text.toUpperCase()];
      else if (type === 'currency') name = CURRENCIES[text.toUpperCase()] && CURRENCIES[text.toUpperCase()][2].replace(/^./, (first) => first.toUpperCase());
      if (name) return name;
      return fallback === 'code' ? (type === 'region' || type === 'currency' ? text.toUpperCase() : text) : undefined;
    }

    resolvedOptions() {
      const { locale, type, style, fallback } = this._state;
      return { locale, style, type, fallback };
    }
  }
  tag(DisplayNames.prototype, 'Intl.DisplayNames');

  // The Intl object and the locale-sensitive methods of built-ins. -------------------

  const supportedLocalesOf = (locales) => canonicalLocales(locales).filter((locale) => localeData(splitTag(locale).base) || localeData(language(locale)));
  for (const constructor of [NumberFormat, DateTimeFormat, PluralRules, RelativeTimeFormat, ListFormat, Collator, Segmenter, DisplayNames]) {
    define(constructor, 'supportedLocalesOf', supportedLocalesOf);
  }

  // NumberFormat, DateTimeFormat and Collator may be called without `new`.
  const callable = (Class) => {
    const constructor = function (locales, options) {
      return new Class(locales, options);
    };
    Object.defineProperty(constructor, 'name', { value: Class.name });
    constructor.prototype = Class.prototype;
    define(Class.prototype, 'constructor', constructor);
    define(constructor, 'supportedLocalesOf', supportedLocalesOf);
    return constructor;
  };

  const Intl = {};
  defineAll(Intl, {
    NumberFormat: callable(NumberFormat),
    DateTimeFormat: callable(DateTimeFormat),
    Collator: callable(Collator),
    PluralRules, RelativeTimeFormat, ListFormat, Segmenter, Locale, DisplayNames,
    getCanonicalLocales: (locales) => canonicalLocales(locales),
    supportedValuesOf(key) {
      switch (String(key)) {
        case 'calendar':
          return ['gregory'];
        case 'collation':
          return ['default'];
        case 'currency':
          return Object.keys(CURRENCIES).sort();
        case 'numberingSystem':
          return ['latn'];
        case 'timeZone':
          return [...new Set([...IANA_ZONES.values()].map((zone) => zone.name).filter((name) => name.includes('/') && !name.startsWith('Etc/')))].sort();
        case 'unit':
          return SANCTIONED_UNITS.slice().sort();
        default:
          throw new RangeError(`Invalid key : ${key}`);
      }
    },
  });
  tag(Intl, 'Intl');
  define(global, 'Intl', Intl);

  const dateMethod = (required, defaults) =>
    function (locales, options) {
      if (Number.isNaN(this.getTime())) return 'Invalid Date';
      const format = Object.create(DateTimeFormat.prototype);
      createDateTimeFormat(format, locales, options, required, defaults);
      return dateParts(format._state, this).map((part) => part.value).join('');
    };
  defineAll(Date.prototype, {
    toLocaleString: dateMethod('any', 'all'),
    toLocaleDateString: dateMethod('date', 'date'),
    toLocaleTimeString: dateMethod('time', 'time'),
  });
  let defaultNumberFormat = null;
  defineAll(Number.prototype, {
    toLocaleString(locales, options) {
      const value = Number.prototype.valueOf.call(this);
      if (locales === undefined && options === undefined) {
        defaultNumberFormat = defaultNumberFormat || new NumberFormat();
        return defaultNumberFormat.format(value);
      }
      return new NumberFormat(locales, options).format(value);
    },
  });
  if (typeof BigInt === 'function') {
    defineAll(BigInt.prototype, {
      toLocaleString(locales, options) {
        return new NumberFormat(locales, options).format(BigInt.prototype.valueOf.call(this));
      },
    });
  }
  let defaultCollator = null;
  const upper = String.prototype.toUpperCase;
  const lower = String.prototype.toLowerCase;
  const turkic = (locales) => {
    try {
      return ['tr', 'az'].includes(language(canonicalLocales(locales)[0] || ''));
    } catch {
      return false;
    }
  };
  defineAll(String.prototype, {
    localeCompare(that, locales, options) {
      if (this === null || this === undefined) throw new TypeError('String.prototype.localeCompare called on null or undefined');
      if (locales === undefined && options === undefined) {
        defaultCollator = defaultCollator || new Collator();
        return defaultCollator.compare(String(this), String(that));
      }
      return new Collator(locales, options).compare(String(this), String(that));
    },
    toLocaleUpperCase(locales) {
      const text = String(this);
      return upper.call(turkic(locales) ? text.replace(/i/g, 'İ') : text);
    },
    toLocaleLowerCase(locales) {
      const text = String(this);
      return lower.call(turkic(locales) ? text.replace(/I/g, 'ı').replace(/İ/g, 'i') : text);
    },
  });
  defineAll(Array.prototype, {
    toLocaleString(locales, options) {
      return Array.prototype.map.call(this, (item) => (item === null || item === undefined ? '' : item.toLocaleString(locales, options))).join(',');
    },
  });
})
