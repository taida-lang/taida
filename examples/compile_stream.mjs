
// taida-runtime.js — Taida Lang JavaScript Runtime

function __taida_debug(...args) {
  for (const arg of args) {
    if (arg && arg.__type) {
      console.log(arg.__type + '(' + JSON.stringify(arg) + ')');
    } else if (Array.isArray(arg)) {
      console.log('@[' + arg.map(x => typeof x === 'string' ? '"' + x + '"' : String(x)).join(', ') + ']');
    } else {
      console.log(typeof arg === 'boolean' ? (arg ? 'true' : 'false') : String(arg));
    }
  }
}

function __taida_ensureNotNull(value, defaultValue) {
  return (value === null || value === undefined) ? defaultValue : value;
}

function __taida_defaultValue(typeName) {
  switch (typeName) {
    case 'Int': return 0;
    case 'Float': return 0.0;
    case 'Str': return '';
    case 'Bool': return false;
    default: return Object.freeze({});
  }
}

function __taida_buchiPack(fields) {
  return Object.freeze(fields);
}

function __taida_list(items) {
  return Object.freeze([...items]);
}

class __TaidaError extends globalThis.Error {
  constructor(type, message, fields) {
    super(message);
    this.type = type;
    this.fields = fields || {};
  }
}

// Standalone throw function (no Object.prototype pollution)
function __taida_throw(obj) {
  throw obj instanceof globalThis.Error ? obj : new __TaidaError(obj.type || 'Error', obj.message || '', obj);
}

// Taida Error base type (not JS Error constructor)
// Error = @(type: Str, message: Str)
function Error(fields) {
  const obj = {
    __type: 'Error',
    type: __taida_ensureNotNull(fields && fields.type, ''),
    message: __taida_ensureNotNull(fields && fields.message, ''),
  };
  return Object.freeze(obj);
}

// ── Async[T] — Promise-based (thenable) ─────────────────
// __TaidaAsync is a thenable: it implements .then() so that
// `await asyncObj` resolves to the inner value or rejects with the error.
// This enables ]=> to map to `await` in generated JS code.
class __TaidaAsync {
  constructor(value, error, status) {
    this.__type = 'Async';
    this.status = status || 'fulfilled';
    this.__value = value;
    this.__error = error;
  }
  // Thenable protocol — makes `await asyncObj` work
  then(resolve, reject) {
    if (this.status === 'rejected') {
      if (reject) reject(this.__error);
    } else {
      if (resolve) resolve(this.__value);
    }
  }
  unmold() {
    if (this.status === 'rejected') throw this.__error;
    return this.__value;
  }
  isPending() { return this.status === 'pending'; }
  isFulfilled() { return this.status === 'fulfilled'; }
  isRejected() { return this.status === 'rejected'; }
  map(fn) {
    if (this.status !== 'fulfilled') return this;
    return new __TaidaAsync(fn(this.__value), null, 'fulfilled');
  }
  getOrDefault(def) {
    if (this.status === 'fulfilled') return this.__value;
    return def;
  }
  toString() {
    if (this.status === 'fulfilled') return 'Async[fulfilled: ' + String(this.__value) + ']';
    if (this.status === 'rejected') return 'Async[rejected: ' + String(this.__error) + ']';
    return 'Async[pending]';
  }
}

function Async(value) {
  return new __TaidaAsync(value, null, 'fulfilled');
}

function AsyncReject(error) {
  return new __TaidaAsync(null, error, 'rejected');
}

// ── Async aggregation — sync/async hybrid ───────────────
// When all inputs are __TaidaAsync (sync thenables), process synchronously.
// When true async Promises are present, use Promise.all/race.
function All(asyncList) {
  // Fast path: all __TaidaAsync (synchronous)
  const allSync = asyncList.every(item => item instanceof __TaidaAsync);
  if (allSync) {
    const values = [];
    for (const item of asyncList) {
      if (item.status === 'rejected') throw item.__error;
      values.push(item.__value);
    }
    return Object.freeze(values);
  }
  // Async path: true Promises present — return a Promise
  return Promise.all(asyncList).then(results => Object.freeze(results));
}

function Race(asyncList) {
  if (asyncList.length === 0) return undefined;
  // Fast path: all __TaidaAsync (synchronous)
  const allSync = asyncList.every(item => item instanceof __TaidaAsync);
  if (allSync) {
    const first = asyncList[0];
    if (first.status === 'rejected') throw first.__error;
    return first.__value;
  }
  // Async path
  return Promise.race(asyncList);
}

function Timeout(asyncVal, ms) {
  // If sync __TaidaAsync, return its value directly
  if (asyncVal instanceof __TaidaAsync) {
    if (asyncVal.status === 'rejected') throw asyncVal.__error;
    return asyncVal.__value;
  }
  // Async path: race against timeout
  return Promise.race([
    asyncVal,
    new Promise((_, reject) => setTimeout(() => reject(new __TaidaError('TimeoutError', 'timeout', {})), ms))
  ]);
}

// ── JSON type — opaque (Molten Iron) ────────────────────
// JSON is opaque. No methods. Must be cast through schema: JSON[raw, Schema]()
class __TaidaJSON {
  constructor(v) { this.__type = 'JSON'; this.__value = v; }
}

// TypeDef registry for JSON schema resolution
const __taida_typeDefs = {};
function __taida_registerTypeDef(name, fieldDefs) {
  __taida_typeDefs[name] = fieldDefs;
}

function __taidaValueToJson(v) {
  if (v instanceof __TaidaJSON) return v.__value;
  if (Array.isArray(v)) return v.map(__taidaValueToJson);
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    const result = {};
    for (const [k, val] of Object.entries(v)) result[k] = __taidaValueToJson(val);
    return result;
  }
  return v;
}

// Schema-based JSON casting: JSON[raw, Schema]() -> Lax
function JSON_mold(rawValue, schema) {
  // Parse raw data
  let jsonData;
  if (rawValue instanceof __TaidaJSON) {
    jsonData = rawValue.__value;
  } else if (typeof rawValue === 'string') {
    try { jsonData = JSON.parse(rawValue); }
    catch (e) {
      const defaultVal = __taida_defaultForSchema(schema);
      return Object.freeze({ __type: 'Lax', hasValue: __taida_hasValue(false), __value: defaultVal, __default: defaultVal, __error: 'JSON parse error: ' + e.message });
    }
  } else {
    jsonData = __taidaValueToJson(rawValue);
  }

  // Cast through schema
  const typedValue = __taida_castJson(jsonData, schema);
  const defaultVal = __taida_defaultForSchema(schema);
  return Object.freeze({ __type: 'Lax', hasValue: __taida_hasValue(true), __value: typedValue, __default: defaultVal });
}

function __taida_castJson(json, schema) {
  if (typeof schema === 'string') {
    switch (schema) {
      case 'Int': return typeof json === 'number' ? Math.trunc(json) : (typeof json === 'string' ? (parseInt(json, 10) || 0) : 0);
      case 'Float': return typeof json === 'number' ? json : (typeof json === 'string' ? (parseFloat(json) || 0.0) : 0.0);
      case 'Str': return typeof json === 'string' ? json : (json === null || json === undefined ? '' : (typeof json === 'object' ? JSON.stringify(json) : String(json)));
      case 'Bool': return typeof json === 'boolean' ? json : false;
      default: {
        // TypeDef lookup
        const td = __taida_typeDefs[schema];
        if (!td || typeof json !== 'object' || json === null || Array.isArray(json)) {
          return __taida_defaultForSchema(schema);
        }
        const result = { __type: schema };
        for (const [fname, fschema] of Object.entries(td)) {
          if (fname in json && json[fname] !== null && json[fname] !== undefined) {
            result[fname] = __taida_castJson(json[fname], fschema);
          } else {
            result[fname] = __taida_defaultForSchema(fschema);
          }
        }
        return Object.freeze(result);
      }
    }
  }
  if (schema && schema.__list) {
    if (!Array.isArray(json)) return Object.freeze([]);
    return Object.freeze(json.map(item => __taida_castJson(item, schema.__list)));
  }
  // Inline BuchiPack schema: { field1: schema1, field2: schema2 }
  if (schema && typeof schema === 'object' && !Array.isArray(schema)) {
    if (typeof json !== 'object' || json === null || Array.isArray(json)) {
      return __taida_defaultForSchema(schema);
    }
    const result = {};
    for (const [fname, fschema] of Object.entries(schema)) {
      if (fname in json && json[fname] !== null && json[fname] !== undefined) {
        result[fname] = __taida_castJson(json[fname], fschema);
      } else {
        result[fname] = __taida_defaultForSchema(fschema);
      }
    }
    return Object.freeze(result);
  }
  return '';
}

function __taida_defaultForSchema(schema) {
  if (typeof schema === 'string') {
    switch (schema) {
      case 'Int': return 0;
      case 'Float': return 0.0;
      case 'Str': return '';
      case 'Bool': return false;
      default: {
        const td = __taida_typeDefs[schema];
        if (!td) return '';
        const result = { __type: schema };
        for (const [fname, fschema] of Object.entries(td)) {
          result[fname] = __taida_defaultForSchema(fschema);
        }
        return Object.freeze(result);
      }
    }
  }
  if (schema && schema.__list) return Object.freeze([]);
  // Inline BuchiPack schema: { field1: schema1, field2: schema2 }
  if (schema && typeof schema === 'object' && !Array.isArray(schema)) {
    const result = {};
    for (const [fname, fschema] of Object.entries(schema)) {
      result[fname] = __taida_defaultForSchema(fschema);
    }
    return Object.freeze(result);
  }
  return '';
}

// ── Optional — ABOLISHED (v0.8.0) ────────────────────────
// Optional has been removed. Use Lax[value]() instead.
function Optional() { throw new Error('Optional has been removed. Use Lax[value]() instead. Lax[T] provides the same safety with default value guarantees.'); }

// ── Some() / None() — ABOLISHED ──────────────────────────
function Some(_) { throw new Error('Some() has been removed. Optional is abolished. Use Lax[value]() instead.'); }
function None() { throw new Error('None() has been removed. Optional is abolished. Use Lax[value]() instead.'); }

// ── Result[T, P] (operation mold with predicate + throw field) ──
// Result[value, pred]() → P: :T => :Bool — predicate for success/failure
// Result[value, pred]() => r — predicate unevaluated (stored as __predicate)
// Result[value, pred]() ]=> r — predicate evaluated: true → value T, false → throw
// Result[value]() — backward compatible: no predicate (always success if no throw)
function __taida_result_create(value, throwVal, predicate) {
  const _value = value;
  const _throw = throwVal || null;
  const _pred = (typeof predicate === 'function') ? predicate : null;
  // Determine error state:
  // 1. If throw is explicitly set (not null), it's an error
  // 2. If predicate exists, evaluate P(value) — true = success, false = error
  // 3. No predicate + no throw = success (backward compatible)
  function _checkError() {
    if (_throw !== null && _throw !== undefined) {
      // If predicate exists, evaluate it even when throw is set
      if (_pred) {
        const predResult = _pred(_value);
        if (!predResult) return true; // predicate failed — error
        return false; // predicate passed even though throw was set — success
      }
      return true;
    }
    if (_pred) {
      const predResult = _pred(_value);
      return !predResult;
    }
    return false;
  }
  return Object.freeze({
    __type: 'Result',
    __value: _value,
    __predicate: _pred,
    throw: _throw,
    isSuccess() { return !_checkError(); },
    isError() { return _checkError(); },
    getOrDefault(def) { return _checkError() ? def : _value; },
    map(fn) {
      if (_checkError()) return this;
      return __taida_result_create(fn(_value), null, null);
    },
    flatMap(fn) {
      if (_checkError()) return this;
      const result = fn(_value);
      if (result && result.__type === 'Result') return result;
      return __taida_result_create(result, null, null);
    },
    mapError(fn) {
      if (!_checkError()) return this;
      const errMsg = _throw && _throw.message ? _throw.message : String(_throw);
      const newMsg = fn(errMsg);
      return __taida_result_create(null, { __type: 'ResultError', message: String(newMsg), type: 'ResultError' }, null);
    },
    getOrThrow() {
      if (!_checkError()) return _value;
      if (_throw && typeof _throw === 'object') {
        throw new __TaidaError(_throw.type || 'ResultError', _throw.message || String(_throw), {});
      }
      if (_throw) {
        throw new __TaidaError('ResultError', String(_throw), {});
      }
      // Predicate failed but no explicit throw — generate default error
      throw new __TaidaError('ResultError', 'Result predicate failed for value: ' + String(_value), {});
    },
    toString() {
      if (!_checkError()) return 'Result(' + String(_value) + ')';
      const errDisplay = _throw && _throw.message ? _throw.message : (_throw ? String(_throw) : 'predicate failed');
      return 'Result(throw <= ' + errDisplay + ')';
    },
    unmold() {
      if (_checkError()) {
        if (_throw && typeof _throw === 'object') {
          throw new __TaidaError(_throw.type || 'ResultError', _throw.message || String(_throw), {});
        }
        if (_throw) throw _throw;
        // Predicate failed but no explicit throw — generate default error
        throw new __TaidaError('ResultError', 'Result predicate failed for value: ' + String(_value), {});
      }
      return _value;
    },
  });
}

// ── Ok() / Err() — ABOLISHED ─────────────────────────────
function Ok(_) { throw new Error('Ok() has been removed. Use Result[value]() instead.'); }
function Err(_) { throw new Error('Err() has been removed. Use Result[value](throw <= error) instead.'); }

// Alias for MoldInst codegen — Result(value, predicate, opts)
function Result(...args) {
  // Result(value, pred, opts) — pred is function, opts may contain throw field
  // Result(value, opts) — backward compat: opts is object with throw field
  // Result(value) — success, no predicate
  let value = args.length > 0 ? args[0] : null;
  let pred = null;
  let throwVal = null;
  let argIdx = 1;
  // Check if second arg is a predicate (function) or options (object)
  if (argIdx < args.length && typeof args[argIdx] === 'function') {
    pred = args[argIdx];
    argIdx++;
  }
  // Check for options object with throw field
  if (argIdx < args.length && args[argIdx] && typeof args[argIdx] === 'object' && 'throw' in args[argIdx]) {
    throwVal = args[argIdx].throw;
    if (throwVal === null || throwVal === undefined) throwVal = null;
  }
  return __taida_result_create(value, throwVal, pred);
}

// ── Lax[T] ───────────────────────────────────────────────
function __taida_lax_default(value) {
  if (typeof value === 'number') return Number.isInteger(value) ? 0 : 0.0;
  if (typeof value === 'string') return '';
  if (typeof value === 'boolean') return false;
  if (Array.isArray(value)) return Object.freeze([]);
  if (value && typeof value === 'object') return Object.freeze({});
  return 0;
}

// Create a callable hasValue that also works as a Boolean-like property
// Allows both `x.hasValue` (field access) and `x.hasValue()` (method call)
function __taida_hasValue(val) {
  const fn = function() { return val; };
  fn.valueOf = function() { return val; };
  fn.toString = function() { return String(val); };
  return fn;
}

function Lax(value, typedDefault) {
  const _hasValue = value !== null && value !== undefined;
  const _default = _hasValue ? __taida_lax_default(value) : (typedDefault !== undefined ? typedDefault : 0);
  const _val = _hasValue ? value : _default;
  return Object.freeze({
    __type: 'Lax',
    __value: _val,
    __default: _default,
    hasValue: __taida_hasValue(_hasValue),
    isEmpty() { return !_hasValue; },
    getOrDefault(def) { return _hasValue ? _val : def; },
    map(fn) { return _hasValue ? Lax(fn(_val)) : this; },
    flatMap(fn) {
      if (!_hasValue) return this;
      const result = fn(_val);
      if (result && result.__type === 'Lax') return result;
      return Lax(result);
    },
    unmold() { return _hasValue ? _val : _default; },
    toString() {
      return _hasValue ? 'Lax(' + String(_val) + ')' : 'Lax(default: ' + String(_default) + ')';
    },
  });
}

// ── Gorillax/Cage Mold types ──
// Gorillax: like Lax but unmold failure = gorilla (program exit)
function Gorillax(value, error) {
  const _hasValue = value !== null && value !== undefined;
  const _error = error || null;
  return Object.freeze({
    __type: 'Gorillax',
    __value: _hasValue ? value : null,
    __error: _error,
    hasValue: __taida_hasValue(_hasValue),
    isEmpty() { return !_hasValue; },
    relax() {
      return Object.freeze({
        __type: 'RelaxedGorillax',
        __value: _hasValue ? value : null,
        __error: _error,
        hasValue: __taida_hasValue(_hasValue),
        isEmpty() { return !_hasValue; },
        toString() {
          return _hasValue ? 'RelaxedGorillax(' + String(value) + ')' : 'RelaxedGorillax(escaped)';
        },
      });
    },
    toString() {
      return _hasValue ? 'Gorillax(' + String(value) + ')' : 'Gorillax(><)';
    },
  });
}

// Cage: execute function in protected context, return Gorillax
function Cage_mold(cageValue, cageFn) {
  try {
    const result = cageFn(cageValue);
    return Gorillax(result, null);
  } catch (e) {
    return Gorillax(null, e);
  }
}

// ── Div/Mod Mold types (safe division returning Lax) ──
function Div_mold(a, b, opts) {
  if (opts === undefined) opts = {};
  const isFloat = typeof a === 'number' && (String(a).includes('.') || typeof b === 'number' && String(b).includes('.'));
  const def = opts.default !== undefined ? opts.default : (isFloat ? 0.0 : 0);
  if (b === 0) return Lax(null, def);
  const result = Number.isInteger(a) && Number.isInteger(b) ? Math.trunc(a / b) : a / b;
  const lax = Lax(result);
  return lax;
}
function Mod_mold(a, b, opts) {
  if (opts === undefined) opts = {};
  const isFloat = typeof a === 'number' && (String(a).includes('.') || typeof b === 'number' && String(b).includes('.'));
  const def = opts.default !== undefined ? opts.default : (isFloat ? 0.0 : 0);
  if (b === 0) return Lax(null, def);
  return Lax(a % b);
}

// ── Type Conversion Mold types (Str/Int/Float/Bool → Lax) ──
function Str_mold(value) {
  return Lax(String(value));
}
function Int_mold(value) {
  if (typeof value === 'number') return Lax(Math.trunc(value));
  if (typeof value === 'boolean') return Lax(value ? 1 : 0);
  if (typeof value === 'string') {
    const n = parseInt(value, 10);
    if (isNaN(n)) return Lax(null, 0);
    return Lax(n);
  }
  return Lax(null, 0);
}
function Float_mold(value) {
  if (typeof value === 'number') return Lax(value);
  if (typeof value === 'boolean') return Lax(value ? 1.0 : 0.0);
  if (typeof value === 'string') {
    const f = parseFloat(value);
    if (isNaN(f)) return Lax(null, 0.0);
    return Lax(f);
  }
  return Lax(null, 0.0);
}
function Bool_mold(value) {
  if (typeof value === 'boolean') return Lax(value);
  if (typeof value === 'number') return Lax(value !== 0);
  if (typeof value === 'string') {
    if (value === 'true') return Lax(true);
    if (value === 'false') return Lax(false);
    return Lax(null, false);
  }
  return Lax(null, false);
}

// ── Stream[T] — time-series mold type ────────────────────
// Stream holds source items + lazy transform chain.
// ]=> (unmold) collects all items, applying transforms.
function __taida_stream(items, transforms) {
  return Object.freeze({ __type: 'Stream', __items: Object.freeze([...items]), __transforms: Object.freeze([...transforms]), __status: 'completed',
    length_() { return this.__status === 'completed' ? this.__items.length : -1; },
    isEmpty() { return this.__items.length === 0 && this.__status === 'completed'; },
    toString() { return this.__status === 'active' ? 'Stream[active]' : 'Stream[completed: ' + this.__items.length + ' items]'; }
  });
}
function Stream_mold(value) { return __taida_stream([value], []); }
function StreamFrom(list) { return __taida_stream(list, []); }

// ── Map/Filter/Fold Mold types (return values directly, like interpreter) ──
// Stream input: append transform (lazy evaluation)
function Map(list, fn) {
  if (list && list.__type === 'Stream') return __taida_stream(list.__items, [...list.__transforms, { op: 'map', fn }]);
  return Object.freeze(list.map(fn));
}
function Filter(list, fn) {
  if (list && list.__type === 'Stream') return __taida_stream(list.__items, [...list.__transforms, { op: 'filter', fn }]);
  return Object.freeze(list.filter(fn));
}
function Fold(list, init, fn) { return list.reduce(fn, init); }
function Reduce(list, init, fn) { return list.reduce(fn, init); }
function Foldr(list, init, fn) { return list.reduceRight(fn, init); }
function Take(list, n) {
  if (list && list.__type === 'Stream') return __taida_stream(list.__items, [...list.__transforms, { op: 'take', n }]);
  return Object.freeze(list.slice(0, n));
}
function TakeWhile(list, fn) {
  if (list && list.__type === 'Stream') return __taida_stream(list.__items, [...list.__transforms, { op: 'takeWhile', fn }]);
  const result = [];
  for (const item of list) { if (fn(item)) result.push(item); else break; }
  return Object.freeze(result);
}
function Drop(list, n) { return Object.freeze(list.slice(n)); }
function DropWhile(list, fn) {
  let dropping = true;
  const result = [];
  for (const item of list) {
    if (dropping && fn(item)) continue;
    dropping = false;
    result.push(item);
  }
  return Object.freeze(result);
}

// ── Stream collect helper (used by __taida_unmold) ───────
function __taida_stream_collect(stream) {
  let items = [...stream.__items];
  for (const t of stream.__transforms) {
    switch (t.op) {
      case 'map': items = items.map(t.fn); break;
      case 'filter': items = items.filter(t.fn); break;
      case 'take': items = items.slice(0, t.n); break;
      case 'takeWhile': {
        const r = [];
        for (const item of items) { if (t.fn(item)) r.push(item); else break; }
        items = r;
        break;
      }
    }
  }
  return Object.freeze(items);
}

// ── String Mold types ───────────────────────────────────
function Upper(str) { return typeof str === 'string' ? str.toUpperCase() : ''; }
function Lower(str) { return typeof str === 'string' ? str.toLowerCase() : ''; }
function Trim(str, opts) {
  if (typeof str !== 'string') return '';
  if (!opts) return str.trim();
  const doStart = opts.start !== false;
  const doEnd = opts.end !== false;
  if (doStart && doEnd) return str.trim();
  if (doStart && !doEnd) return str.trimStart();
  if (!doStart && doEnd) return str.trimEnd();
  return str;
}
function Split(str, delim) { return Object.freeze(typeof str === 'string' ? str.split(delim) : []); }
function Replace(str, old, rep, opts) {
  if (typeof str !== 'string') return '';
  if (opts && opts.all) return str.split(old).join(rep);
  return str.replace(old, rep);
}
function Slice(str, opts) {
  if (typeof str !== 'string') return '';
  const start = (opts && opts.start !== undefined) ? opts.start : 0;
  const end = (opts && opts.end !== undefined) ? opts.end : str.length;
  return str.slice(start, end);
}
function CharAt(str, idx) { return typeof str === 'string' && idx >= 0 && idx < str.length ? str[idx] : ''; }
function Repeat(str, n) { return typeof str === 'string' ? str.repeat(Math.max(0, n)) : ''; }
function Reverse(val) {
  if (typeof val === 'string') return val.split('').reverse().join('');
  if (Array.isArray(val)) { const copy = [...val]; copy.reverse(); return Object.freeze(copy); }
  return val;
}
function Pad(str, len, opts) {
  if (typeof str !== 'string') return '';
  const side = (opts && opts.side) || 'start';
  const ch = (opts && opts.char) || ' ';
  if (side === 'start') return str.padStart(len, ch);
  if (side === 'end') return str.padEnd(len, ch);
  return str;
}

// ── Number Mold types ───────────────────────────────────
function ToFixed(num, digits) { return typeof num === 'number' ? num.toFixed(digits) : '0'; }
function Abs(num) { return typeof num === 'number' ? Math.abs(num) : 0; }
function Floor(num) { return typeof num === 'number' ? Math.floor(num) : 0; }
function Ceil(num) { return typeof num === 'number' ? Math.ceil(num) : 0; }
function Round(num) { return typeof num === 'number' ? Math.round(num) : 0; }
function Truncate(num) { return typeof num === 'number' ? Math.trunc(num) : 0; }
function Clamp(num, min, max) { return typeof num === 'number' ? Math.min(Math.max(num, min), max) : 0; }

// ── List Mold types (new operation molds) ───────────────
function Concat(list, other) { return Object.freeze([...(list || []), ...(other || [])]); }
function Append(list, val) { return Object.freeze([...(list || []), val]); }
function Prepend(list, val) { return Object.freeze([val, ...(list || [])]); }
function Join(list, sep) { return (list || []).join(sep); }
function Sum(list) { return (list || []).reduce((a, b) => a + b, 0); }
function Sort(list, opts) {
  const copy = [...(list || [])];
  if (opts && opts.by) {
    const fn = opts.by;
    copy.sort((a, b) => { const ka = fn(a), kb = fn(b); return ka < kb ? -1 : ka > kb ? 1 : 0; });
  } else {
    copy.sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
  }
  if (opts && opts.reverse) copy.reverse();
  return Object.freeze(copy);
}
function Unique(list, opts) {
  const result = [];
  const arr = list || [];
  if (opts && opts.by) {
    const fn = opts.by;
    const seen = [];
    for (const item of arr) {
      const key = fn(item);
      if (!seen.some(k => __taida_equals(k, key))) { seen.push(key); result.push(item); }
    }
  } else {
    for (const item of arr) {
      if (!result.some(x => __taida_equals(x, item))) result.push(item);
    }
  }
  return Object.freeze(result);
}
function Flatten(list) {
  const result = [];
  for (const item of (list || [])) {
    if (Array.isArray(item)) result.push(...item); else result.push(item);
  }
  return Object.freeze(result);
}
function Find(list, fn) {
  for (let i = 0; i < (list || []).length; i++) {
    if (fn(list[i]) === true) return Lax(list[i]);
  }
  return Lax(null);
}
function FindIndex(list, fn) {
  for (let i = 0; i < (list || []).length; i++) {
    if (fn(list[i]) === true) return i;
  }
  return -1;
}
function Count(list, fn) {
  let c = 0;
  for (let i = 0; i < (list || []).length; i++) {
    if (fn(list[i]) === true) c++;
  }
  return c;
}
function Zip(list, other) {
  const a = list || [], b = other || [];
  const len = Math.min(a.length, b.length);
  const result = [];
  for (let i = 0; i < len; i++) result.push(Object.freeze({ first: a[i], second: b[i] }));
  return Object.freeze(result);
}
function Enumerate(list) {
  return Object.freeze((list || []).map((value, index) => Object.freeze({ index, value })));
}

// ── Trampoline for tail recursion (self + mutual) ────────
class __TaidaTailCall {
  constructor(fn, args) { this.fn = fn; this.args = args; }
}
function __taida_trampoline(fn) {
  return function(...args) {
    let result = fn(...args);
    while (result instanceof __TaidaTailCall) {
      result = result.fn(...result.args);
    }
    return result;
  };
}

// ── stdout — Taida output function ───────────────────────
function __taida_stdout(...args) {
  for (const arg of args) {
    if (Array.isArray(arg)) {
      console.log('@[' + arg.map(x => __taida_format(x)).join(', ') + ']');
    } else if (arg && arg.__type === 'Async') {
      const status = arg.status;
      if (status === 'fulfilled') {
        console.log('Async[fulfilled: ' + String(arg.__value) + ']');
      } else if (status === 'rejected') {
        console.log('Async[rejected: ' + String(arg.__error) + ']');
      } else {
        console.log('Async[pending]');
      }
    } else if (arg && arg.__type === 'Result') {
      if (arg.isSuccess()) console.log('Result[' + String(arg.__value) + ']');
      else console.log('Result(throw)');
    } else if (arg && arg.__type === 'Lax') {
      const _lhv = typeof arg.hasValue === 'function' ? arg.hasValue() : arg.hasValue;
      if (_lhv) console.log('Lax(' + String(arg.__value) + ')');
      else console.log('Lax(default: ' + String(arg.__default) + ')');
    } else if (arg && typeof arg === 'object' && !Array.isArray(arg)) {
      // BuchiPack-like object
      const entries = Object.entries(arg).filter(([k]) => !k.startsWith('__'));
      const formatted = entries.map(([k, v]) => k + ' <= ' + __taida_format(v)).join(', ');
      console.log('@(' + formatted + ')');
    } else {
      console.log(typeof arg === 'boolean' ? (arg ? 'true' : 'false') : String(arg));
    }
  }
}

function __taida_format(v) {
  if (typeof v === 'string') return '"' + v + '"';
  if (Array.isArray(v)) return '@[' + v.map(x => __taida_format(x)).join(', ') + ']';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  if (v && typeof v === 'object' && !Array.isArray(v) && !v.__type) {
    const entries = Object.entries(v).filter(([k]) => !k.startsWith('__'));
    return '@(' + entries.map(([k, val]) => k + ' <= ' + __taida_format(val)).join(', ') + ')';
  }
  return String(v);
}

// ── stderr — Taida error output function (prelude) ──────
function __taida_stderr(...args) {
  for (const a of args) process.stderr.write(String(a) + '\n');
}

// ── stdin — Taida input function (prelude) ───────────────
// ESM: node:fs is loaded via top-level await dynamic import (no require())
const __taida_fs = await import('node:fs').catch(() => null);

function __taida_stdin(prompt) {
  if (typeof globalThis.process !== 'undefined' && __taida_fs) {
    if (prompt) process.stdout.write(prompt);
    try {
      const buf = Buffer.alloc(1024); let line = '';
      const fd = __taida_fs.openSync('/dev/stdin', 'r');
      let n; while ((n = __taida_fs.readSync(fd, buf, 0, 1)) > 0) {
        const ch = buf.toString('utf-8', 0, n); if (ch === '\n') break; line += ch;
      }
      __taida_fs.closeSync(fd);
      return line.replace(/\r$/, '');
    } catch(e) { return ''; }
  }
  return '';
}

// ── JSON prelude functions (output-direction only) ──────
function __taida_jsonEncode(v) {
  if (v instanceof __TaidaJSON) return JSON.stringify(__taidaSortKeys(v.__value));
  return JSON.stringify(__taidaSortKeys(v));
}

function __taida_jsonPretty(v) {
  if (v instanceof __TaidaJSON) return JSON.stringify(__taidaSortKeys(v.__value), null, 2);
  return JSON.stringify(__taidaSortKeys(v), null, 2);
}

// ── Safe unmold helper ───────────────────────────────────
// Unwrap mold types or return the value as-is.
// JSON is opaque — cannot be unmolded directly.
function __taida_unmold(v) {
  if (v instanceof __TaidaJSON) throw new __TaidaError('TypeError', 'Cannot unmold JSON directly. Use JSON[raw, Schema]() first.', {});
  if (v instanceof __TaidaAsync) {
    if (v.status === 'rejected') throw v.__error;
    return v.__value;
  }
  if (v && typeof v === 'object') {
    // Stream unmold: collect all items
    if (v.__type === 'Stream') return __taida_stream_collect(v);
    // Lax unmold
    if (v.__type === 'Lax') {
      const hv = typeof v.hasValue === 'function' ? v.hasValue() : v.hasValue;
      return hv ? v.__value : v.__default;
    }
    // Result unmold — evaluate predicate if present
    if (v.__type === 'Result') {
      if (typeof v.unmold === 'function') return v.unmold();
      // Fallback for raw objects without .unmold()
      if (v.throw !== null && v.throw !== undefined) {
        if (v.throw && typeof v.throw === 'object') {
          throw new __TaidaError(v.throw.type || 'ResultError', v.throw.message || String(v.throw), {});
        }
        throw v.throw;
      }
      return v.__value;
    }
    // Gorillax unmold: success → value, failure → gorilla (exit)
    if (v.__type === 'Gorillax') {
      const hv = typeof v.hasValue === 'function' ? v.hasValue() : v.hasValue;
      if (hv) return v.__value;
      if (typeof process !== 'undefined') process.exit(1);
      throw new Error('><');
    }
    // RelaxedGorillax unmold: success → value, failure → throw (catchable)
    if (v.__type === 'RelaxedGorillax') {
      const hv = typeof v.hasValue === 'function' ? v.hasValue() : v.hasValue;
      if (hv) return v.__value;
      throw new __TaidaError('RelaxedGorillaEscaped', 'Relaxed gorilla escaped', {});
    }
  }
  return v;
}

// ── Structural equality helper ───────────────────────────
// Taida uses structural equality (value-based) not reference identity.
function __taida_equals(a, b) {
  if (a === b) return true;
  if (a == null || b == null) return false;
  if (typeof a !== typeof b) return false;
  if (typeof a !== 'object') return a === b;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    return a.every((v, i) => __taida_equals(v, b[i]));
  }
  if (Array.isArray(a) || Array.isArray(b)) return false;
  // Filter out internal keys (__*) and method keys (function values)
  const ka = Object.keys(a).filter(k => !k.startsWith('__') && typeof a[k] !== 'function');
  const kb = Object.keys(b).filter(k => !k.startsWith('__') && typeof b[k] !== 'function');
  if (ka.length !== kb.length) return false;
  return ka.every(k => __taida_equals(a[k], b[k]));
}

// ── List/String/Number method extensions ─────────────────
// Only state-check methods, safe accessors, and monadic ops remain as prototype methods.
// All operation methods have been moved to standalone mold functions above.
if (!Array.prototype.__taida_patched) {
  Object.defineProperty(Array.prototype, '__taida_patched', { value: true, enumerable: false });
  Object.defineProperty(Array.prototype, 'length_', {
    value: function() { return this.length; }, enumerable: false
  });
  Object.defineProperty(Array.prototype, 'first', {
    value: function() {
      if (this.length === 0) return Lax(null);
      return Lax(this[0]);
    }, enumerable: false
  });
  Object.defineProperty(Array.prototype, 'last', {
    value: function() {
      if (this.length === 0) return Lax(null);
      return Lax(this[this.length - 1]);
    }, enumerable: false
  });
  Object.defineProperty(Array.prototype, 'contains', {
    value: function(v) { return this.some(x => __taida_equals(x, v)); }, enumerable: false
  });
  // Override Array.prototype.indexOf with structural equality for Taida
  Object.defineProperty(Array.prototype, 'indexOf', {
    value: function(v) {
      for (let i = 0; i < this.length; i++) {
        if (__taida_equals(this[i], v)) return i;
      }
      return -1;
    }, enumerable: false, configurable: true
  });
  // lastIndexOf(val) — last index of element using structural equality
  Object.defineProperty(Array.prototype, 'lastIndexOf', {
    value: function(v) {
      for (let i = this.length - 1; i >= 0; i--) {
        if (__taida_equals(this[i], v)) return i;
      }
      return -1;
    }, enumerable: false, configurable: true
  });
  // any(fn) — true if any element satisfies the predicate
  Object.defineProperty(Array.prototype, 'any', {
    value: function(fn) {
      for (let i = 0; i < this.length; i++) {
        if (fn(this[i]) === true) return true;
      }
      return false;
    }, enumerable: false
  });
  // all(fn) — true if all elements satisfy the predicate
  Object.defineProperty(Array.prototype, 'all', {
    value: function(fn) {
      for (let i = 0; i < this.length; i++) {
        if (fn(this[i]) !== true) return false;
      }
      return true;
    }, enumerable: false
  });
  // none(fn) — true if no element satisfies the predicate
  Object.defineProperty(Array.prototype, 'none', {
    value: function(fn) {
      for (let i = 0; i < this.length; i++) {
        if (fn(this[i]) === true) return false;
      }
      return true;
    }, enumerable: false
  });
  // get(index, customDefault?) — return Lax
  Object.defineProperty(Array.prototype, 'get', {
    value: function(idx, customDefault) {
      if (idx >= 0 && idx < this.length) {
        const val = this[idx];
        const lax = Lax(val);
        if (customDefault !== undefined) {
          return Object.freeze({
            __type: 'Lax',
            __value: val,
            __default: customDefault,
            hasValue: __taida_hasValue(true),
            isEmpty() { return false; },
            getOrDefault(def) { return val; },
            map(fn) { return Lax(fn(val)); },
            flatMap(fn) { const r = fn(val); return r && r.__type === 'Lax' ? r : Lax(r); },
            unmold() { return val; },
            toString() { return 'Lax(' + String(val) + ')'; },
          });
        }
        return lax;
      }
      const def = customDefault !== undefined ? customDefault : (this.length > 0 ? __taida_lax_default(this[0]) : 0);
      return Object.freeze({
        __type: 'Lax',
        __value: def,
        __default: def,
        hasValue: __taida_hasValue(false),
        isEmpty() { return true; },
        getOrDefault(d) { return d; },
        map(fn) { return this; },
        flatMap(fn) { return this; },
        unmold() { return def; },
        toString() { return 'Lax(default: ' + String(def) + ')'; },
      });
    }, enumerable: false
  });
  // isEmpty() — check if list is empty
  Object.defineProperty(Array.prototype, 'isEmpty', {
    value: function() { return this.length === 0; }, enumerable: false
  });
  // max() — return Lax (empty list returns Lax with hasValue=false)
  Object.defineProperty(Array.prototype, 'max', {
    value: function() {
      if (this.length === 0) return Lax(null);
      return Lax(this.reduce((a, b) => a > b ? a : b));
    }, enumerable: false
  });
  // min() — return Lax (empty list returns Lax with hasValue=false)
  Object.defineProperty(Array.prototype, 'min', {
    value: function() {
      if (this.length === 0) return Lax(null);
      return Lax(this.reduce((a, b) => a < b ? a : b));
    }, enumerable: false
  });
  Object.defineProperty(Array.prototype, 'toString', {
    value: function() { return '@[' + this.map(x => typeof x === 'string' ? '"' + x + '"' : String(x)).join(', ') + ']'; }, enumerable: false
  });
}

// Taida calls .toString() on numbers, booleans, etc.
// Number/Boolean already have toString, but ensure Taida-compatible formatting.
if (!Number.prototype.__taida_patched) {
  Object.defineProperty(Number.prototype, '__taida_patched', { value: true, enumerable: false });
  Object.defineProperty(Number.prototype, 'isNaN', {
    value: function() { return Number.isNaN(this.valueOf()); }, enumerable: false
  });
  Object.defineProperty(Number.prototype, 'isInfinite', {
    value: function() { const v = this.valueOf(); return !Number.isFinite(v) && !Number.isNaN(v); }, enumerable: false
  });
  Object.defineProperty(Number.prototype, 'isFinite', {
    value: function() { return Number.isFinite(this.valueOf()); }, enumerable: false
  });
  Object.defineProperty(Number.prototype, 'isPositive', {
    value: function() { return this.valueOf() > 0; }, enumerable: false
  });
  Object.defineProperty(Number.prototype, 'isNegative', {
    value: function() { return this.valueOf() < 0; }, enumerable: false
  });
  Object.defineProperty(Number.prototype, 'isZero', {
    value: function() { return this.valueOf() === 0; }, enumerable: false
  });
}
// String methods: only state-check and safe access methods remain.
// Operation methods (reverse, trim, etc.) are now standalone mold functions.
if (!String.prototype.__taida_str_patched) {
  // Save native startsWith/endsWith BEFORE overwriting
  const __native_startsWith = String.prototype.startsWith;
  const __native_endsWith = String.prototype.endsWith;
  Object.defineProperty(String.prototype, '__taida_str_patched', { value: true, enumerable: false });
  Object.defineProperty(String.prototype, 'length_', {
    value: function() { return this.length; }, enumerable: false
  });
  Object.defineProperty(String.prototype, 'contains', {
    value: function(v) { return this.includes(v); }, enumerable: false
  });
  // startsWith/endsWith — delegate to saved native references
  Object.defineProperty(String.prototype, 'startsWith', {
    value: function(v) { return __native_startsWith.call(this, v); }, enumerable: false, configurable: true
  });
  Object.defineProperty(String.prototype, 'endsWith', {
    value: function(v) { return __native_endsWith.call(this, v); }, enumerable: false, configurable: true
  });
  // indexOf — already native, structural for string is identity
  // lastIndexOf — already native
  // get(index) — return Lax for string character access
  Object.defineProperty(String.prototype, 'get', {
    value: function(idx) {
      if (idx >= 0 && idx < this.length) return Lax(this[idx]);
      return Object.freeze({
        __type: 'Lax', __value: '', __default: '', hasValue: __taida_hasValue(false),
        isEmpty() { return true; }, getOrDefault(d) { return d; },
        map(fn) { return this; }, flatMap(fn) { return this; },
        unmold() { return ''; }, toString() { return 'Lax(default: "")'; },
      });
    }, enumerable: false
  });
}

// ── Helper: sort object keys for deterministic JSON output ──
function __taidaSortKeys(obj) {
  if (Array.isArray(obj)) return obj.map(__taidaSortKeys);
  if (obj && typeof obj === 'object' && !(obj instanceof __TaidaJSON)) {
    const sorted = {};
    for (const k of Object.keys(obj).sort()) sorted[k] = __taidaSortKeys(obj[k]);
    return sorted;
  }
  return obj;
}

// ── __taida_std removed (std dissolution) ─────────────────
// stdout/stderr/stdin: __taida_stdout/__taida_stderr/__taida_stdin
// JSON: __taida_jsonEncode/__taida_jsonPretty (output-direction only)
// jsonParse/jsonDecode/jsonFrom: ABOLISHED (Molten Iron)

// ── Prelude utility functions ─────────────────────────────

// ── HashMap — immutable key-value collection ──
// Internal: entries = [{key, value}, ...] (frozen BuchiPack pairs)
// All mutating methods return a new HashMap.
function __taida_createHashMap(entries) {
  const _entries = Object.freeze(entries);
  const hm = {
    __type: 'HashMap',
    __entries: _entries,
    get(key) {
      for (const e of _entries) {
        if (__taida_equals(e.key, key)) return Lax(e.value);
      }
      return Lax(undefined);
    },
    set(key, value) {
      const newEntries = [];
      let found = false;
      for (const e of _entries) {
        if (__taida_equals(e.key, key)) {
          newEntries.push(Object.freeze({ key, value }));
          found = true;
        } else {
          newEntries.push(e);
        }
      }
      if (!found) newEntries.push(Object.freeze({ key, value }));
      return __taida_createHashMap(newEntries);
    },
    remove(key) {
      return __taida_createHashMap(_entries.filter(e => !__taida_equals(e.key, key)));
    },
    has(key) {
      return _entries.some(e => __taida_equals(e.key, key));
    },
    keys() {
      return Object.freeze(_entries.map(e => e.key));
    },
    values() {
      return Object.freeze(_entries.map(e => e.value));
    },
    entries() {
      return Object.freeze(_entries.map(e => Object.freeze({ first: e.key, second: e.value })));
    },
    size() {
      return _entries.length;
    },
    isEmpty() {
      return _entries.length === 0;
    },
    merge(other) {
      if (!other || other.__type !== 'HashMap') return hm;
      const merged = [..._entries];
      for (const oe of other.__entries) {
        const idx = merged.findIndex(e => __taida_equals(e.key, oe.key));
        if (idx >= 0) {
          merged[idx] = oe;
        } else {
          merged.push(oe);
        }
      }
      return __taida_createHashMap(merged);
    },
    toString() {
      const pairs = _entries.map(e => {
        const k = typeof e.key === 'string' ? '"' + e.key + '"' : String(e.key);
        const v = typeof e.value === 'string' ? '"' + e.value + '"' : String(e.value);
        return k + ': ' + v;
      });
      return 'HashMap({' + pairs.join(', ') + '})';
    },
  };
  return Object.freeze(hm);
}

function hashMap(entries) {
  if (Array.isArray(entries)) {
    const parsed = [];
    for (const entry of entries) {
      if (entry && typeof entry === 'object') {
        const key = entry.key !== undefined ? entry.key : (entry.first !== undefined ? entry.first : undefined);
        const value = entry.value !== undefined ? entry.value : (entry.second !== undefined ? entry.second : undefined);
        if (key !== undefined && value !== undefined) {
          parsed.push(Object.freeze({ key, value }));
        }
      }
    }
    return __taida_createHashMap(parsed);
  }
  // BuchiPack (plain object): each field becomes a key-value entry
  // hashMap(@(a <= 1, b <= 2)) -> [{key: "a", value: 1}, {key: "b", value: 2}]
  if (entries && typeof entries === 'object' && !Array.isArray(entries)) {
    const parsed = [];
    for (const [k, v] of Object.entries(entries)) {
      if (k !== '__type') {
        parsed.push(Object.freeze({ key: k, value: v }));
      }
    }
    return __taida_createHashMap(parsed);
  }
  return __taida_createHashMap([]);
}

// ── Set — immutable unique collection ──
// Internal: items = [...unique values] (frozen array)
// All mutating methods return a new Set.
function __taida_createSet(items) {
  const _items = Object.freeze(items);
  const s = {
    __type: 'Set',
    __items: _items,
    add(item) {
      if (_items.some(x => __taida_equals(x, item))) return s;
      return __taida_createSet([..._items, item]);
    },
    remove(item) {
      return __taida_createSet(_items.filter(x => !__taida_equals(x, item)));
    },
    has(item) {
      return _items.some(x => __taida_equals(x, item));
    },
    union(other) {
      if (!other || other.__type !== 'Set') return s;
      const result = [..._items];
      for (const item of other.__items) {
        if (!result.some(x => __taida_equals(x, item))) result.push(item);
      }
      return __taida_createSet(result);
    },
    intersect(other) {
      if (!other || other.__type !== 'Set') return __taida_createSet([]);
      return __taida_createSet(_items.filter(item => other.__items.some(x => __taida_equals(x, item))));
    },
    diff(other) {
      if (!other || other.__type !== 'Set') return s;
      return __taida_createSet(_items.filter(item => !other.__items.some(x => __taida_equals(x, item))));
    },
    toList() {
      return _items;
    },
    size() {
      return _items.length;
    },
    isEmpty() {
      return _items.length === 0;
    },
    toString() {
      const strs = _items.map(i => typeof i === 'string' ? '"' + i + '"' : String(i));
      return 'Set({' + strs.join(', ') + '})';
    },
  };
  return Object.freeze(s);
}

function setOf(items) {
  if (!Array.isArray(items)) return __taida_createSet([]);
  const unique = [];
  for (const item of items) {
    if (!unique.some(x => __taida_equals(x, item))) unique.push(item);
  }
  return __taida_createSet(unique);
}

function range(start, end) {
  if (end === undefined) { end = start; start = 0; }
  const result = [];
  for (let i = start; i < end; i++) result.push(i);
  return Object.freeze(result);
}

function enumerate(list) {
  if (!Array.isArray(list)) return Object.freeze([]);
  return Object.freeze(list.map((value, index) => Object.freeze({ index, value })));
}

function zip(a, b) {
  if (!Array.isArray(a) || !Array.isArray(b)) return Object.freeze([]);
  const len = Math.min(a.length, b.length);
  const result = [];
  for (let i = 0; i < len; i++) {
    result.push(Object.freeze({ first: a[i], second: b[i] }));
  }
  return Object.freeze(result);
}

function __taida_assert(cond, msg) {
  if (!cond) throw new __TaidaError('AssertionError', msg || 'Assertion failed', {});
}

function __taida_typeof(x) {
  if (x === null || x === undefined) return 'Unit';
  if (typeof x === 'number') return Number.isInteger(x) ? 'Int' : 'Float';
  if (typeof x === 'boolean') return 'Bool';
  if (typeof x === 'string') return 'Str';
  if (Array.isArray(x)) return 'List';
  if (x instanceof __TaidaJSON) return 'JSON';
  if (x instanceof __TaidaAsync) return 'Async';
  if (x && x.__type) return x.__type;
  if (typeof x === 'object') return 'BuchiPack';
  return 'Unknown';
}

// ── JS interop helpers (Molten operations) ──
function __taida_js_spread(target, source) {
  if (Array.isArray(target)) {
    return [...target, ...(Array.isArray(source) ? source : [source])];
  }
  return {...target, ...source};
}

// ── taida-lang/os — Core-bundled OS package (13 APIs) ──
// Uses Node.js fs, child_process, process modules.

// ESM: reuse __taida_fs for fs operations, load child_process via dynamic import
const __os_fs = __taida_fs || null;
const __os_cp = await import('node:child_process').catch(() => null);

const __OS_MAX_READ_SIZE = 64 * 1024 * 1024; // 64 MB

// Helper: create os Result success value
function __taida_os_result_ok(inner) {
  return __taida_result_create(inner, null, null);
}

// Helper: create os Result failure from error
function __taida_os_result_fail(err) {
  const code = err.errno || -1;
  const message = err.message || String(err);
  const inner = Object.freeze({ ok: false, code: code, message: message });
  const throwVal = { __type: 'IoError', type: 'IoError', message: message, fields: { code: code } };
  return __taida_result_create(inner, throwVal, null);
}

// Helper: standard success inner @(ok=true, code=0, message="")
function __taida_os_ok_inner() {
  return Object.freeze({ ok: true, code: 0, message: '' });
}

// Helper: process result inner @(stdout, stderr, code)
function __taida_os_process_inner(stdout, stderr, code) {
  return Object.freeze({ stdout: stdout, stderr: stderr, code: code });
}

// ── Input molds (Read, ListDir, Stat, Exists, EnvVar) ──

function __taida_os_read(path) {
  if (!__os_fs) return Lax(null, '');
  try {
    const stat = __os_fs.statSync(path);
    if (stat.size > __OS_MAX_READ_SIZE) return Lax(null, '');
    const content = __os_fs.readFileSync(path, 'utf-8');
    return Lax(content);
  } catch (e) {
    return Lax(null, '');
  }
}

function __taida_os_listdir(path) {
  if (!__os_fs) return Lax(null, Object.freeze([]));
  try {
    const entries = __os_fs.readdirSync(path).sort();
    return Lax(Object.freeze(entries));
  } catch (e) {
    return Lax(null, Object.freeze([]));
  }
}

function __taida_os_stat(path) {
  const defaultStat = Object.freeze({ size: 0, modified: '', isDir: false });
  if (!__os_fs) return Lax(null, defaultStat);
  try {
    const stat = __os_fs.statSync(path);
    const modified = new Date(stat.mtimeMs).toISOString().replace(/\.\d{3}Z$/, 'Z');
    const result = Object.freeze({
      size: stat.size,
      modified: modified,
      isDir: stat.isDirectory()
    });
    return Lax(result);
  } catch (e) {
    return Lax(null, defaultStat);
  }
}

function __taida_os_exists(path) {
  if (!__os_fs) return false;
  return __os_fs.existsSync(path);
}

function __taida_os_envvar(name) {
  const val = typeof process !== 'undefined' && process.env ? process.env[name] : undefined;
  if (val !== undefined) return Lax(val);
  return Lax(null, '');
}

// ── Side-effect functions (writeFile, appendFile, remove, createDir, rename) ──

function __taida_os_writeFile(path, content) {
  try {
    __os_fs.writeFileSync(path, content);
    return __taida_os_result_ok(__taida_os_ok_inner());
  } catch (e) {
    return __taida_os_result_fail(e);
  }
}

function __taida_os_appendFile(path, content) {
  try {
    __os_fs.appendFileSync(path, content);
    return __taida_os_result_ok(__taida_os_ok_inner());
  } catch (e) {
    return __taida_os_result_fail(e);
  }
}

function __taida_os_remove(path) {
  try {
    __os_fs.rmSync(path, { recursive: true, force: true });
    return __taida_os_result_ok(__taida_os_ok_inner());
  } catch (e) {
    return __taida_os_result_fail(e);
  }
}

function __taida_os_createDir(path) {
  try {
    __os_fs.mkdirSync(path, { recursive: true });
    return __taida_os_result_ok(__taida_os_ok_inner());
  } catch (e) {
    return __taida_os_result_fail(e);
  }
}

function __taida_os_rename(from, to) {
  try {
    __os_fs.renameSync(from, to);
    return __taida_os_result_ok(__taida_os_ok_inner());
  } catch (e) {
    return __taida_os_result_fail(e);
  }
}

// ── Process functions (run, execShell) ──

function __taida_os_run(program, args) {
  if (!__os_cp) {
    return __taida_os_result_fail(new Error('child_process not available'));
  }
  try {
    const result = __os_cp.execFileSync(program, args || [], { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'pipe'] });
    const inner = __taida_os_process_inner(result, '', 0);
    return __taida_os_result_ok(inner);
  } catch (e) {
    if (e.status !== undefined) {
      // Process exited with non-zero code
      const stdout = e.stdout ? String(e.stdout) : '';
      const stderr = e.stderr ? String(e.stderr) : '';
      const code = e.status !== null ? e.status : -1;
      const inner = __taida_os_process_inner(stdout, stderr, code);
      const throwVal = { __type: 'ProcessError', type: 'ProcessError', message: "Process '" + program + "' exited with code " + code, fields: { code: code } };
      return __taida_result_create(inner, throwVal, null);
    }
    return __taida_os_result_fail(e);
  }
}

function __taida_os_execShell(command) {
  if (!__os_cp) {
    return __taida_os_result_fail(new Error('child_process not available'));
  }
  try {
    const result = __os_cp.execSync(command, { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'pipe'] });
    const inner = __taida_os_process_inner(result, '', 0);
    return __taida_os_result_ok(inner);
  } catch (e) {
    if (e.status !== undefined) {
      const stdout = e.stdout ? String(e.stdout) : '';
      const stderr = e.stderr ? String(e.stderr) : '';
      const code = e.status !== null ? e.status : -1;
      const inner = __taida_os_process_inner(stdout, stderr, code);
      const throwVal = { __type: 'ProcessError', type: 'ProcessError', message: 'Shell command exited with code ' + code + ': ' + command, fields: { code: code } };
      return __taida_result_create(inner, throwVal, null);
    }
    return __taida_os_result_fail(e);
  }
}

// ── Query function (allEnv) ──

function __taida_os_allEnv() {
  const entries = [];
  if (typeof process !== 'undefined' && process.env) {
    for (const [key, value] of Object.entries(process.env)) {
      entries.push(Object.freeze({ key: key, value: value || '' }));
    }
  }
  return Object.freeze({ __entries: Object.freeze(entries), __type: 'HashMap' });
}

// ── Phase 2: Async APIs ───────────────────────────────────

// Helper: build HTTP response Lax
function __taida_os_http_response(status, body, headers) {
  const headerObj = {};
  if (headers) {
    for (const [k, v] of headers) {
      headerObj[k.toLowerCase()] = v;
    }
  }
  return Lax(Object.freeze({ status: status, body: body, headers: Object.freeze(headerObj) }));
}

function __taida_os_http_failure() {
  return Lax(null, Object.freeze({ status: 0, body: '', headers: Object.freeze({}) }));
}

// ReadAsync[path]() -> Promise<Lax[Str]>
async function __taida_os_readAsync(path) {
  if (!__os_fs) return Lax(null, '');
  try {
    const fsp = __os_fs.promises || await import('node:fs/promises').then(m => m.default || m).catch(() => null);
    if (!fsp) return Lax(null, '');
    const stat = await fsp.stat(path);
    if (stat.size > 64 * 1024 * 1024) return Lax(null, '');
    const content = await fsp.readFile(path, 'utf-8');
    return Lax(content);
  } catch (e) {
    return Lax(null, '');
  }
}

// HttpGet[url]() -> Promise<Lax[@(status, body, headers)]>
async function __taida_os_httpGet(url) {
  try {
    const resp = await fetch(url);
    const body = await resp.text();
    const headers = [];
    resp.headers.forEach((v, k) => headers.push([k, v]));
    return __taida_os_http_response(resp.status, body, headers);
  } catch (e) {
    return __taida_os_http_failure();
  }
}

// HttpPost[url, body]() -> Promise<Lax[@(status, body, headers)]>
async function __taida_os_httpPost(url, body) {
  try {
    const resp = await fetch(url, { method: 'POST', body: body || '' });
    const respBody = await resp.text();
    const headers = [];
    resp.headers.forEach((v, k) => headers.push([k, v]));
    return __taida_os_http_response(resp.status, respBody, headers);
  } catch (e) {
    return __taida_os_http_failure();
  }
}

// HttpRequest[method, url](headers, body) -> Promise<Lax[@(status, body, headers)]>
async function __taida_os_httpRequest(method, url, body) {
  try {
    const opts = { method: method || 'GET' };
    if (body) opts.body = body;
    const resp = await fetch(url, opts);
    const respBody = await resp.text();
    const headers = [];
    resp.headers.forEach((v, k) => headers.push([k, v]));
    return __taida_os_http_response(resp.status, respBody, headers);
  } catch (e) {
    return __taida_os_http_failure();
  }
}

// TCP: load net module
const __os_net = await import('node:net').catch(() => null);

// tcpConnect(host, port) -> Promise<Result[@(socket, host, port), _]>
async function __taida_os_tcpConnect(host, port) {
  if (!__os_net) return __taida_os_result_fail(new Error('net module not available'));
  return new Promise((resolve) => {
    const socket = new (__os_net.Socket || __os_net.default.Socket)();
    socket.connect(port, host, () => {
      const inner = Object.freeze({ socket: socket, host: host, port: port });
      resolve(__taida_os_result_ok(inner));
    });
    socket.on('error', (err) => {
      resolve(__taida_os_result_fail(err));
    });
  });
}

// tcpListen(port) -> Promise<Result[@(listener, port), _]>
async function __taida_os_tcpListen(port) {
  if (!__os_net) return __taida_os_result_fail(new Error('net module not available'));
  return new Promise((resolve) => {
    const server = (__os_net.createServer || __os_net.default.createServer)();
    server.listen(port, '0.0.0.0', () => {
      const inner = Object.freeze({ listener: server, port: port });
      resolve(__taida_os_result_ok(inner));
    });
    server.on('error', (err) => {
      resolve(__taida_os_result_fail(err));
    });
  });
}

// socketSend(socket, data) -> Promise<Result[@(ok, bytesSent), _]>
async function __taida_os_socketSend(socketOrPack, data) {
  const socket = (socketOrPack && socketOrPack.socket) ? socketOrPack.socket : socketOrPack;
  if (!socket || !socket.write) return __taida_os_result_fail(new Error('Invalid socket'));
  return new Promise((resolve) => {
    const ok = socket.write(data, 'utf-8', (err) => {
      if (err) {
        resolve(__taida_os_result_fail(err));
      } else {
        const inner = Object.freeze({ ok: true, bytesSent: Buffer.byteLength(data, 'utf-8') });
        resolve(__taida_os_result_ok(inner));
      }
    });
  });
}

// socketRecv(socket) -> Promise<Lax[Str]>
async function __taida_os_socketRecv(socketOrPack) {
  const socket = (socketOrPack && socketOrPack.socket) ? socketOrPack.socket : socketOrPack;
  if (!socket || !socket.once) return Lax(null, '');
  return new Promise((resolve) => {
    socket.once('data', (chunk) => {
      resolve(Lax(chunk.toString('utf-8')));
    });
    socket.once('end', () => {
      resolve(Lax(null, ''));
    });
    socket.once('error', () => {
      resolve(Lax(null, ''));
    });
  });
}

function isEven(x) {
  const r = __taida_unmold(Mod_mold(x, 2));
  return __taida_equals(r, 0);
}

function double(x) {
  return (x * 2);
}

function lessThan5(x) {
  return (x < 5);
}

function timesTen(x) {
  return (x * 10);
}

const s1 = Stream_mold(42);
const r1 = await __taida_unmold(s1);
__taida_stdout(Join(r1, " "));
const nums = Object.freeze([1, 2, 3, 4, 5]);
const s2 = StreamFrom(nums);
const r2 = await __taida_unmold(s2);
__taida_stdout(Join(r2, " "));
const s3 = StreamFrom(Object.freeze([10, 20, 30]));
const mapped = Map(s3, double);
const r3 = await __taida_unmold(mapped);
__taida_stdout(Join(r3, " "));
const s4 = StreamFrom(Object.freeze([1, 2, 3, 4, 5, 6]));
const filtered = Filter(s4, isEven);
const r4 = await __taida_unmold(filtered);
__taida_stdout(Join(r4, " "));
const s5 = StreamFrom(Object.freeze([10, 20, 30, 40, 50]));
const taken = Take(s5, 3);
const r5 = await __taida_unmold(taken);
__taida_stdout(Join(r5, " "));
const s6 = StreamFrom(Object.freeze([1, 2, 3, 10, 20]));
const tw = TakeWhile(s6, lessThan5);
const r6 = await __taida_unmold(tw);
__taida_stdout(Join(r6, " "));
const s7 = StreamFrom(Object.freeze([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]));
const step1 = Filter(s7, isEven);
const step2 = Map(step1, timesTen);
const step3 = Take(step2, 3);
const results = await __taida_unmold(step3);
__taida_stdout(Join(results, " "));
const s8 = StreamFrom(Object.freeze([1, 2, 3]));
__taida_stdout(s8.toString());
const len8 = s8.length_();
__taida_stdout(len8.toString());
const empty8 = s8.isEmpty();
__taida_stdout(empty8.toString());
const s9 = StreamFrom(Object.freeze([]));
const empty9 = s9.isEmpty();
__taida_stdout(empty9.toString());
