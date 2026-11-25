const out = [];
globalThis.__electronLog = null;
__electronLog = { level: 'native' };
out.push(__electronLog.level);
let effects = 0;
try { missingWrite = (effects++, 1); } catch (error) { out.push(error.name, effects); }
freshWrite = (globalThis.freshWrite = 0, 7);
out.push(freshWrite);
globalThis.toDelete = 1;
try { toDelete = (delete globalThis.toDelete, 2); } catch (error) { out.push(error.name, typeof toDelete); }
globalThis.counter = 1;
out.push(counter++, ++counter, counter += 4, counter ||= 99, counter &&= 8, counter ??= 9);
globalThis.destructured = 0;
({ destructured } = { destructured: 11 });
out.push(destructured);
({ defaultWrite = (globalThis.defaultWrite = 1, 12) } = {});
out.push(defaultWrite);
[defaultArray = (globalThis.defaultArray = 0, 13)] = [];
out.push(defaultArray);
Object.defineProperty(globalThis, 'fixedGlobal', { value: 3, writable: false, configurable: true });
try { fixedGlobal = 10; } catch (error) { out.push(error.name, fixedGlobal); }
export const result = out.join('|');
